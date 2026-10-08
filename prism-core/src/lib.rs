//! PrismOS core logic.
//!
//! ROLE:
//! Pure, hardware-free modules shared by the bare-metal kernel. This crate
//! contains ZERO port I/O, ZERO interrupts, ZERO globals: only data
//! structures and algorithms. That is what makes it testable with plain
//! `cargo test` on any laptop.
//!
//! `no_std` STRATEGY:
//! `#![cfg_attr(not(test), no_std)]` means: freestanding (core only) in the
//! real kernel build, but linked with `std` under `cargo test` so the test
//! harness works. All code here must use `core` only — never `std` — so
//! both configurations compile from the same source.
//!
//! MODULES:
//!   - `memory`    : physical RAM statistics + bump frame allocator.
//!   - `scheduler` : cooperative round-robin task table.
//!   - `parser`    : shell command parsing (no I/O, no hardware).
#![cfg_attr(not(test), no_std)]

pub mod memory;
pub mod parser;
pub mod scheduler;
