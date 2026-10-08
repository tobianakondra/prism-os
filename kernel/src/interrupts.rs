//! PrismOS CPU exception handlers (Phase 2, step 1: IDT).
//!
//! ROLE:
//! Catches CPU-raised exceptions (breakpoint, page fault, general
//! protection, ...) and reports them on serial instead of triple-faulting
//! into a silent reboot. This is the foundation every later Phase-2 piece
//! needs: PIC remap, timer, and keyboard all deliver through this same IDT.
//!
//! SCOPE (deliberately narrow):
//! CPU exceptions ONLY. No PIC/APIC remap yet, so no hardware IRQs arrive
//! and interrupts stay disabled (`sti` is never executed). That makes every
//! handler non-reentrant by construction: no handler can preempt another,
//! so sharing the serial lock with the rest of the kernel is safe.
//!
//! KNOWN LIMITATION (documented, not hidden):
//! The double-fault handler has NO separate IST stack yet (that needs a GDT
//! plus TSS, next RFC). A stack-overflow double fault will therefore still
//! triple-fault. Every other registered exception reports cleanly.
//!
//! WIRING:
//! `init()` builds the table once (`spin::Once`) and loads it with `lidt`.
//! `kernel_main` then runs an `int3` self-test proving a handler runs AND
//! returns to the interrupted code.

use spin::Once;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

/// The single system IDT. `Once` guarantees exactly one build + load; a
/// second `init()` call is a harmless no-op, never a double-`lidt`.
static IDT: Once<InterruptDescriptorTable> = Once::new();

/// Build and load the IDT. Must be called once from `kernel_main` before
/// any exception can occur (in practice: right after console init).
pub fn init() {
    let idt = IDT.call_once(|| {
        let mut idt = InterruptDescriptorTable::new();
        // Recoverable: prints the frame, then returns to the caller.
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        // Fatal: print diagnostics, then halt (never return).
        idt.double_fault.set_handler_fn(double_fault_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault
            .set_handler_fn(general_protection_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        idt
    });
    // `lidt`: loads our table address into the CPU's IDTR register.
    // Safe: the table lives in a `static` (never moves, never drops).
    idt.load();
}

/// Breakpoint (#BP, vector 3). Raised by `int3`; recoverable.
///
/// Prints the interrupted instruction pointer so a reviewer can verify the
/// faulting address matches the `int3` site, then returns normally.
extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    crate::println!("[idt] breakpoint hit — returning to kernel");
    crate::println!("[idt] interrupted frame: {:#?}", stack_frame);
}

/// Double fault (#DF, vector 8). Diverging: never returns.
///
/// Raised when the CPU fails WHILE delivering a previous exception (e.g.
/// IDT entry missing, kernel stack unmapped). Prints the error code and the
/// frame, then halts. See module docs for the missing-IST limitation.
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
