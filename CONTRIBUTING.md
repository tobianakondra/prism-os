# Contributing to PrismOS

Thank you for considering a contribution. PrismOS is AI-assisted but
human-governed: clarity and reviewability matter more than speed.

## 1. Ground rules

1. **English only** in code, comments, and commit messages. The Rust OS
   community is global; French discussion is welcome in issues, code is English.
2. **One file, one job.** If you cannot describe a module in 5 lines, split it.
3. **Document every public item.** `cargo doc` must be warning-free.
4. **Test pure logic on the host.** `prism-core` (`memory`, `scheduler`,
   `parser`) owns all `#[cfg(test)]` coverage. The `kernel` binary is
   `test = false` (bare-metal `_start` cannot link on the host) — verify it
   via freestanding build + QEMU boot, never by forcing host tests.
5. **No drive-by `unsafe`.** See ARCHITECTURE.md safety policy.
6. **Keep `main.rs` files thin.** Wiring only; logic goes in modules.

## 2. Comment standard (mandatory)

Every file starts with a `//!` header answering:

- ROLE: what this file does and does not do.
- WHY: why this design and not the obvious alternative.
- SAFETY (if applicable): invariants, tests.
- TESTS: how to verify (`cargo test`, QEMU steps).

Every public function documents: what it does, what it does NOT do,
error cases, and a usage note. Bad: `// increment i`. Good: full doc comment.

## 3. Workflow

```sh
# 1. Sync and branch
git checkout main && git pull
git checkout -b feat/short-name

# 2. Write code + docs + tests together (never code without docs)

# 3. Verify (all six, in order)
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo clippy -p kernel --target x86_64-unknown-none -- -D warnings
cargo test
cargo build -p kernel --target x86_64-unknown-none
cargo run-uefi   # boot in QEMU, try `help`, `mem`, `tasks`

# 4. Commit (small, English, imperative)
git commit -m "shell: add uptime command with tick counter"

# 5. Push + open PR with: what, why, how tested, screenshot/log of QEMU boot
```

## 4. Good first issues (starter ladder)

1. Add a shell command (`version`, `help <topic>`) with parser test.
2. Improve `mem` output (MiB/GiB formatting) with test.
3. Add scheduler stats (`max tasks`, `idle ticks`) with test.
4. Write a `docs/` note (boot sequence, QEMU flags, OVMF troubleshooting).
5. Triage a clippy lint and document WHY the fix is correct.

Bigger work (interrupts, allocator, keyboard) needs an RFC in `docs/` first:
context, options considered, decision, consequences, test plan.

## 5. Review checklist (for reviewers)

- [ ] `cargo fmt --check` clean
- [ ] Both clippy invocations clean (host AND kernel target, see above)
- [ ] New pure logic has host tests in `prism-core`
- [ ] `cargo build -p kernel --target x86_64-unknown-none` passes
- [ ] QEMU boot log pasted in PR (`help`, `mem`, `tasks` exercised)
- [ ] No new `unsafe` without SAFETY comment + test
- [ ] Docs updated (`README` / `ARCHITECTURE` / `docs` if structure changed)

## 6. AI assistance disclosure

If you used AI to draft a PR, say so in the PR body: what was generated,
what you reviewed line-by-line, and what you tested on hardware/QEMU.
Undisclosed bulk-generated code will be rejected — not because it is AI,
but because it cannot be trusted without stated verification.
