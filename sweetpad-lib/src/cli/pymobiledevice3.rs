//! Streaming a physical iOS device's os_log via the external `pymobiledevice3`
//! tool. The host `log stream` can't target a device and the `devicectl` console
//! carries only the app's stdout/stderr, so device `os_log`/`Logger` output comes
//! from `pymobiledevice3 syslog live`. Mirrors the VS Code extension's `Pymd3Sidecar`.

use std::process::Child;

use crate::cli::{CliError, process};

/// The tool's binary name — assumed on `PATH`.
pub const BINARY: &str = "pymobiledevice3";

/// One parsed `pymobiledevice3 syslog live --label` entry. Fields borrow from the
/// source line, so an entry lives only as long as that line.
pub struct SyslogEntry<'a> {
    /// Raw Apple timestamp (`YYYY-MM-DD HH:MM:SS.ffffff`), clocked by the renderer.
    pub timestamp: &'a str,
    /// The Mach-O that emitted the entry — the app's executable or its `.debug.dylib`
    /// in Debug builds, or a framework (e.g. `CoreFoundation`). Used to keep app-code
    /// logs and drop framework chatter running inside the app's process.
    pub image: &'a str,
    /// os_log severity name (`Debug`/`Info`/`Notice`/`Error`/`Fault`/`Default`),
    /// mapped from the syslog `<LEVEL>` so [`oslog::render_fields`] renders it like
    /// the simulator/macOS streams.
    ///
    /// [`oslog::render_fields`]: crate::cli::oslog::render_fields
    pub level: &'static str,
    pub category: &'a str,
    pub message: &'a str,
}

/// Whether `pymobiledevice3` is on `PATH` (probed via `version`).
#[must_use]
pub fn is_available() -> bool {
    process::capture(BINARY, &["version"], None).is_ok()
}

/// Spawn `pymobiledevice3 syslog live --label --process-name <exe>` with stdout
/// piped for the renderer. `--no-color` (a top-level option, before the subcommand)
/// keeps the parser's input clean; `--label` appends `[subsystem][category]`.
pub fn spawn(exe: &str) -> Result<Child, CliError> {
    process::spawn_piped(
        BINARY,
        &[
            "--no-color",
            "syslog",
            "live",
            "--label",
            "--process-name",
            exe,
        ],
        None,
    )
}

/// Map a syslog `<LEVEL>` to the os_log severity name [`oslog::render_fields`]
/// expects. `USER_ACTION` (an interaction event) folds to `Notice`; an unknown
/// level becomes `Default` (rendered as `Notice`).
///
/// [`oslog::render_fields`]: crate::cli::oslog::render_fields
fn map_level(level: &str) -> &'static str {
    match level {
        "DEBUG" => "Debug",
        "INFO" => "Info",
        "NOTICE" | "USER_ACTION" => "Notice",
        "ERROR" => "Error",
        "FAULT" => "Fault",
        _ => "Default",
    }
}

/// Parse one `syslog live --label` line, e.g.
/// `2026-04-16 12:52:32.707 App{App.debug.dylib+0x1a}[123] <NOTICE>: msg [sub][cat]`.
/// `None` for banners / partial lines that don't match the shape, so they're dropped.
#[must_use]
pub fn parse_line(line: &str) -> Option<SyslogEntry<'_>> {
    // `<timestamp> <process>{<image>[+0x<offset>]}[<pid>] <<level>>: <message>`.
    let (head, after_brace) = line.split_once('{')?;
    // The timestamp is `head`'s first two whitespace tokens (date + time); the
    // process name (which may contain spaces) follows but isn't needed.
    let mut tokens = head.splitn(3, ' ');
    let date = tokens.next()?;
    let time = tokens.next()?;
    // A real entry starts with a `YYYY-MM-DD` date — guards against stray `{` lines.
    if date.len() != 10 || !date.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let timestamp = head.get(..date.len() + 1 + time.len())?;

    let (image_field, after_image) = after_brace.split_once('}')?;
    // Drop the `+0x<offset>` load address when present.
    let image = image_field
        .split_once("+0x")
        .map_or(image_field, |(name, _)| name);

    let (_, after_lt) = after_image.split_once('<')?;
    let (level, rest) = after_lt.split_once('>')?;
    let body = rest.strip_prefix(": ")?;
    let (message, category) = split_label(body, image);

    Some(SyslogEntry {
        timestamp,
        image,
        level: map_level(level),
        category,
        message,
    })
}

/// Split a `--label` body into its message and category, peeling the trailing
/// ` [subsystem][category]` suffix. Falls back to the `image` name as the category
/// when there's no label (matching the entry's emitting binary), and ignores a
/// message that merely ends in `]` so a stray `[…]` payload isn't mistaken for one.
fn split_label<'a>(body: &'a str, image: &'a str) -> (&'a str, &'a str) {
    if let Some(without_close) = body.strip_suffix(']')
        && let Some((before_category, category)) = without_close.rsplit_once('[')
        && let Some(before_subsystem_close) = before_category.strip_suffix(']')
        && let Some((message, _subsystem)) = before_subsystem_close.rsplit_once(" [")
    {
        return (message, category);
    }
    (body, image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_labelled_entry_and_strips_the_load_address() {
        let line =
            "2026-04-16 12:52:32.707333 Lab{Lab.debug.dylib+0x1a2b}[67135] <NOTICE>: hello [com.x.app][net]";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.timestamp, "2026-04-16 12:52:32.707333");
        assert_eq!(entry.image, "Lab.debug.dylib");
        assert_eq!(entry.level, "Notice");
        assert_eq!(entry.category, "net");
        assert_eq!(entry.message, "hello");
    }

    #[test]
    fn falls_back_to_the_image_when_unlabelled() {
        let line = "2026-04-16 12:52:32.707333 Lab{Lab+0x1}[67135] <ERROR>: boom";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.image, "Lab");
        assert_eq!(entry.level, "Error");
        assert_eq!(entry.category, "Lab");
        assert_eq!(entry.message, "boom");
    }

    #[test]
    fn a_message_ending_in_a_bracket_is_not_a_label() {
        let line = "2026-04-16 12:52:32.707333 Lab{Lab}[1] <INFO>: values [1, 2]";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.message, "values [1, 2]");
        assert_eq!(entry.category, "Lab");
    }

    #[test]
    fn process_names_with_spaces_keep_the_timestamp() {
        let line = "2026-04-16 12:52:32.707333 Control Room{Control Room+0x1}[1] <INFO>: hi";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.timestamp, "2026-04-16 12:52:32.707333");
        assert_eq!(entry.image, "Control Room");
        assert_eq!(entry.message, "hi");
    }

    #[test]
    fn banners_and_malformed_lines_are_dropped() {
        assert!(parse_line("Filtering the log data").is_none());
        assert!(parse_line("").is_none());
        // Has a `{` but no leading date.
        assert!(parse_line("note{x}[1] <INFO>: hi").is_none());
    }

    #[test]
    fn maps_syslog_levels_to_os_log_names() {
        assert_eq!(map_level("DEBUG"), "Debug");
        assert_eq!(map_level("USER_ACTION"), "Notice");
        assert_eq!(map_level("FAULT"), "Fault");
        assert_eq!(map_level("WAT"), "Default");
    }
}
