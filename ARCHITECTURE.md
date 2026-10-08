# PrismOS Architecture

This document is the single source of truth for WHERE code lives and WHY.
Update it with every structural change; stale architecture docs divide communities.

## 1. Vision

A Rust OS that is secure by construction AND beautiful by default:

- Microkernel-inspired: minimal trusted kernel, everything else isolated.
- Capability-based security: no all-powerful root, per-app permissions.
- Atomic A/B updates, verified boot, encrypted by default (Phase 3+).
- Modern compositor: 120Hz+, HDR, coherent design system (Phase 4).

We deliberately do NOT chase POSIX compatibility or run Linux binaries in Phase 1-2.
Clarity beats compatibility while the foundation is young.

## 2. Crate map (Phase 1)

```text
prism-os (host, std)            dev tool: build images + launch QEMU
  src/main.rs                   `cargo run -- --uefi|--bios|build`

prism-core (lib, core-only)     pure logic, host-tested, no hardware
  memory.rs                     UEFI regions -> stats + bump allocator
  scheduler.rs                  cooperative round-robin, zero unsafe
  parser.rs                     shell grammar (help|mem|tasks|...)

kernel (no_std bin)             thin hardware wiring, bare-metal only
  main.rs                       boot sequence + logger task + main loop
  console.rs                    serial logging + GOP background
  shell.rs                      serial line editor + execute
```

Rules:

- `prism-core` uses `core` ONLY (`#![cfg_attr(not(test), no_std)]`):
  no port I/O, no interrupts, no globals, no printing. This is what makes
  `cargo test` meaningful — every pure function has a host test.
- `kernel/` never depends on `std`. Only `core`, `bootloader_api`,
  `prism-core`, `x86_64` (instructions only), `uart_16550`, `spin`.
- No cross-imports between core modules except via explicit parameters
  (`MemoryStats`, `&Scheduler`). No globals except the serial lock.
- `kernel/main.rs` is wiring only: init, spawn, loop. Grammar and
  algorithms live in `prism-core`.

## 2b. Testing strategy (why the kernel is excluded from `cargo test`)

`bootloader_api::entry_point!` emits a bare-metal `_start` symbol. Linking
the kernel for the host collides with libc (`duplicate symbol: _start`),
so the kernel sets `test = false` and is excluded from default workspace
members. Verification is split honestly:

- `cargo test` → `prism-core` (+ host tool): pure logic, real assertions.
- `cargo build -p kernel --target x86_64-unknown-none` → it links freestanding.
- `cargo run -- --uefi` → live QEMU boot; `help`, `mem`, `tasks` exercised.
- CI runs all three; a PR without the QEMU boot log is incomplete.

## 3. Boot sequence (UEFI path)

1. QEMU loads OVMF (`-bios OVMF.4m.fd`) — emulated UEFI firmware.
2. OVMF reads GPT `uefi.img`, runs the bootloader's UEFI application.
3. Bootloader collects UEFI memory map + GOP framebuffer, `ExitBootServices`.
4. Bootloader jumps to `kernel_main(BootInfo)` in 64-bit long mode.
5. Kernel inits serial, loads IDT, runs `int3` self-test, paints background,
   prints banner, inits alloc/sched/shell.
6. Kernel loops forever: `sched.tick()` + `shell.poll()`.

## 4. Safety and review policy

- Phase 1: `unsafe` appears in exactly ONE documented spot in our code:
  the COM1 `SerialPort::new(0x3F8)` static construction (`console.rs`,
  standard PC address, SAFETY comment on site). Everything else `unsafe`
  lives in pinned upstream crates (`bootloader`, `uart_16550`, `x86_64`).
- Any future `unsafe` MUST carry:
  `// SAFETY: <what invariant holds> <why it holds> <how it is tested>`
  plus a host or QEMU test, plus one human reviewer approval.
- `panic =` strategy: kernel halts with serial diagnostics. No silent reboot.
- Fuzz pure parsers (`shell::parse_command`) in CI when they grow.

## 5. Roadmap

### Phase 1 (this repo state) — DONE
UEFI boot, framebuffer proof, serial shell, bump allocator stats,
cooperative scheduler, host tests, one-command QEMU.

### Phase 2 — Interactive kernel (in progress, one RFC-sized step at a time)
- [x] CPU exceptions: IDT with breakpoint + double/page/GP/invalid-opcode
  handlers, serial dumps, `int3` boot self-test (`kernel/src/interrupts.rs`).
- [ ] GDT + TSS with IST stack for the double-fault handler (next).
- [ ] PIC remap + timer IRQ (tick source for preemption later).
- [ ] Keyboard: PS/2 scancode driver over interrupts.
- [ ] Framebuffer text: embedded font, scrolling console (replaces serial-only).
- [ ] Real allocators: bitmap/frame stack + linked-list heap + `#[global_allocator]`.
- [ ] Preemptive scheduler: timer-driven, still no user space.

### Phase 3 — Isolation
User mode (ring 3), syscalls, capability IPC, userspace drivers,
verified boot chain notes, A/B update design.

### Phase 4 — Beauty
GPU compositor prototype, input stack (mouse/touch), design system,
sandboxed app format, App Store sketch.

## 6. Non-goals (explicit)

- No custom UEFI loader in Phase 1 (reuse audited `bootloader` crate).
- No SMP in Phase 1 (`-smp 1` only; locking is future-proofing, not proof).
- No network, no filesystem, no GUI in Phase 1.
- No `unsafe` assembly context switch without a dedicated RFC + tests.
