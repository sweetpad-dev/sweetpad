//! Formatting for streamed `os_log` output (`simctl spawn … log stream
//! --style ndjson`).
//!
//! Mirrors the VS Code extension's renderer (`src/run/utils.ts`): each ndjson
//! entry becomes a bold, color-coded `HH:MM:SS.sss L [category] message` line —
//! the level as a single letter (D/I/N/E/F), the prefix tinted by severity, the
//! message left in the terminal's default color. Lines that aren't JSON (the
//! `Filtering the log data …` banner the stream prints first, say) are shown as
//! a blue `system` note carrying the raw text.

use std::borrow::Cow;

use serde::Deserialize;

/// One ndjson entry from `log stream --style ndjson`. Unknown fields are ignored.
#[derive(Deserialize)]
struct Entry {
    timestamp: Option<String>,
    #[serde(rename = "messageType")]
    message_type: Option<String>,
    category: Option<String>,
    #[serde(rename = "eventMessage")]
    event_message: Option<String>,
}

/// os_log severity, ordered low→high so a live filter can compare against a
/// threshold. `Default`/`Notice` collapse to `Notice`; an unknown `messageType`
/// is treated as `Notice`.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Notice = 2,
    Error = 3,
    Fault = 4,
}

impl Level {
    fn from_message_type(message_type: &str) -> Self {
        match message_type {
            "Debug" => Level::Debug,
            "Info" => Level::Info,
            "Error" => Level::Error,
            "Fault" => Level::Fault,
            // "Default"/"Notice" and the unknown fallback.
            _ => Level::Notice,
        }
    }

    /// The level as a `u8`, for storing the live filter threshold in an atomic and
    /// comparing rendered lines against it.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Single-letter severity tag. `Default` and `Notice` are indistinguishable at
/// the os_log layer, so both render as `N`; anything unrecognized is `?`.
fn level_letter(message_type: &str) -> &'static str {
    match message_type {
        "Debug" => "D",
        "Info" => "I",
        "Default" | "Notice" => "N",
        "Error" => "E",
        "Fault" => "F",
        _ => "?",
    }
}

/// ANSI color code for a severity (90 = bright black / gray, also the fallback).
fn level_color(message_type: &str) -> u8 {
    match message_type {
        "Info" => 36,               // cyan
        "Default" | "Notice" => 34, // blue
        "Error" => 31,              // red
        "Fault" => 35,              // magenta
        // "Debug" and the unknown fallback both render gray.
        _ => 90,
    }
}

/// A rendered log line plus its severity, so callers can filter by [`Level`]
/// against a live threshold before printing.
pub struct Line {
    pub level: Level,
    pub text: String,
}

/// Render one ndjson line as a colored log line with its severity. Non-JSON input
/// (the stream's banner, or anything unexpected) is shown as a blue `system` note
/// at `Notice` level.
#[must_use]
pub fn render_ndjson_line(line: &str, color: bool) -> Line {
    match serde_json::from_str::<Entry>(line) {
        Ok(entry) => render_fields(
            entry.timestamp.as_deref(),
            entry.message_type.as_deref().unwrap_or("Default"),
            entry.category.as_deref().unwrap_or("?"),
            entry.event_message.as_deref().unwrap_or(""),
            color,
        ),
        // Banner / non-JSON: a blue `N [system]` note carrying the raw line.
        Err(_) => render_fields(None, "Default", "system", line, color),
    }
}

/// Render already-parsed log fields into a [`Line`], shared by [`render_ndjson_line`]
/// and the device syslog renderer so both produce identical
/// `HH:MM:SS.sss L [category] message` output. `timestamp` is a raw Apple timestamp
/// (clocked here, or dropped if it doesn't parse); `message_type` is the os_log
/// severity name (`Debug`/`Info`/`Notice`/`Default`/`Error`/`Fault`).
#[must_use]
pub fn render_fields(
    timestamp: Option<&str>,
    message_type: &str,
    category: &str,
    message: &str,
    color: bool,
) -> Line {
    let time = timestamp.and_then(clock_time);
    render_clocked(time.as_deref(), message_type, category, message, color)
}

/// Assemble a [`Line`] from an already-formatted `HH:MM:SS.sss` clock string (or
/// `None`). Shared by [`render_fields`] — which parses the clock out of a raw Apple
/// timestamp — and [`render_console_line`], which is handed a wall-clock stamp.
fn render_clocked(
    time: Option<&str>,
    message_type: &str,
    category: &str,
    message: &str,
    color: bool,
) -> Line {
    Line {
        level: Level::from_message_type(message_type),
        text: format_line(
            time,
            level_letter(message_type),
            category,
            message,
            level_color(message_type),
            color,
        ),
    }
}

