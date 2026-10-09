//! PrismOS exception + hardware interrupt handlers (Phase 2, steps 1 and 3).
//!
//! ROLE:
//! Catches CPU-raised exceptions (breakpoint, page fault, general
//! protection, ...) AND the first hardware IRQ (timer) and reports them on
//! serial instead of triple-faulting into a silent reboot. The IDT is the
//! single front door for PIC remap, timer, and (next) keyboard.
//!
//! SCOPE:
//! CPU exceptions plus PIC-remapped timer IRQ0. The PICs are remapped to
//! vectors 32-47 so hardware IRQs can no longer masquerade as CPU
//! exceptions 0-15 (the infamous double-fault-on-every-timer-tick trap).
//! Keyboard IRQ1 and friends arrive in the next step; their IDT slots are
//! still empty, so an unexpected IRQ halts loudly instead of corrupting.
//!
//! REENTRANCY RULE (read before touching a handler):
//! Interrupts are ENABLED after `init()`, so the timer CAN preempt the main
//! loop at any point — including mid-`println!` while the serial lock is
//! held. Therefore the timer handler NEVER locks, NEVER prints: it bumps a
//! lock-free atomic counter and sends the EOI. All formatting happens in the
//! main loop (shell `timer` command). Fatal handlers print + halt, which is
//! safe because nothing runs after them.
//!
//! KNOWN LIMITATION (documented, not hidden):
//! Page-fault and GP handlers still halt instead of recovering (no pager
//! yet — next RFCs). The double fault itself, however, now runs on a
//! dedicated IST stack (see `gdt`), so even a smashed kernel stack still
//! produces a serial dump instead of a silent triple-fault.
//!
//! WIRING:
//! `init()` builds the table once (`spin::Once`) and loads it with `lidt`.
//! Call `gdt::init()` FIRST: the double-fault entry below borrows the IST
//! index owned by `gdt`, and the CPU needs TR loaded before any #DF fires.
//! `kernel_main` then runs an `int3` self-test proving a handler runs AND
//! returns to the interrupted code.

use super::gdt::DOUBLE_FAULT_IST_INDEX;
use core::sync::atomic::{AtomicU64, Ordering};
use pic8259::ChainedPics;
use spin::{Mutex, Once};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

/// Hardware IRQ base vectors after remap. 32-39 = primary PIC (timer at 32),
/// 40-47 = secondary PIC. Chosen to sit ABOVE the 32 CPU exception vectors
/// so an IRQ can never be mistaken for an exception (or vice versa).
pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = 40;

/// Hardware IRQ numbers we currently serve: timer (IRQ0) and keyboard
/// (IRQ1). Remaining vectors stay empty on purpose: an unexpected IRQ halts
/// loudly via #NP instead of running a half-written handler.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
}

impl InterruptIndex {
    /// Numeric IDT vector, e.g. for EOI notifications. `pub(crate)` because
    /// device drivers (`keyboard`) need it for their EOI; nothing outside
    /// the kernel ever should.
    pub(crate) fn as_u8(self) -> u8 {
        self as u8
    }

    /// Same vector as `usize` for IDT indexing (`Index<usize>` by API).
    fn as_usize(self) -> usize {
        self as u8 as usize
    }
}

/// The chained primary + secondary 8259 PICs.
///
/// SAFETY of construction: offsets 32/40 are each used once and never
/// overlap CPU vectors 0-31, so no double-registration can occur.
pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// Timer IRQs serviced since boot. Atomic = lock-free: the ONLY shared state
/// the timer handler touches (see REENTRANCY RULE above).
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

/// How many timer interrupts have fired. Called from the shell, never from
/// a handler (a handler reading it would be pointless but harmless).
pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}

/// The single system IDT. `Once` guarantees exactly one build + load; a
/// second `init()` call is a harmless no-op, never a double-`lidt`.
static IDT: Once<InterruptDescriptorTable> = Once::new();

/// Build and load the IDT. Must be called once from `kernel_main` before
/// any exception can occur (in practice: right after console init).
///
/// CONTRACT: call `gdt::init()` FIRST. The double-fault entry below arms an
/// IST stack, and an IST index is only meaningful once TR points at our TSS.
/// Calling this without a loaded TSS would make a future #DF read garbage.
pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        // Recoverable: prints the frame, then returns to the caller.
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        // Fatal: print diagnostics, then halt (never return).
        // The double fault runs on the dedicated IST stack from `gdt`, so it
        // survives even a destroyed kernel stack (e.g. stack overflow).
        unsafe {
            // SAFETY: index comes from `gdt::DOUBLE_FAULT_IST_INDEX` (0 < 7,
            // the table has exactly 7 slots), and the caller contract above
            // guarantees `gdt::init()` already loaded the TSS into TR. A
            // wrong index here would corrupt #DF delivery — hence `unsafe`.
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(DOUBLE_FAULT_IST_INDEX);
        }
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        // A missing IDT gate raises #NP (not #GP): without this entry an
        // unexpected IRQ would triple-fault silently instead of dumping.
        idt.segment_not_present
            .set_handler_fn(segment_not_present_handler);
        // First hardware IRQs: timer + keyboard. More vectors arrive with
        // future devices; each gets its own minimal handler + EOI.
        idt[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_handler);
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(crate::keyboard::irq_handler);
        idt
    });
    // `lidt`: loads our table address into the CPU's IDTR register.
    // Safe: the table lives in a `static` (never moves, never drops).
    idt.load();

    // Remap the PICs BEFORE `sti`: from power-on they deliver IRQs on
    // vectors 0-15 (colliding with CPU exceptions), which is unusable.
    unsafe {
        // SAFETY: single-core boot, interrupts still disabled, offsets
        // verified unique at `PICS` construction. Re-running `init()` only
        // re-sends the same init sequence (idempotent by PIC design).
        PICS.lock().initialize();
    }

    // Program the PIT BEFORE unmasking: OVMF leaves it idle, so without this
    // the timer IRQ would never fire (observed: counter stuck at 0).
    super::pit::init();

    // Unmask IRQ0 (timer) and IRQ1 (keyboard) ONLY. Mask bits are
    // active-high: 0xFC enables bits 0-1 and masks 2-7, 0xFF masks the whole
    // secondary PIC. Both enabled IRQs have installed IDT gates; anything
    // else stays masked until its driver (and gate) lands.
    unsafe {
        // SAFETY: same single-core pre-`sti` context as above; the mask
        // values only enable IRQs whose handlers are already installed.
        PICS.lock().write_masks(0xFC, 0xFF);
    }

    // From here on, hardware IRQs can preempt the main loop: every handler
    // must obey the REENTRANCY RULE (no locks, no printing in `timer_handler`).
    x86_64::instructions::interrupts::enable();
    crate::println!("[idt] PIC remapped (32-47), PIT at 100 Hz, timer + keyboard armed");
}

