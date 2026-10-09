//! PrismOS Global Descriptor Table + Task State Segment (Phase 2, step 2).
//!
//! ROLE:
//! Replaces the bootloader's GDT with our own, whose ONLY addition is a TSS
//! describing a dedicated double-fault stack (IST). From here on, a #DF
//! switches to a KNOWN-GOOD stack instead of faulting on the (possibly
//! destroyed) kernel stack — turning silent triple-faults into serial dumps.
//!
//! LAYOUT (5 slots, 40 bytes):
//!   slot 0 : null descriptor (required, never used).
//!   slot 1 : 64-bit kernel code segment (our CS after reload).
//!   slot 2 : 64-bit kernel data segment (our DS/ES/SS after reload).
//!   slots 3-4 : TSS descriptor (16 bytes wide, hence two slots).
//!
//! WHY A DATA SEGMENT (lesson learned the hard way):
//! Our first version had code + TSS only, and the very first `iretq` (return
//! from the `int3` self-test) raised `#GP(0x10)`. Cause: the bootloader left
//! SS = 0x10 (GDT index 2), and `iretq` VALIDATES the restored SS even in
//! long mode — but index 2 was now our TSS descriptor (a system descriptor,
//! not writable data). Keeping a data segment at index 2 makes every stale
//! bootloader selector (DS/ES/SS = 0x10) valid again, and we additionally
//! reload all three explicitly so no stale state survives. Only CS (mode)
//! and TR (TSS) are strictly required; DS/ES/SS reloads are hygiene.
//!
//! SAFETY (five `unsafe` operations, each justified on site):
//! 1. `CS::set_reg` — selector points at the code descriptor we just loaded.
//! 2. `DS/ES/SS::set_reg` — selector points at the data descriptor we loaded.
//! 3. `load_tss` — selector points at the TSS descriptor we just loaded,
//!    describing a static TSS that is never moved nor aliased.
//! 4. Stack address math — `addr_of!` (never a guessed constant), boot-time
//!    single-core, interrupts disabled, so no concurrent mutation.
//!
//! WIRING:
//! `init()` runs BEFORE `interrupts::init()`: the IDT's double-fault entry
//! references the IST index defined here, and the CPU consults the TR the
//! moment a #DF fires — both must exist before any fault can occur.

use core::ptr::addr_of;
use spin::Once;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

/// IST index reserved for the double-fault stack. Shared with `interrupts`
/// so the IDT entry and the TSS slot can never disagree.
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

/// Size of the double-fault stack: 5 pages (20 KiB). A #DF handler prints a
/// large `InterruptStackFrame` with `{:#?}` formatting, so generosity here
/// is cheap insurance paid once at boot.
const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;

/// The actual stack bytes. Zeroed at boot; touched only by CPU-driven stack
/// switches during a double fault (plus our one-time address computation).
static mut DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];

/// The TSS. `Once` = built exactly once, then shared by reference forever.
static TSS: Once<TaskStateSegment> = Once::new();

/// Our GDT plus the selectors the CPU needs back (code + data + TSS).
struct Selectors {
    code_selector: SegmentSelector,
    data_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

/// The GDT itself. `Once` for the same exactly-once reason as the TSS.
static GDT: Once<(GlobalDescriptorTable, Selectors)> = Once::new();

/// Build our GDT + TSS, load them, reload CS and TR.
///
/// Must be called once from `kernel_main`, BEFORE `interrupts::init()`.
/// A second call is a harmless no-op (both `Once`s are already set).
pub fn init() {
    // 1. TSS first: the GDT's TSS descriptor borrows it, so it must exist.
    let tss = TSS.call_once(|| {
        let mut tss = TaskStateSegment::new();
        // Stacks grow DOWN: the CPU starts at `end` and pushes toward
        // `start`. Forgetting this inverts the stack into unmapped memory.
        let stack_start = VirtAddr::new(addr_of!(DOUBLE_FAULT_STACK) as u64);
        let stack_end = stack_start + DOUBLE_FAULT_STACK_SIZE as u64;
        tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_end;
        tss
    });

    // 2. GDT referencing the TSS. `tss_segment` takes `&'static` — `call_once`
    // returns a reference tied to the `static`, which satisfies the bound.
    let (gdt, selectors) = GDT.call_once(|| {
        let mut gdt = GlobalDescriptorTable::new();
        let code_selector = gdt.add_entry(Descriptor::kernel_code_segment());
        // Data BEFORE TSS on purpose: stale bootloader selectors (SS = 0x10,
        // i.e. index 2) must land on writable data, never on the TSS. See
        // module docs ("WHY A DATA SEGMENT").
        let data_selector = gdt.add_entry(Descriptor::kernel_data_segment());
        let tss_selector = gdt.add_entry(Descriptor::tss_segment(tss));
        (
            gdt,
            Selectors {
                code_selector,
                data_selector,
                tss_selector,
            },
        )
    });

    // 3. Activate. `lgdt` swaps the table; the reloads below point the CPU
    // at our descriptors. Interrupts are disabled during boot and we run
    // single-core, so no fault can interleave these instructions with
    // a half-loaded state.
    gdt.load();
    unsafe {
        // SAFETY: `code_selector` indexes the 64-bit code descriptor added
        // above in the just-loaded GDT; far-jump reload is the documented
        // way to refresh CS (Intel SDM Vol. 3A, SEGMENTATION chapter).
        CS::set_reg(selectors.code_selector);
        // SAFETY: `data_selector` indexes the writable data descriptor added
        // above; CPL 0 loading a present writable data segment into DS/ES/SS
        // is always valid and cannot fault.
        DS::set_reg(selectors.data_selector);
        ES::set_reg(selectors.data_selector);
        SS::set_reg(selectors.data_selector);
        // SAFETY: `tss_selector` indexes the TSS descriptor added above; the
        // TSS is `static` (never moves, never freed) and not busy (fresh
        // boot, TR still holds the bootloader's TSS).
        load_tss(selectors.tss_selector);
    }

    crate::println!("[gdt] GDT loaded (code + data + TSS), TR armed, IST0 = 20 KiB stack");
}
