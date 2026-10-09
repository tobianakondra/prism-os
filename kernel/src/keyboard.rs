//! PrismOS PS/2 keyboard driver (Phase 2, step 4: input).
//!
//! ROLE:
//! Turns IRQ1 scancodes into bytes the shell can consume. Chain per keypress:
//! 8042 controller raises IRQ1 -> `irq_handler` reads port 0x60 -> `pc-keyboard`
//! decodes Set-1 scancodes with the US-104 layout -> Unicode chars are pushed
//! as UTF-8 bytes into the shared queue -> main loop drains via `pop_key`.
//!
//! WHAT IS IGNORED (on purpose, for now):
//! - Non-Unicode keys (arrows, F-keys, modifiers alone): decoded as `RawKey`
//!   and dropped. They need escape-sequence handling (Phase 3 console).
//! - Decode errors (stray prefix bytes): dropped; the next press resyncs.
//! - Queue overflow past 256 bytes: newest byte dropped (see `KeyQueue`).
//!
//! LOCK DISCIPLINE (no nesting, ever):
//! The handler takes each lock in its own scope and drops it before the
//! next: decode under `KEYBOARD`, then queue under `QUEUE`, then the EOI
//! under `PICS`. The consumer (`pop_key`) runs with interrupts masked
//! (`without_interrupts`), so it can never be preempted mid-pop by the
//! producer — see the concurrency contract in `prism_core::keyqueue`.
//! LAYOUT NOTE: US-104 is hardcoded for Phase 2 (code and docs are English).
//! Upstream ships an Azerty layout; swapping it later is a `layouts` import
//! plus a shell `layout` command — no driver change needed.
//!
//! PORT NOTE: 0x60 is the IBM PC standard PS/2 data port, read-only here.
//! The write/command port 0x64 is never touched (no LED or rate commands
//! yet), so this driver cannot misconfigure the controller.

use pc_keyboard::{layouts, DecodedKey, HandleControl, ScancodeSet1};
use prism_core::keyqueue::KeyQueue;
use spin::Mutex;
use x86::io::inb;

/// PS/2 controller data port: a pending scancode byte waits here when IRQ1
/// fires. Read-only in this driver.
const PS2_DATA_PORT: u16 = 0x60;

/// Scancode decoder + layout state (shift tracking, multi-byte prefixes).
/// `Mutex` because handler and (never concurrent) future users share it;
/// the IRQ itself serializes handler executions.
static KEYBOARD: Mutex<pc_keyboard::PS2Keyboard<layouts::Us104Key, ScancodeSet1>> =
    Mutex::new(pc_keyboard::PS2Keyboard::new(
        ScancodeSet1::new(),
        layouts::Us104Key,
        HandleControl::Ignore,
    ));

/// Decoded bytes waiting for the shell. Producer: `irq_handler`.
/// Consumer: `pop_key` (main loop, interrupts masked).
static QUEUE: Mutex<KeyQueue> = Mutex::new(KeyQueue::new());

/// IRQ1 handler (vector 33). Reads ONE scancode, decodes it, queues any
/// resulting text, then EOIs. Fast by design: the only waits are three port
/// reads/writes and two short critical sections, never nested.
pub extern "x86-interrupt" fn irq_handler(
    _stack_frame: x86_64::structures::idt::InterruptStackFrame,
) {
    // 1. Read the scancode. IRQ1 guarantees data is ready.
    let scancode = unsafe {
        // SAFETY: read-only access to the standard PS/2 data port; the IRQ
        // itself is the readiness proof, so no status-port polling needed.
        inb(PS2_DATA_PORT)
    };

    // 2. Decode under the keyboard lock, then RELEASE before queuing.
    let decoded = {
        let mut keyboard = KEYBOARD.lock();
        match keyboard.add_byte(scancode) {
            Ok(Some(event)) => keyboard.process_keyevent(event),
            // Ok(None): multi-byte prefix, more bytes coming. Err: stray
            // byte, next press resyncs. Both: nothing to queue.
            Ok(None) | Err(_) => None,
        }
    };

    // 3. Queue text (if any) under the queue lock, then release before EOI.
    if let Some(DecodedKey::Unicode(c)) = decoded {
        let mut queue = QUEUE.lock();
        let mut utf8 = [0u8; 4];
        for byte in c.encode_utf8(&mut utf8).bytes() {
            if !queue.push(byte) {
                break; // Full: drop the rest of this char, keep old bytes.
            }
        }
    }
    // (RawKey and unmapped events fall through: intentionally unconsumed.)

    // 4. EOI under the PIC lock. Last step so a nested timer IRQ (higher
    // urgency, same PIC) is never delayed by our locks — all dropped above.
    unsafe {
        // SAFETY: notifies exactly the vector just serviced (`Keyboard`)
        // after its work is done; same single-EOI-site argument as timer.
        crate::interrupts::PICS
            .lock()
            .notify_end_of_interrupt(crate::interrupts::InterruptIndex::Keyboard.as_u8());
    }
}

/// Drain one decoded byte for the shell, or `None` when the queue is empty.
///
/// Runs with interrupts masked so an IRQ1 cannot preempt the pop midway
/// (see the contract in `prism_core::keyqueue`). Called from the main loop,
/// never from a handler.
pub fn pop_key() -> Option<u8> {
    x86_64::instructions::interrupts::without_interrupts(|| QUEUE.lock().pop())
}
