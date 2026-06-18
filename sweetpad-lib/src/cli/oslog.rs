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

/// Render one ndjson line as a colored log line. Non-JSON input (the stream's
/// banner, or anything unexpected) is shown as a blue `system` note.
#[must_use]
pub fn render_ndjson_line(line: &str, color: bool) -> String {
    match serde_json::from_str::<Entry>(line) {
        Ok(entry) => {
            let msg_type = entry.message_type.as_deref().unwrap_or("Default");
            let time = entry.timestamp.as_deref().and_then(clock_time);
            let category = entry.category.as_deref().unwrap_or("?");
            let message = entry.event_message.as_deref().unwrap_or("");
            format_line(
                time.as_deref(),
                level_letter(msg_type),
                category,
                message,
                level_color(msg_type),
                color,
            )
        }
        // Banner / non-JSON: a blue `N [system]` note carrying the raw line.
        Err(_) => format_line(None, "N", "system", line, 34, color),
    }
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
        // No color: plain "HH:MM:SS.sss L [cat] msg".
        assert_eq!(
            render_ndjson_line(line, false),
            "23:59:59.123 I [networking] Request started"
        );
        // Color: bold + cyan (36) prefix, reset before the (uncolored) message.
        assert_eq!(
            render_ndjson_line(line, true),
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
    fn non_json_lines_become_a_system_note() {
        let line = "Filtering the log data using \"process == ...\"";
        assert_eq!(render_ndjson_line(line, false), format!("N [system] {line}"));
        assert_eq!(
            render_ndjson_line(line, true),
            format!("\x1b[1;34mN [system]\x1b[0m {line}")
        );
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
        assert_eq!(render_ndjson_line(line, false), "E [db] boom");
    }

    #[test]
    fn strip_ansi_removes_sgr_escapes() {
        // `\x1b` here is a real ESC byte in the Rust &str (not JSON).
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m text"), "red text");
        // Untouched input is borrowed, not reallocated.
        assert!(matches!(strip_ansi("plain"), Cow::Borrowed("plain")));
    }
}
