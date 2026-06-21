//! Minimal raw-terminal input for the `app run` interactive rebuild session.
//!
//! We don't want a full TUI: the app's logs keep streaming to the terminal and
//! we only need to read single keystrokes (`r` to rebuild, `q`/Ctrl-C to quit)
//! without waiting for Enter. So rather than `cfmakeraw` (which clears `OPOST`
//! and would stair-step the streamed log lines), we flip *only* the input
//! line-discipline flags by hand:
//!
//! - clear `ICANON` so reads return per-keystroke instead of per-line,
//! - clear `ECHO` so keys don't print into the log stream,
//! - clear `ISIG` so Ctrl-C arrives as a byte (`0x03`) we handle ourselves —
//!   this is what lets [`RawMode`]'s `Drop` always restore the terminal, where
//!   a delivered `SIGINT` would have killed us mid-session and left it cooked.
//!
//! Output post-processing (`OPOST`/`ONLCR`) is left untouched, so `\n` from the
//! log stream still renders as a clean newline. Reads are non-blocking (a 0.1s
//! `VTIME` poll) so the session loop — and the watcher thread during a build —
//! can stay responsive instead of parking forever on a keypress. Unix-only; the
//! CLI targets macOS.

use std::io;

/// RAII guard that puts stdin into single-key poll mode and restores the
/// original terminal settings on drop (including panics and early returns).
pub struct RawMode {
    original: libc::termios,
}

impl RawMode {
    /// Enable single-key polling input on stdin. Fails when stdin isn't a
    /// terminal, so callers gate the interactive session on a TTY and fall back
    /// otherwise.
    pub fn enable() -> io::Result<Self> {
        // Safety: tcgetattr/tcsetattr take a fd and a termios we own; we read
        // the current settings, stash them, and write back a tweaked copy.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &raw mut term) != 0 {
                return Err(io::Error::last_os_error());
            }
            let original = term;
            term.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            // VMIN=0, VTIME=1: return after one byte *or* ~0.1s with nothing, so
            // a read never blocks the loop indefinitely (see [`poll_key`]).
            term.c_cc[libc::VMIN] = 0;
            term.c_cc[libc::VTIME] = 1;
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const term) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { original })
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Safety: restoring the exact termios we captured in `enable`.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const self.original);
        }
    }
}

/// The result of one [`poll_key`] tick.
pub enum Input {
    /// A key was pressed, decoded as a whole UTF-8 character (one byte for ASCII
    /// and control keys, more for e.g. a Cyrillic key — so the session can map
    /// non-Latin layouts back to their Latin shortcut).
    Key(char),
    /// Nothing was pressed this tick (the `VTIME` timeout elapsed).
    Idle,
    /// stdin read errored — treat as "quit".
    Closed,
}

/// Poll stdin for a single keystroke, returning within ~0.1s even if nothing is
/// pressed. Reads the raw fd directly (not std's buffered `Stdin`) so concurrent
/// users — the idle loop and the build-time watcher — never fight over a shared
/// buffer. A multi-byte UTF-8 keystroke (e.g. a Cyrillic letter) is read whole.
/// Only valid while a [`RawMode`] guard is active. On a terminal a zero-byte read
/// is the `VTIME` timeout (EOF arrives as a `^D` byte), so it maps to
/// [`Input::Idle`], not a close.
#[must_use]
pub fn poll_key() -> Input {
    // Wait up to ~0.1s for stdin to be readable *before* reading, so the read never
    // blocks when the caller hasn't enabled raw mode (where VMIN/VTIME would bound
    // it). The build's Ctrl-C watcher runs without raw mode on the interactive
    // `--hot` path; a bare blocking read there wedges its loop in canonical mode and
    // hangs the join after the build. Nothing readable this tick → [`Input::Idle`].
    if !readable(STDIN_POLL_MS) {
        return Input::Idle;
    }
    let mut buf = [0u8; 1];
    // Safety: reading one byte into a stack buffer we own.
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), 1) };
    match n {
        1 => decode_char(buf[0]),
        0 => Input::Idle,
        _ => Input::Closed,
    }
}

/// The per-tick poll budget, matching the old `VTIME` cadence so the session loop
/// stays responsive without busy-spinning when stdin is a quiet terminal/pipe.
const STDIN_POLL_MS: i32 = 100;

/// Whether stdin has data within `timeout_ms`, via `poll(2)` — independent of the
/// terminal's line discipline, so the subsequent read won't block on a cooked TTY
/// or an open-but-idle pipe.
fn readable(timeout_ms: i32) -> bool {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // Safety: poll over a single pollfd we own; revents is filled on return.
    let n = unsafe { libc::poll(&raw mut fd, 1, timeout_ms) };
    n > 0 && (fd.revents & libc::POLLIN) != 0
}

/// Decode a full UTF-8 character from its first byte, reading any continuation
/// bytes (the bytes of one keystroke arrive together, so the follow-up reads
/// return promptly). ASCII and control bytes (e.g. Ctrl-C `0x03`) are a single
/// byte; a Cyrillic key is two. A truncated or invalid sequence is [`Input::Idle`].
fn decode_char(first: u8) -> Input {
    let mut buf = [0u8; 4];
    buf[0] = first;
    let mut filled = 1;
    while filled < utf8_len(first) {
        let mut b = [0u8; 1];
        // Safety: one continuation byte into a stack buffer we own.
        let n = unsafe { libc::read(libc::STDIN_FILENO, b.as_mut_ptr().cast(), 1) };
        if n == 1 {
            buf[filled] = b[0];
            filled += 1;
        } else {
            break;
        }
    }
    match std::str::from_utf8(&buf[..filled])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(c) => Input::Key(c),
        None => Input::Idle,
    }
}

/// Expected UTF-8 length from a leading byte. Continuation/invalid bytes report
/// 1, so they decode as themselves (ASCII/control) or fail the decode harmlessly.
fn utf8_len(first: u8) -> usize {
    match first {
        0xF0..=0xF7 => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}