/// Render one line of an app's own stdout/stderr — its direct console output
/// (`print()`, etc.), as opposed to os_log — as a blue `N [print]` note at `Notice`
/// level, the analog of the VS Code extension's `print` lines. `time` is the local
/// wall-clock stamp for when the line was read (see [`now_clock`]): console output
/// carries no timestamp of its own, so the reader supplies one, aligning these lines
/// with the `HH:MM:SS.sss` os_log lines. The app owns the text, so it's shown
/// verbatim (its ANSI is stripped by [`format_line`], whose prefix coloring replaces it).
#[must_use]
pub fn render_console_line(time: Option<&str>, line: &str, color: bool) -> Line {
    render_clocked(time, "Default", "print", line, color)
}

/// Assemble `HH:MM:SS.sss L [cat] message`, the prefix in bold + `code` when
/// color is on. A missing time drops just that token. The message's own ANSI is
/// stripped — os_log payloads are plain text, and the prefix owns the coloring.
fn format_line(
    time: Option<&str>,
    letter: &str,
    category: &str,
    message: &str,
    code: u8,
    color: bool,
) -> String {
    let prefix = match time {
        Some(t) => format!("{t} {letter} [{category}]"),
        None => format!("{letter} [{category}]"),
    };
    let message = strip_ansi(message);
    if color {
        format!("\x1b[1;{code}m{prefix}\x1b[0m {message}")
    } else {
        format!("{prefix} {message}")
    }
}

/// Extract `HH:MM:SS.sss` from an Apple timestamp like
/// `2024-12-31 23:59:59.000000-0800`. Returns `None` if the shape doesn't match,
/// so the caller falls back to a time-less prefix.
fn clock_time(timestamp: &str) -> Option<String> {
    // The clock portion follows the date: "HH:MM:SS.ffffff±zzzz".
    let (hms, frac) = timestamp.split(' ').nth(1)?.split_once('.')?;
    let mut parts = hms.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    let two_digits = |x: &str| x.len() == 2 && x.bytes().all(|b| b.is_ascii_digit());
    if parts.next().is_some() || !(two_digits(h) && two_digits(m) && two_digits(s)) {
        return None;
    }
    // First three fractional digits (milliseconds); right-pad if ever shorter.
    let millis: String = frac
        .bytes()
        .take_while(u8::is_ascii_digit)
        .take(3)
        .map(char::from)
        .collect();
    if millis.is_empty() {
        return None;
    }
    Some(format!("{h}:{m}:{s}.{millis:0<3}"))
}

/// The current local wall-clock time as `HH:MM:SS.sss` — the same shape as the
/// os_log [`clock_time`] stamps. App console output (`print`/stderr) carries no
/// timestamp of its own, so [`render_console_line`] stamps each line with the moment
/// it's read.
#[must_use]
pub fn now_clock() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (h, m, s) = local_hms(now.as_secs());
    format_clock(h, m, s, now.subsec_millis())
}

/// Split an epoch-second count into local `(hour, minute, second)` via
/// `localtime_r`, so the stamp honors the machine's timezone the way the
/// simulator's os_log timestamps already do.
fn local_hms(epoch_secs: u64) -> (u32, u32, u32) {
    let t: libc::time_t = i64::try_from(epoch_secs).unwrap_or(i64::MAX);
    // SAFETY: `localtime_r` fills the caller-owned `tm` from `t` — the reentrant,
    // thread-safe form (console lines render on detached threads). We read only the
    // always-in-range clock fields, so `unsigned_abs` can't lose information.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&raw const t, &raw mut tm);
    }
    (
        tm.tm_hour.unsigned_abs(),
        tm.tm_min.unsigned_abs(),
        tm.tm_sec.unsigned_abs(),
    )
}

/// `HH:MM:SS.sss`, zero-padded — the os_log clock format produced by [`clock_time`].
fn format_clock(hour: u32, min: u32, sec: u32, millis: u32) -> String {
    format!("{hour:02}:{min:02}:{sec:02}.{millis:03}")
}

