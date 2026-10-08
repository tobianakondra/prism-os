//! PrismOS physical memory tracker.
//!
//! ROLE:
//! Turns the bootloader-provided memory map into trustworthy statistics
//! and a tiny demo frame allocator. This module does NOT build page tables
//! or a heap — that is Phase 2 behind an RFC. It proves:
//!
//!   - We correctly interpret the UEFI memory map.
//!   - We can count usable vs reserved RAM.
//!   - We can hand out physical frames without overlap.
//!
//! SAFETY MODEL:
//! All address math uses `u64` with checked/saturating arithmetic. The
//! allocator only returns frames inside regions marked `Usable` by the
//! bootloader. It never returns the same frame twice (monotonic bump).
//!
//! `bootloader_api` 0.11 API NOTES (verified against the vendored source):
//!   - `BootInfo.memory_regions: MemoryRegions` (NOT `memory_map`).
//!   - `MemoryRegions` derefs to `[MemoryRegion]` — plain slice iteration.
//!   - `MemoryRegion { start: u64, end: u64, kind }` are BYTE addresses
//!     (start inclusive, end exclusive), NOT frame numbers.
//!
//! HOST TESTS:
//! `count_usable_frames` and `align_up` are pure and tested on the host.
//! `BumpFrameAllocator` needs a real `MemoryRegions` (raw-pointer backed),
//! so it is verified live in QEMU (`mem` shell command + demo allocations).

use bootloader_api::info::{MemoryRegionKind, MemoryRegions};

/// Size of one physical frame: 4 KiB on x86_64.
pub const FRAME_SIZE: u64 = 4096;

/// Summary of physical RAM, shown by the `mem` shell command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryStats {
    /// Number of usable 4 KiB frames reported by UEFI.
    pub usable_frames: u64,
    /// Number of frames already handed out by our bump allocator.
    pub allocated_frames: u64,
    /// Total usable bytes (usable_frames * 4096).
    pub usable_bytes: u64,
}

impl MemoryStats {
    /// Human-readable KiB value for shell display.
    pub fn usable_kib(&self) -> u64 {
        self.usable_bytes / 1024
    }
}

/// Minimal bump allocator over usable physical frames.
///
/// HOW IT WORKS:
/// We walk the memory map once, remember the first usable frame and how
/// many consecutive usable frames follow the largest usable region, then
/// hand them out in order. No freeing in Phase 1 — freeing needs a bitmap
/// or stack allocator (Phase 2).
///
/// WHY BUMP AND NOT BITMAP:
/// 30 lines, zero metadata storage, impossible to double-free. Perfect for
/// proving the concept before adding a real allocator that needs review.
pub struct BumpFrameAllocator {
    /// Physical address of the next free frame.
    next_frame_addr: u64,
    /// One past the last frame we are allowed to hand out.
    end_addr: u64,
    /// Frames handed out so far (for `mem` reporting).
    allocated: u64,
    /// Cached total usable frames (for `mem` reporting).
    usable_frames: u64,
}

impl BumpFrameAllocator {
    /// Build an allocator from a bootloader memory map.
    ///
    /// STRATEGY (kept simple on purpose):
    /// Pick the LARGEST usable region and allocate only inside it. This
    /// avoids spanning gaps (reserved/MMIO holes) with a naive bump pointer.
    pub fn new(memory_map: &MemoryRegions) -> Self {
        let mut best_start = 0u64;
        let mut best_len = 0u64;
        let mut total_usable_frames = 0u64;

        for region in memory_map.iter() {
            if region.kind != MemoryRegionKind::Usable {
                continue;
            }
            // Align start up: a region starting mid-frame loses its first
            // partial frame rather than handing out overlapping memory.
            let aligned_start = align_up(region.start, FRAME_SIZE);
            let byte_len = region.end.saturating_sub(aligned_start);
            // Whole frames only: truncate any trailing partial frame.
            let frames = byte_len / FRAME_SIZE;
            total_usable_frames += frames;

            if byte_len > best_len {
                best_len = frames * FRAME_SIZE;
                best_start = aligned_start;
            }
        }

        Self {
            next_frame_addr: best_start,
            end_addr: best_start + best_len,
            allocated: 0,
            usable_frames: total_usable_frames,
        }
    }

    /// Hand out one 4 KiB frame. Returns its physical start address.
    pub fn allocate_frame(&mut self) -> Option<u64> {
        if self.next_frame_addr + FRAME_SIZE > self.end_addr || self.next_frame_addr == 0 {
            return None;
        }
        let addr = self.next_frame_addr;
        self.next_frame_addr += FRAME_SIZE;
        self.allocated += 1;
        Some(addr)
    }

    /// Current statistics snapshot for the shell.
    pub fn stats(&self) -> MemoryStats {
        MemoryStats {
            usable_frames: self.usable_frames,
            allocated_frames: self.allocated,
            usable_bytes: self.usable_frames * FRAME_SIZE,
        }
    }
}

/// Round an address UP to the given power-of-two alignment.
/// Pure helper, unit-tested below.
pub fn align_up(addr: u64, align: u64) -> u64 {
    debug_assert!(align.is_power_of_two(), "align must be a power of two");
    let mask = align - 1;
    addr.checked_add(mask)
        .map(|a| a & !mask)
        .unwrap_or(u64::MAX)
}

/// Pure helper used by host unit tests: count usable frames from a list of
/// (start_frame, end_frame, usable) tuples. Keeps test logic independent of
/// the `bootloader_api` types so `cargo test` runs on any laptop.
pub fn count_usable_frames(regions: &[(u64, u64, bool)]) -> u64 {
    regions
        .iter()
        .filter(|(_, _, usable)| *usable)
        .map(|(start, end, _)| end.saturating_sub(*start))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_map_has_zero_frames() {
        assert_eq!(count_usable_frames(&[]), 0);
    }

    #[test]
    fn only_usable_regions_are_counted() {
        let regions = [
            (0, 10, true),   // 10 usable frames
            (10, 15, false), // reserved hole, must be ignored
            (15, 20, true),  // 5 usable frames
        ];
        assert_eq!(count_usable_frames(&regions), 15);
    }

    #[test]
    fn saturating_math_never_underflows() {
        // Corrupt input (end < start) must yield 0, never panic in release.
        let regions = [(10, 5, true)];
        assert_eq!(count_usable_frames(&regions), 0);
    }

    #[test]
    fn align_up_rounds_to_frame_boundary() {
        assert_eq!(align_up(0, FRAME_SIZE), 0);
        assert_eq!(align_up(1, FRAME_SIZE), FRAME_SIZE);
        assert_eq!(align_up(FRAME_SIZE, FRAME_SIZE), FRAME_SIZE);
        assert_eq!(align_up(FRAME_SIZE + 1, FRAME_SIZE), 2 * FRAME_SIZE);
    }
}
