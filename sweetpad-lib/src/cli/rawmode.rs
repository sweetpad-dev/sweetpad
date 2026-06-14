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
//! log stream still renders as a clean newline. Unix-only; the CLI targets macOS.

use std::io::{self, Read};

/// RAII guard that puts stdin into single-key mode and restores the original
/// terminal settings on drop (including panics and early returns).
pub struct RawMode {
    original: libc::termios,
}

impl RawMode {
    /// Enable single-key input on stdin. Fails when stdin isn't a terminal, so
    /// callers gate the interactive session on a TTY and fall back otherwise.
    pub fn enable() -> io::Result<Self> {
        // Safety: tcgetattr/tcsetattr take a fd and a termios we own; we read
        // the current settings, stash them, and write back a tweaked copy.
        unsafe {
            let mut term: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut term) != 0 {
                return Err(io::Error::last_os_error());
            }
            let original = term;
            term.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN);
            term.c_cc[libc::VMIN] = 1; // block until at least one byte
            term.c_cc[libc::VTIME] = 0; // …with no inter-byte timeout
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &term) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { original })
        }
    }

    /// Block for the next keystroke. Returns `None` on EOF (Ctrl-D closes the
    /// stream) or a read error, which callers treat as "quit".
    pub fn read_key(&self) -> Option<u8> {
        let mut buf = [0u8; 1];
        match io::stdin().read(&mut buf) {
            Ok(1) => Some(buf[0]),
            _ => None,
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // Safety: restoring the exact termios we captured in `enable`.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
    }
}
