//! PrismOS text shell (kernel side: line editing + execution).
//!
//! ROLE:
//! Tiny interactive shell over the serial port. It is the ONLY user
//! interface in Phase 1 and the primary demo that the kernel is alive:
//!
//!   prism> help
//!   prism> mem
//!   prism> tasks
//!
//! WHY SERIAL AND NOT KEYBOARD:
//! A PS/2 or USB keyboard driver needs interrupts + scancode tables —
//! Phase 2 work. QEMU `-serial stdio` turns your laptop keyboard into the
//! OS keyboard for free, so we get a real interactive shell on day one.
//!
//! SPLIT WITH `prism-core`:
//! Command GRAMMAR (`parse_command`, `ShellCommand`) lives in
//! `prism_core::parser` (pure, host-tested). This file owns only what needs
//! hardware: the line buffer, terminal echo, and `execute` (which prints to
//! serial and reads live `MemoryStats` / `Scheduler`).
//!
//! DESIGN:
//! Byte-at-a-time, no heap, fixed 256-byte line buffer. Backspace edits,
//! Enter executes. Longer lines are truncated with a bell rather than
//! overflowing.

use prism_core::memory::MemoryStats;
use prism_core::parser::{parse_command, ShellCommand, MAX_LINE_LEN};
use prism_core::scheduler::Scheduler;

/// A completed input line, OWNED (copied out of the shell buffer).
///
/// WHY OWNED AND NOT `&str`:
/// Returning `&str` would borrow `Shell` across the `execute` call, which
/// needs `&mut Shell`. That is a classic borrow-checker deadlock. Copying
/// 256 bytes once per Enter key is negligible and keeps call sites clean.
pub struct CompletedLine {
    buf: [u8; MAX_LINE_LEN],
    len: usize,
}

impl CompletedLine {
    /// View the completed line as text (lossy-safe: invalid UTF-8 becomes "").
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Interactive shell state. Owns the line buffer; the kernel owns one.
pub struct Shell {
    line: [u8; MAX_LINE_LEN],
    len: usize,
}

impl Shell {
    /// Fresh shell waiting for the first keystroke.
    pub const fn new() -> Self {
        Self {
            line: [0; MAX_LINE_LEN],
            len: 0,
        }
    }
    /// Print the prompt. Called once at boot and after every command.
    pub fn print_prompt(&self) {
        crate::print!("prism> ");
    }

    /// Feed one received byte into the line editor.
    ///
    /// Returns an owned `CompletedLine` when Enter is pressed, else None.
    /// Handles backspace (DEL + Ctrl+H) by erasing one char both in the
    /// buffer and on the terminal (destructive backspace sequence).
    pub fn push_byte(&mut self, byte: u8) -> Option<CompletedLine> {
        match byte {
            // Enter (CR or LF): echo newline, hand out an owned copy.
            b'\r' | b'\n' => {
                crate::print!("\n");
                let mut buf = [0u8; MAX_LINE_LEN];
                buf[..self.len].copy_from_slice(&self.line[..self.len]);
                Some(CompletedLine { buf, len: self.len })
            }
            // Backspace: DEL (127) or Ctrl+H (8).
            8 | 127 => {
                if self.len > 0 {
                    self.len -= 1;
                    // Erase the char on the user's terminal.
                    crate::print!("\x08 \x08");
                }
                None
            }
            // Printable ASCII: append if room, echo back.
            32..=126 => {
                if self.len < MAX_LINE_LEN {
                    self.line[self.len] = byte;
                    self.len += 1;
                    crate::print!("{}", byte as char);
                } else {
                    // Buffer full: beep-style feedback, keep old content.
                    crate::print!("\x07");
                }
                None
            }
            // Ignore other control bytes (arrows come as escapes; Phase 2).
            _ => None,
        }
    }

    /// Reset the line buffer after a command has executed.
    pub fn reset_line(&mut self) {
        self.len = 0;
    }

    /// Execute a completed line against live kernel state.
    pub fn execute(&mut self, line: &str, mem: MemoryStats, sched: &Scheduler) {
        match parse_command(line) {
            ShellCommand::Help => {
                crate::println!("PrismOS shell - commands:");
                crate::println!("  help          this message");
                crate::println!("  mem           physical memory statistics");
                crate::println!("  tasks         list scheduler tasks");
                crate::println!("  uptime        ticks since boot");
                crate::println!("  echo <text>   print text back");
                crate::println!("  banner        reprint boot banner");
                crate::println!("  clear         clear screen (scroll)");
                crate::println!("  overflow      crash test: stack overflow -> double fault");
            }
            ShellCommand::Mem => {
                crate::println!("Memory:");
                crate::println!("  usable frames : {}", mem.usable_frames);
                crate::println!("  usable        : {} KiB", mem.usable_kib());
                crate::println!("  allocated     : {} frames", mem.allocated_frames);
            }
            ShellCommand::Tasks => {
                crate::println!("Tasks ({}):", sched.task_count());
                sched.each_task(|t| {
                    crate::println!("  #{} {:<10} runs={}", t.id, t.name, t.run_count);
                });
                crate::println!("  ticks={}", sched.ticks);
            }
            ShellCommand::Uptime => {
                crate::println!("uptime: {} ticks", sched.ticks);
            }
            ShellCommand::Echo(rest) => {
                crate::println!("{rest}");
            }
            ShellCommand::Clear => {
                // Serial has no real clear; print 20 blank lines as scroll.
                for _ in 0..20 {
                    crate::println!();
                }
            }
            ShellCommand::Banner => {
                print_banner();
            }
            ShellCommand::Overflow => {
                crate::println!("overflowing the stack on purpose...");
                crate::println!("expect: [idt] FATAL double fault dump, then halt.");
                crate::stack_overflow();
            }
            ShellCommand::Unknown("") => {
                // Empty line: just re-prompt, no error.
            }
            ShellCommand::Unknown(other) => {
                crate::println!("unknown command: '{other}' (try 'help')");
            }
        }
    }
}

impl Default for Shell {
    /// Default = fresh shell. Required by Clippy (`new_without_default`).
    fn default() -> Self {
        Self::new()
    }
}

/// Print the boot banner. Shared between boot and the `banner` command so
/// there is exactly one copy of the ASCII art.
pub fn print_banner() {
    crate::println!();
    crate::println!("  ____       _                   ___  ____  ");
    crate::println!(" |  _ \\ _ __(_)___ _ __ ___    / _ \\/ ___| ");
    crate::println!(" | |_) | '__| / __| '_ ` _ \\  | | | \\___ \\ ");
    crate::println!(" |  __/| |  | \\__ \\ | | | | | | |_| |___) |");
    crate::println!(" |_|   |_|  |_|___/_| |_| |_|  \\___/|____/ ");
    crate::println!();
    crate::println!("  Beautiful, secure OS in Rust - Phase 1 (UEFI + serial shell)");
    crate::println!("  Type 'help' to list commands.");
}
