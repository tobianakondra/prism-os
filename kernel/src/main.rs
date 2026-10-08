//! PrismOS kernel entry point.
//!
//! ROLE:
//! The thinnest possible `no_std` kernel that proves the full boot chain:
//! UEFI firmware -> `bootloader` crate -> `kernel_main` -> serial shell.
//!
//! WHAT HAPPENS AT BOOT (in order):
//!   1. `kernel_main` receives `BootInfo` (memory regions + framebuffer).
//!   2. Console inits (serial + framebuffer background paint).
//!   3. Banner prints so you KNOW you booted PrismOS, not garbage.
//!   4. Memory stats are computed + a few frames allocated as proof.
//!   5. Two demo tasks spawn (heartbeat + logger).
//!   6. Main loop: scheduler tick + serial shell poll, forever.
//!
//! WHAT THIS FILE DOES NOT DO (on purpose):
//! No interrupts, no heap, no paging setup, no keyboard driver, no
//! filesystem. Each of those is a separate RFC + module so reviewers can
//! approve them one at a time.
//!
//! TESTING:
//! This binary is NEVER built for the host (`test = false` in Cargo.toml):
//! `entry_point!` emits a bare-metal `_start` that collides with the host
//! libc at link time. All logic lives in `prism-core` (host-tested); this
//! file is verified by `cargo build -p kernel --target x86_64-unknown-none`
//! plus a live QEMU boot.
//!
//! PANIC STRATEGY:
//! Kernel panics print to serial and halt with `hlt`.

#![no_std]
#![no_main]
// Deny the most common review complaints at compile time.
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use bootloader_api::{entry_point, BootInfo};
use core::panic::PanicInfo;

mod console;
mod shell;

use prism_core::memory::BumpFrameAllocator;
use prism_core::scheduler::{Scheduler, Task};

// The bootloader crate calls this symbol after setting up long mode,
// a framebuffer, and a memory map. Signature is fixed by `bootloader_api`.
entry_point!(kernel_main);

/// Kernel entry point. Never returns (`!`).
fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // `Optional::as_mut` converts the FFI-safe framebuffer slot to a plain
    // `Option<&mut FrameBuffer>` with no unsafe on our side.
    let framebuffer = boot_info.framebuffer.as_mut();

    console::init(framebuffer);

    shell::print_banner();
    println!("[boot] PrismOS kernel entered via UEFI bootloader");

    // --- Memory ---
    // NOTE: `bootloader_api` 0.11 names the field `memory_regions`
    // (type `MemoryRegions`, byte-address `MemoryRegion`s).
    let mut frame_alloc = BumpFrameAllocator::new(&boot_info.memory_regions);
    let stats_before = frame_alloc.stats();
    println!(
        "[mem] usable: {} frames ({} KiB)",
        stats_before.usable_frames,
        stats_before.usable_kib()
    );

    // Prove the allocator hands out distinct frames (not just counting).
    // We allocate 3 and print their addresses: a reviewer can verify in the
    // QEMU log that they are 4 KiB apart and inside usable RAM.
    for i in 0..3 {
        match frame_alloc.allocate_frame() {
            Some(addr) => println!("[mem] demo frame {i}: phys=0x{addr:016x}"),
            None => println!("[mem] demo frame {i}: allocation FAILED"),
        }
    }

    // --- Scheduler ---
    let mut sched = Scheduler::new();
    sched.spawn(Task::new(
        "heartbeat",
        prism_core::scheduler::heartbeat_step,
    ));
    sched.spawn(Task::new("logger", logger_step));
    println!(
        "[sched] {} demo tasks spawned (cooperative)",
        sched.task_count()
    );

    // --- Shell ---
    println!("[shell] serial shell ready. Type 'help'.");
    let mut shell = shell::Shell::new();
    shell.print_prompt();

    // Main loop: cooperative multitasking + interactive shell.
    // No interrupts yet, so we poll. `spin_loop` hints the CPU we are
    // busy-waiting (power-friendly on real hardware, fast in QEMU).
    loop {
        sched.tick();

        // Drain all pending serial bytes (usually 0 or 1 per tick).
        while let Some(byte) = console::try_read_byte() {
            if let Some(completed) = shell.push_byte(byte) {
                let stats = frame_alloc.stats();
                // `completed` is owned, so no borrow conflict with `shell`.
                shell.execute(completed.as_str(), stats, &sched);
                shell.reset_line();
                shell.print_prompt();
            }
        }

        core::hint::spin_loop();
    }
}

/// Demo workload 2 (kernel-side): periodic liveness log.
///
/// Lives in the kernel (NOT in `prism-core`) because it performs I/O
/// (`println!` to serial). Prints once every 200 runs so the boot log stays
/// readable while proving the scheduler keeps calling us.
fn logger_step(task: &mut Task) {
    if task.run_count % 200 == 1 {
        println!(
            "[sched] logger task '{}' alive (runs={})",
            task.name, task.run_count
        );
    }
}

/// Panic handler: print diagnostics to serial, then halt.
///
/// `hlt` in a loop keeps QEMU attached for debugging instead of
/// triple-faulting into a reboot loop.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // `println!` is safe here: serial lock is re-entrant-safe enough for a
    // panic path in single-core Phase 1 (no interrupt can preempt us).
    println!();
    println!("[PANIC] {}", info);
    println!("[PANIC] system halted. Restart QEMU to reboot.");

    loop {
        x86_64::instructions::hlt();
    }
}
