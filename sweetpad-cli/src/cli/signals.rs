//! Process-wide SIGINT/SIGTERM handling for the CLI half.
//!
//! Without a handler, a signal during a non-raw-mode stretch (a plain build,
//! the `--hot` initial build, a log follow) kills the CLI with the default
//! semantics: a live spinner line is left on stderr, a raw-mode terminal stays
//! no-echo, an xcodebuild spawned into its own process group keeps building
//! detached, and log-stream children leak. The handler installed here performs
//! the `Drop`-equivalent cleanup using only async-signal-safe calls —
//! `write(2)`, `tcsetattr`, `kill(2)`, `_exit` — then exits `128 + signo`
//! (130 for Ctrl-C, the shell convention).
//!
//! The interactive `app run` session disables `ISIG`, so Ctrl-C there arrives
//! as a key byte and never reaches this handler; it exists for everything
//! outside raw mode, and for SIGTERM at any time. One child reap is out of
//! reach: `simctl spawn … log stream` reparents its `log` process to
//! `launchd_sim`, and reaping it needs a `pkill` no signal handler can spawn —
//! that one still relies on the normal `Drop` path (or the simulator shutting
//! down).

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

/// Whether stderr was a TTY at startup — read in the handler (where `isatty`
/// isn't guaranteed safe) to decide if a spinner line may need erasing.
static STDERR_TTY: AtomicBool = AtomicBool::new(false);

/// The process group of the currently-running interruptible build (0 = none).
/// The handler forwards the signal here so an `xcodebuild` spawned into its own
/// group can't keep building detached after the CLI dies.
static BUILD_PGID: AtomicU32 = AtomicU32::new(0);

/// Long-running child processes (log streams, device consoles) to SIGTERM on
/// the way out. A small fixed pool: registration is best-effort — a session has
/// at most a handful of these at once.
static CHILD_PIDS: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

/// Whether stdin is currently in raw (no-echo) mode, plus the termios to
/// restore. The pointer is written once (leaked box) and updated in place.
static RAW_ACTIVE: AtomicBool = AtomicBool::new(false);
static RAW_TERMIOS: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());

/// Install the SIGINT/SIGTERM handler. Called once at CLI startup.
#[allow(clippy::fn_to_numeric_cast_any)] // sighandler_t is how signal(2) takes a handler
pub fn install() {
    let handler = handle as extern "C" fn(libc::c_int);
    // Safety: isatty on a constant fd; signal() installs an async-signal-safe
    // handler before any work spawns children or flips terminal modes.
    unsafe {
        STDERR_TTY.store(libc::isatty(libc::STDERR_FILENO) == 1, Ordering::Relaxed);
        libc::signal(libc::SIGINT, handler as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handler as libc::sighandler_t);
    }
}

/// Record the interruptible build's process group so the handler forwards
/// signals to it. Cleared with [`clear_build_pgid`] once the build is waited.
pub fn set_build_pgid(pgid: u32) {
    BUILD_PGID.store(pgid, Ordering::Release);
}

pub fn clear_build_pgid() {
    BUILD_PGID.store(0, Ordering::Release);
}

/// Register a long-running child for SIGTERM-on-exit. Returns the slot to pass
/// to [`unregister_child`]; `None` when the pool is full (best-effort).
pub fn register_child(pid: u32) -> Option<usize> {
    for (i, slot) in CHILD_PIDS.iter().enumerate() {
        if slot
            .compare_exchange(0, pid, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return Some(i);
        }
    }
    None
}

pub fn unregister_child(slot: Option<usize>) {
    if let Some(i) = slot {
        CHILD_PIDS[i].store(0, Ordering::Release);
    }
}

/// Record the original termios to restore if a signal lands while raw mode is
/// active. Called by [`crate::cli::rawmode::RawMode::enable`].
pub fn set_raw(original: libc::termios) {
    let ptr = RAW_TERMIOS.load(Ordering::Acquire);
    if ptr.is_null() {
        RAW_TERMIOS.store(Box::into_raw(Box::new(original)), Ordering::Release);
    } else {
        // Safety: the pointer is a leaked box only this (main-thread) writer
        // updates; the handler only reads it.
        unsafe { *ptr = original };
    }
    RAW_ACTIVE.store(true, Ordering::Release);
}

pub fn clear_raw() {
    RAW_ACTIVE.store(false, Ordering::Release);
}

/// Erase-the-spinner-line escape, written to a TTY stderr on the way out.
const CLEAR_LINE: &[u8] = b"\r\x1b[K";

/// The handler: erase any in-flight spinner line, restore the terminal,
/// forward the signal to the build's process group, SIGTERM registered
/// children, and exit `128 + signo`. Async-signal-safe calls only.
extern "C" fn handle(sig: libc::c_int) {
    if STDERR_TTY.load(Ordering::Relaxed) {
        // Safety: write(2) of a static buffer to stderr.
        unsafe {
            libc::write(
                libc::STDERR_FILENO,
                CLEAR_LINE.as_ptr().cast(),
                CLEAR_LINE.len(),
            );
        }
    }
    if RAW_ACTIVE.load(Ordering::Acquire) {
        let ptr = RAW_TERMIOS.load(Ordering::Acquire);
        if !ptr.is_null() {
            // Safety: restoring a termios the main thread stashed.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, ptr);
            }
        }
    }
    let build_pgid = BUILD_PGID.load(Ordering::Acquire);
    if build_pgid != 0 {
        // Safety: kill(2) on a negative pgid signals the whole build tree.
        unsafe {
            libc::kill(-build_pgid.cast_signed(), sig);
        }
    }
    for slot in &CHILD_PIDS {
        let pid = slot.swap(0, Ordering::AcqRel);
        if pid != 0 {
            // Safety: kill(2) on a recorded child pid; a stale pid is harmless.
            unsafe {
                libc::kill(pid.cast_signed(), libc::SIGTERM);
            }
        }
    }
    // Safety: _exit(2) is the async-signal-safe process exit.
    unsafe {
        libc::_exit(128 + sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_registry_hands_out_and_reclaims_slots() {
        let a = register_child(111).expect("slot");
        let b = register_child(222).expect("slot");
        assert_ne!(a, b);
        unregister_child(Some(a));
        // The freed slot is reusable.
        let c = register_child(333).expect("slot");
        assert_eq!(c, a);
        unregister_child(Some(b));
        unregister_child(Some(c));
        unregister_child(None); // no-op
    }

    #[test]
    fn build_pgid_set_and_clear() {
        set_build_pgid(4242);
        assert_eq!(BUILD_PGID.load(Ordering::Acquire), 4242);
        clear_build_pgid();
        assert_eq!(BUILD_PGID.load(Ordering::Acquire), 0);
    }
}
