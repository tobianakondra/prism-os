//! PrismOS PIT divisor math (pure, no I/O).
//!
//! ROLE:
//! Computes the 8254 Programmable Interval Timer divisor for a requested
//! interrupt rate. The actual port writes live in the kernel (`pit`
//! module); this file owns ONLY the arithmetic, so it can be unit-tested
//! on the host. A wrong divisor does not crash anything — it just changes
//! the tick rate — but an untested divide-by-zero or overflow would be
//! embarrassing in bring-up code reviewers read first.
//!
//! HARDWARE FACTS (Intel 8254 + PC platform):
//! The PIT input clock runs at 1_193_182 Hz. Channel 0 fires IRQ0 every
//! `divisor` input ticks, so `rate = 1_193_182 / divisor`. The divisor is
//! 16-bit: valid range 1..=65535, where the register value 0 MEANS 65536
//! (slowest, ~18.2 Hz). PrismOS Phase 2 runs the timer at 100 Hz.

/// PIT input clock frequency in Hz (the infamous 1.193182 MHz).
pub const PIT_BASE_FREQUENCY_HZ: u32 = 1_193_182;

/// PrismOS timer rate: 100 IRQs per second. Fast enough for a future
/// preemptive scheduler quantum, slow enough that the EOI overhead is noise.
pub const TARGET_TIMER_HZ: u32 = 100;

/// Compute the 16-bit PIT divisor for `target_hz`.
///
/// - `target_hz == 0` (nonsense input): slowest possible rate (65535).
/// - Rates needing a divisor < 1 (above base clock): fastest (1).
/// - Rates needing a divisor > 65535 (below ~18.2 Hz): slowest (65535).
///   Integer division truncates, so the real rate is always >= requested
///   (up to +1 Hz error) — documented, never hidden.
pub fn divisor_for_rate(target_hz: u32) -> u16 {
    if target_hz == 0 {
        return u16::MAX;
    }
    let divisor = PIT_BASE_FREQUENCY_HZ / target_hz;
    divisor.clamp(1, u16::MAX as u32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hundred_hertz_gives_expected_divisor() {
        // 1_193_182 / 100 = 11931 (truncated). Real rate: ~100.007 Hz.
        assert_eq!(divisor_for_rate(100), 11931);
    }

    #[test]
    fn zero_rate_requests_slowest_not_panic() {
        assert_eq!(divisor_for_rate(0), u16::MAX);
    }

    #[test]
    fn extreme_rates_clamp_to_hardware_range() {
        // Faster than the input clock: fastest possible.
        assert_eq!(divisor_for_rate(u32::MAX), 1);
        assert_eq!(divisor_for_rate(PIT_BASE_FREQUENCY_HZ), 1);
        // Slower than the 16-bit range allows: slowest possible.
        assert_eq!(divisor_for_rate(1), u16::MAX);
        assert_eq!(divisor_for_rate(18), u16::MAX);
    }
}