/// Strip SGR color escapes (`ESC[…m`) from a message. Operates on bytes — SGR
/// sequences are ASCII, and every other byte is copied verbatim, so the UTF-8
/// stays valid. Borrows untouched input.
fn strip_ansi(s: &str) -> Cow<'_, str> {
    if !s.contains('\x1b') {
        return Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0x1b && b.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                j += 1;
            }
            if b.get(j) == Some(&b'm') {
                i = j + 1; // drop the whole `ESC[…m` sequence
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    Cow::Owned(String::from_utf8(out).unwrap_or_else(|_| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_an_ndjson_entry_with_level_letter_and_category() {
        let line = r#"{"timestamp":"2024-12-31 23:59:59.123456-0800","messageType":"Info","category":"networking","eventMessage":"Request started"}"#;
        let plain = render_ndjson_line(line, false);
        assert_eq!(plain.level, Level::Info);
        // No color: plain "HH:MM:SS.sss L [cat] msg".
        assert_eq!(plain.text, "23:59:59.123 I [networking] Request started");
        // Color: bold + cyan (36) prefix, reset before the (uncolored) message.
        assert_eq!(
            render_ndjson_line(line, true).text,
            "\x1b[1;36m23:59:59.123 I [networking]\x1b[0m Request started"
        );
    }

    #[test]
    fn maps_each_level_to_its_letter_and_color() {
        assert_eq!((level_letter("Debug"), level_color("Debug")), ("D", 90));
        assert_eq!((level_letter("Info"), level_color("Info")), ("I", 36));
        assert_eq!((level_letter("Default"), level_color("Default")), ("N", 34));
        assert_eq!((level_letter("Notice"), level_color("Notice")), ("N", 34));
        assert_eq!((level_letter("Error"), level_color("Error")), ("E", 31));
        assert_eq!((level_letter("Fault"), level_color("Fault")), ("F", 35));
        // Unknown messageType → "?" / gray.
        assert_eq!((level_letter("Weird"), level_color("Weird")), ("?", 90));
    }

    #[test]
    fn renders_app_console_output_as_a_print_note() {
        // No stamp: a bare "N [print] msg".
        let plain = render_console_line(None, "hello from print()", false);
        assert_eq!(plain.level, Level::Notice);
        assert_eq!(plain.text, "N [print] hello from print()");
        // With a local-time stamp it reads like the os_log lines: "HH:MM:SS.sss N [print] msg".
        assert_eq!(
            render_console_line(Some("15:02:13.053"), "hello from print()", false).text,
            "15:02:13.053 N [print] hello from print()"
        );
        // Color: bold + blue (34) prefix (the stamp included), message left uncolored.
        assert_eq!(
            render_console_line(Some("15:02:13.053"), "hello from print()", true).text,
            "\x1b[1;34m15:02:13.053 N [print]\x1b[0m hello from print()"
        );
    }

    #[test]
    fn format_clock_zero_pads_each_field() {
        assert_eq!(format_clock(15, 2, 13, 53), "15:02:13.053");
        assert_eq!(format_clock(0, 0, 0, 0), "00:00:00.000");
        assert_eq!(format_clock(23, 59, 59, 999), "23:59:59.999");
    }

    #[test]
    fn now_clock_matches_the_oslog_clock_shape() {
        // Same `HH:MM:SS.sss` shape as `clock_time`, so console and os_log lines align.
        let c = now_clock();
        assert_eq!(c.len(), 12);
        assert!(c.as_bytes().iter().enumerate().all(|(i, &b)| match i {
            2 | 5 => b == b':',
            8 => b == b'.',
            _ => b.is_ascii_digit(),
        }));
    }

    #[test]
    fn non_json_lines_become_a_system_note() {
        let line = "Filtering the log data using \"process == ...\"";
        let plain = render_ndjson_line(line, false);
        assert_eq!(plain.level, Level::Notice);
        assert_eq!(plain.text, format!("N [system] {line}"));
        assert_eq!(
            render_ndjson_line(line, true).text,
            format!("\x1b[1;34mN [system]\x1b[0m {line}")
        );
    }

    #[test]
    fn levels_order_low_to_high_for_filtering() {
        // The live filter compares `as_u8`, so the ordering must be monotonic.
        assert!(Level::Debug < Level::Info && Level::Info < Level::Error);
        assert!(Level::Debug.as_u8() < Level::Info.as_u8());
        assert!(Level::Info.as_u8() < Level::Error.as_u8());
    }

    #[test]
    fn extracts_the_clock_time_from_an_apple_timestamp() {
        assert_eq!(
            clock_time("2024-12-31 23:59:59.000000-0800").as_deref(),
            Some("23:59:59.000")
        );
        assert_eq!(
            clock_time("2024-12-31 08:05:01.5-0800").as_deref(),
            Some("08:05:01.500")
        );
        assert_eq!(clock_time("garbage"), None);
        assert_eq!(clock_time("2024-12-31 9:5:1.0"), None); // not zero-padded
    }

    #[test]
    fn missing_timestamp_drops_only_the_time_token() {
        let line = r#"{"messageType":"Error","category":"db","eventMessage":"boom"}"#;
        let rendered = render_ndjson_line(line, false);
        assert_eq!(rendered.level, Level::Error);
        assert_eq!(rendered.text, "E [db] boom");
    }

    #[test]
    fn strip_ansi_removes_sgr_escapes() {
        // `\x1b` here is a real ESC byte in the Rust &str (not JSON).
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m text"), "red text");
        // Untouched input is borrowed, not reallocated.
        assert!(matches!(strip_ansi("plain"), Cow::Borrowed("plain")));
    }
}
