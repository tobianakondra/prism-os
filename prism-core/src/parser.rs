//! PrismOS shell command parser (pure, no I/O).
//!
//! ROLE:
//! Turns a raw input line into a `ShellCommand`. Parsing and executing are
//! DELIBERATELY separated: parsing is pure and host-tested here, while
//! execution (which prints to serial and reads kernel state) lives in the
//! kernel's `shell` module. A reviewer can audit the full command grammar
//! in this one file without touching hardware.
//!
//! GRAMMAR (Phase 1):
//!   help | mem | tasks | uptime | clear | banner | echo <rest...> | <unknown>

/// Maximum command line length (bytes). Shared with the kernel line editor
/// so both sides agree; fits `echo <text>` demos while staying stack-tiny.
pub const MAX_LINE_LEN: usize = 256;

/// Commands the shell understands.
#[derive(Debug, PartialEq, Eq)]
pub enum ShellCommand<'a> {
    Help,
    Mem,
    Tasks,
    Uptime,
    Echo(&'a str),
    Clear,
    Banner,
    Unknown(&'a str),
}

/// Parse a raw input line into a command. Pure: no hardware, no globals.
pub fn parse_command(line: &str) -> ShellCommand<'_> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return ShellCommand::Unknown("");
    }
    // Split into first word + rest so `echo hello world` keeps its argument.
    let (word, rest) = match trimmed.find(char::is_whitespace) {
        Some(i) => (&trimmed[..i], trimmed[i..].trim()),
        None => (trimmed, ""),
    };
    match word {
        "help" => ShellCommand::Help,
        "mem" => ShellCommand::Mem,
        "tasks" => ShellCommand::Tasks,
        "uptime" => ShellCommand::Uptime,
        "clear" => ShellCommand::Clear,
        "banner" => ShellCommand::Banner,
        "echo" => ShellCommand::Echo(rest),
        other => ShellCommand::Unknown(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_known_commands() {
        assert_eq!(parse_command("help"), ShellCommand::Help);
        assert_eq!(parse_command("  mem  "), ShellCommand::Mem);
        assert_eq!(parse_command("tasks"), ShellCommand::Tasks);
        assert_eq!(parse_command("uptime"), ShellCommand::Uptime);
        assert_eq!(parse_command("clear"), ShellCommand::Clear);
        assert_eq!(parse_command("banner"), ShellCommand::Banner);
    }

    #[test]
    fn echo_keeps_full_argument() {
        assert_eq!(
            parse_command("echo hello world"),
            ShellCommand::Echo("hello world")
        );
        // Bare `echo` echoes the empty string (prints a blank line).
        assert_eq!(parse_command("echo"), ShellCommand::Echo(""));
    }

    #[test]
    fn unknown_command_preserves_word() {
        assert_eq!(parse_command("foobar"), ShellCommand::Unknown("foobar"));
        // Extra args do not change the unknown word.
        assert_eq!(
            parse_command("foobar --help"),
            ShellCommand::Unknown("foobar")
        );
    }

    #[test]
    fn empty_line_is_empty_unknown() {
        assert_eq!(parse_command(""), ShellCommand::Unknown(""));
        assert_eq!(parse_command("   "), ShellCommand::Unknown(""));
    }
}
