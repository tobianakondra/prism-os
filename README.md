# PrismOS — Beautiful, Secure OS in Rust

> Phase 1: UEFI-booted minimal kernel with serial shell, memory stats, and cooperative scheduler.
> Goal: a tiny, hyper-maintainable foundation that any Rust developer can audit in one evening.

## Why PrismOS?

Linux, Windows, and macOS are written in C/C++ where ~70% of critical CVEs are memory-safety bugs.
PrismOS starts from zero in Rust:

- **Memory safety by construction** — no buffer overflows, no use-after-free, no data races.
- **Microkernel spirit** — tiny kernel, drivers and services isolated in user space (roadmap).
- **UEFI-first** — modern boot (GOP framebuffer, memory map), legacy BIOS only as fallback.
- **Beautiful by default** — GPU compositor and coherent design system (Phase 3+, see ARCHITECTURE.md).
- **AI-assisted, human-governed** — every AI draft is reviewed by a human; `unsafe` needs written justification.

## Architecture (Phase 1)

```text
  UEFI firmware (OVMF in QEMU)
        |
  bootloader crate (Stage-2, audited upstream)
        |  BootInfo { memory_regions, framebuffer }
        v
  kernel (no_std, x86_64-unknown-none, thin wiring only)
   |- main.rs       boot sequence + logger demo task + main loop
   |- console.rs    serial COM1 + GOP background paint
   `- shell.rs      serial line editor + execute (grammar in prism-core)
  prism-core (no_std-compatible lib, host-tested)
   |- memory.rs     BumpFrameAllocator over UEFI usable RAM + stats
   |- scheduler.rs  cooperative round-robin (no interrupts yet)
   `- parser.rs     shell grammar: help|mem|tasks|uptime|echo|clear|banner
```

Full vision (microkernel, capability security, compositor) lives in `ARCHITECTURE.md`.

## Prerequisites

- Rust NIGHTLY via rustup (pinned in `rust-toolchain.toml`)
- `qemu-system-x86_64`
- OVMF firmware (Arch: `sudo pacman -S edk2-ovmf`)
- Target `x86_64-unknown-none` (installed automatically via rust-toolchain)

Why nightly? The audited `bootloader` crate builds its BIOS/UEFI stages
with unstable Cargo/Rust features. Every mainstream Rust OS (Redox,
blog_os) does the same. Pure logic stays `core`-only and host-tested, so
nightly affects image building only — never code review.

Check:

```sh
rustc --version
qemu-system-x86_64 --version
ls /usr/share/edk2/x64/OVMF.4m.fd
```

## Build and boot (one command)

```sh
# UEFI (primary, recommended)
cargo run-uefi
# equivalent: cargo run -- --uefi

# Legacy BIOS fallback
cargo run-bios
```

What you should see in your terminal (serial):

```text
  ____       _                   ___  ____
 |  _ \ _ __(_)___ _ __ ___    / _ \/ ___|
 ...

  Beautiful, secure OS in Rust - Phase 1 (UEFI + serial shell)
  Type 'help' to list commands.
[boot] PrismOS kernel entered via UEFI bootloader
[mem] usable: ... frames (... KiB)
[sched] 2 demo tasks spawned (cooperative)
[shell] serial shell ready. Type 'help'.
prism>
```

Try: `help`, `mem`, `tasks`, `echo hello prism`, `uptime`, `banner`, `clear`.

Quit QEMU: `Ctrl-A` then `X` (serial is attached to stdio, display is none).

## Quality gates (zero compromise)

Every PR must pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy -p kernel --target x86_64-unknown-none -- -D warnings
cargo test
cargo build -p kernel --target x86_64-unknown-none
cargo run -- build   # build target/images/{uefi,bios}.img
```

`cargo clippy` runs on default members (host tool + `prism-core`) only:
the `kernel` binary cannot compile for the host (bare-metal `_start`,
freestanding panic handler), so it gets its own target-scoped invocation.
Do NOT use `cargo clippy --workspace --all-targets` — it force-builds the
kernel for Linux and fails by design. Same rule as `cargo test`.

- `rustfmt` with `rustfmt.toml` (max 100 cols).
- `clippy` strict, warnings denied.
- Host unit tests for all pure logic (`prism-core`: memory, scheduler,
  shell parser). The `kernel` binary is excluded from `cargo test` on
  purpose: `entry_point!` emits a bare-metal `_start` that cannot link on
  the host. It is verified by freestanding build + live QEMU boot instead.
- No `unsafe` in our code except two documented spots: the COM1 port
  construction (`console.rs`, standard address) and the framebuffer slice
  (inside the audited `bootloader_api`, not our code).

## Repository layout

```text
prism-os/
  Cargo.toml            host dev-tool + workspace (default: host + prism-core)
  src/main.rs           build images + launch QEMU (stable-compatible, no bindeps)
  prism-core/           pure logic lib (host-tested, no_std-compatible)
    src/lib.rs          crate docs + no_std strategy
    src/memory.rs       frame allocator + stats
    src/scheduler.rs    cooperative scheduler
    src/parser.rs       shell grammar
  kernel/
    Cargo.toml          no_std kernel (test=false: bare-metal only)
    src/main.rs         entry point + boot sequence
    src/console.rs      serial + GOP paint
    src/shell.rs        serial line editor + execute
  docs/                 boot sequence, ADRs
  ARCHITECTURE.md       vision + crate map + roadmap
  CONTRIBUTING.md       how to contribute (read first)
```

## Contributing

Read `CONTRIBUTING.md` first. TL;DR:

1. Open an issue or pick a `good first issue`.
2. Keep crates small; one file = one responsibility.
3. English comments; every public item documented.
4. Add a host test with every pure function.
5. One-command verification before push (see gates above).

## Roadmap

- [x] Phase 1 — UEFI boot, serial shell, mem stats, cooperative scheduler
- [ ] Phase 2 — interrupts, keyboard, framebuffer text, bitmap allocator, heap
- [ ] Phase 3 — user space, syscalls, capability IPC, driver isolation
- [ ] Phase 4 — GPU compositor, design system, App Store sandbox

See `ARCHITECTURE.md` for details and non-goals.

## License

MIT. See `LICENSE-MIT`.
