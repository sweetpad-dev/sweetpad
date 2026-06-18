//! A transient spinner for long, otherwise-silent steps (booting a simulator,
//! installing, …).
//!
//! While a step runs, a background thread animates `⠋ message` on stderr,
//! redrawing in place with `\r`. When the step ends the line is erased — so the
//! caller's following status line takes its place, and stdout stays clean for
//! data/logs. Inert when stderr isn't an interactive TTY (piped, CI, `--json`),
//! where the step's closure still runs and the caller's static notes print.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TICK: Duration = Duration::from_millis(80);

/// A running spinner. Dropping it stops the animation and erases the line.
pub struct Spinner {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Spinner {
    /// Start a spinner showing `message`. When `active` is false (non-TTY/JSON),
    /// returns an inert handle that animates nothing and erases nothing.
    #[must_use]
    pub fn start(message: &str, active: bool, color: bool) -> Self {
        if !active {
            return Self {
                stop: Arc::new(AtomicBool::new(true)),
                handle: None,
            };
        }
        let stop = Arc::new(AtomicBool::new(false));
        let message = message.to_string();
        let handle = thread::spawn({
            let stop = Arc::clone(&stop);
            move || animate(&message, &stop, color)
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Redraw `⠋ message` in place each tick until told to stop, then erase the line.
fn animate(message: &str, stop: &AtomicBool, color: bool) {
    let mut err = std::io::stderr();
    let mut i = 0;
    while !stop.load(Ordering::Relaxed) {
        let frame = FRAMES[i % FRAMES.len()];
        // `\r` to column 0, then `\x1b[K` to clear any leftover from a longer
        // earlier frame/message.
        let _ = if color {
            write!(err, "\r\x1b[36m{frame}\x1b[0m {message}\x1b[K")
        } else {
            write!(err, "\r{frame} {message}\x1b[K")
        };
        let _ = err.flush();
        i += 1;
        thread::sleep(TICK);
    }
    let _ = write!(err, "\r\x1b[K");
    let _ = err.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inert_spinner_spawns_no_thread_and_drops_cleanly() {
        // The non-TTY/JSON path: no animation thread, nothing written, no hang.
        let spinner = Spinner::start("booting simulator", false, true);
        assert!(spinner.handle.is_none());
        drop(spinner);
    }
}
