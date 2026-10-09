//! PrismOS 8254 PIT driver (Phase 2, step 3b: timer bring-up).
//!
//! ROLE:
//! Programs PIT channel 0 to fire IRQ0 at `TARGET_TIMER_HZ` (100 Hz). This
//! module exists because real firmware cannot be trusted here: OVMF leaves
//! the PIT unprogrammed and the PIC fully masked (it prefers APIC/HPET), so
//! after a UEFI boot our timer IRQ would NEVER fire without explicit
//! bring-up. Symptoms of skipping this: `timer` shell command stuck at 0
//! despite `sti` and a remapped PIC — exactly what we observed.
//!
//! PROTOCOL (Intel 8254, channel 0, mode 3 square-wave): first write 0x36
//! to command port 0x43 (channel 0, lobyte/hibyte access, mode 3, binary),
//! then the divisor lobyte and hibyte to data port 0x40. The divisor itself
//! comes from `prism_core::pit` (tested arithmetic); this file only moves
//! those two bytes to hardware.
//!
//! SAFETY: raw port I/O to the standard, PC-fixed PIT ports, executed once
//! at boot (single-core, interrupts disabled during `init`). Wrong ports
//! would program the wrong device — the constants below are the IBM PC
//! standard, unchanged since 1981, asserted by every OS on the planet.

use x86::io::outb;

/// PIT command port (write-only): selects channel, access mode, waveform.
const PIT_COMMAND_PORT: u16 = 0x43;

/// PIT channel-0 data port (the one wired to IRQ0).
const PIT_CHANNEL0_PORT: u16 = 0x40;

/// Command byte: channel 0 | lobyte/hibyte | mode 3 (square wave) | binary.
/// 0b00_11_011_0 = 0x36. See module docs for the field breakdown.
const PIT_MODE_SQUARE_WAVE: u8 = 0x36;

/// Program channel 0 for `TARGET_TIMER_HZ` square-wave interrupts.
///
/// Call once from `interrupts::init()`, BEFORE unmasking IRQ0: programming
/// first avoids a burst of misrated ticks between unmask and setup.
pub fn init() {
    let divisor = prism_core::pit::divisor_for_rate(prism_core::pit::TARGET_TIMER_HZ);
    unsafe {
        // SAFETY: fixed PC-standard PIT ports (see module docs); single
        // boot-time use with interrupts disabled, so no concurrent port
        // access from a handler can interleave the 3-byte sequence.
        outb(PIT_COMMAND_PORT, PIT_MODE_SQUARE_WAVE);
        outb(PIT_CHANNEL0_PORT, (divisor & 0xFF) as u8);
        outb(PIT_CHANNEL0_PORT, (divisor >> 8) as u8);
    }
    crate::println!(
        "[pit] channel 0 at {} Hz (divisor {})",
        prism_core::pit::TARGET_TIMER_HZ,
        divisor
    );
}