/// Breakpoint (#BP, vector 3). Raised by `int3`; recoverable.
///
/// Prints the interrupted instruction pointer so a reviewer can verify the
/// faulting address matches the `int3` site, then returns normally.
extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    crate::println!("[idt] breakpoint hit — returning to kernel");
    crate::println!("[idt] interrupted frame: {:#?}", stack_frame);
}

/// Timer IRQ0 (vector 32). Runs at 100 Hz (see `pit::init`).
///
/// DELIBERATELY MINIMAL: bump the atomic counter, send the EOI, return. No
/// locks, no printing — see REENTRANCY RULE. Missing the EOI would silence
/// the PIC forever (no further IRQs); a wrong vector in the EOI would
/// misroute the next interrupt. Both are covered by the single constant.
extern "x86-interrupt" fn timer_handler(_stack_frame: InterruptStackFrame) {
    TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        // SAFETY: notifies exactly the vector just serviced (`Timer`), after
        // its work is done. The PIC is initialized (see `init`) and this is
        // the only EOI site, so no double-EOI can occur.
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }
}

/// Segment-not-present (#NP, vector 11). Diverging.
///
/// Typical cause from here on: an IRQ whose IDT gate was never installed
/// (a future device before its driver lands, or a spurious PIC IRQ). The
/// error code identifies the missing selector/gate — the first clue when
/// bringing up a new device.
extern "x86-interrupt" fn segment_not_present_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!();
    crate::println!("[idt] FATAL: segment not present (error code {error_code})");
    crate::println!("[idt] frame: {:#?}", stack_frame);
    crate::println!("[idt] system halted. Restart QEMU to reboot.");
    loop {
        x86_64::instructions::hlt();
    }
}

/// Double fault (#DF, vector 8). Diverging: never returns.
///
/// Raised when the CPU fails WHILE delivering a previous exception (e.g.
/// IDT entry missing, kernel stack unmapped, stack overflow). Runs on the
/// dedicated IST stack (see `gdt::init`), so it can still print even when
/// the normal kernel stack is gone. Prints the error code and the frame,
/// then halts.
extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    crate::println!();
    crate::println!("[idt] FATAL: double fault (error code {error_code})");
    crate::println!("[idt] frame: {:#?}", stack_frame);
    crate::println!("[idt] system halted. Restart QEMU to reboot.");
    loop {
        x86_64::instructions::hlt();
    }
}

/// Page fault (#PF, vector 14). Diverging in Phase 1 (no pager yet).
///
/// Prints the faulting virtual address (CR2) plus the error-code flags
/// (present/write/user/instruction-fetch). A future pager RFC will turn
/// some of these into recoverable faults; until then, halt loudly.
extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    crate::println!();
    crate::println!("[idt] FATAL: page fault while accessing {:#?}", Cr2::read());
    crate::println!("[idt] error flags: {error_code:?}");
    crate::println!("[idt] frame: {:#?}", stack_frame);
    crate::println!("[idt] system halted. Restart QEMU to reboot.");
    loop {
        x86_64::instructions::hlt();
    }
}

/// General protection fault (#GP, vector 13). Diverging.
///
/// Typical causes: segment violation, unhandled IDT vector, privileged
/// instruction in user mode (later phases). The error code holds the
/// offending segment selector, or 0 when not selector-related.
extern "x86-interrupt" fn general_protection_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    crate::println!();
    crate::println!("[idt] FATAL: general protection fault (error code {error_code})");
    crate::println!("[idt] frame: {:#?}", stack_frame);
    crate::println!("[idt] system halted. Restart QEMU to reboot.");
    loop {
        x86_64::instructions::hlt();
    }
}

/// Invalid opcode (#UD, vector 6). Diverging.
///
/// Raised when the CPU decodes an unknown instruction — usually a corrupted
/// return address or a data-executed-as-code bug. Halts with the frame.
extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    crate::println!();
    crate::println!("[idt] FATAL: invalid opcode");
    crate::println!("[idt] frame: {:#?}", stack_frame);
    crate::println!("[idt] system halted. Restart QEMU to reboot.");
    loop {
        x86_64::instructions::hlt();
    }
}
