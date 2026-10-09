# Boot sequence (Phase 1, UEFI)

Developer note: how PrismOS goes from power-on to `prism>` prompt in QEMU.

## Commands

```sh
cargo run -- build   # only build target/images/{uefi,bios}.img
cargo run-uefi       # build + boot UEFI in QEMU
```

`cargo run-uefi` expands (after building) to:

```sh
qemu-system-x86_64 \
  -bios /usr/share/edk2/x64/OVMF.4m.fd \
  -drive format=raw,file=target/images/uefi.img \
  -serial stdio -display none \
  -m 512M -smp 1
```

## Stages

1. **OVMF (UEFI firmware)**: initializes virtual hardware, exposes
   Boot Services (memory map, GOP graphics, block I/O).
2. **`bootloader` UEFI app** (from `uefi.img` ESP): loads `kernel` ELF,
   builds `BootInfo { memory_map, framebuffer, ... }`, calls
   `ExitBootServices`, jumps to `kernel_main`.
3. **PrismOS `kernel_main`**:
   - `console::init` — serial init + GOP background paint.
   - `gdt::init` — own GDT (code + TSS), reload CS, load TR (IST0 armed).
   - `interrupts::init` + `int3` self-test (proves a handler runs + returns).
   - banner + `[boot]` log line.
   - `BumpFrameAllocator::new(memory_regions)` + 3 demo allocations.
   - `Scheduler::new` + `heartbeat` + `logger` tasks.
   - `Shell::new` + prompt, then infinite `tick + poll` loop.

## Troubleshooting

- `OVMF firmware not found`: install `edk2-ovmf` (Arch) or adjust
  `OVMF_BIOS_PATHS` in `src/main.rs`.
- `/dev/kvm` errors: PrismOS never passes `-enable-kvm` on purpose.
  Remove any local alias that adds it.
- No serial output: check `-serial stdio` is present and that the guest
  reached `console::init` (framebuffer should still paint blue).
- Images are rebuilt on every `cargo run` invocation into
  `target/images/`; stale files are never reused silently because the
  kernel ELF is recompiled first via subprocess.
