//! Process-wide signal handling for the CLI half.
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
//! Covered signals: SIGINT and SIGTERM (interrupt/kill), SIGHUP (a closed
//! terminal must stop the build group, not detach it), SIGPIPE (a session
//! writing its log stream into a gone pipe must still restore the terminal),
//! and SIGTSTP/SIGCONT (suspending mid-session hands the shell a cooked
//! terminal and resuming re-asserts raw mode). Each of INT/TERM/HUP/TSTP
//! honors an inherited `SIG_IGN` — a background job a shell protected from
//! Ctrl-C stays protected. SIGPIPE is the exception: the Rust runtime sets it
//! to `SIG_IGN` before `main` (and `main` resets it to default), so a
//! parent's deliberate ignore-disposition is unobservable by the time this
//! runs — PIPE always gets the handler.
//!
//! The interactive `app run` session disables `ISIG`, so Ctrl-C there arrives
//! as a key byte and never reaches this handler; it exists for everything
//! outside raw mode, and for SIGTERM at any time. One child reap is out of
//! reach: `simctl spawn … log stream` reparents its `log` process to
//! `launchd_sim`, and reaping it needs a `pkill` no signal handler can spawn —
//! that one still relies on the normal `Drop` path (or the simulator shutting
//! down).
//!
//! Forward-only mode ([`set_forward_child`]): a command whose child must
//! *finalize* on interruption (`simctl io recordVideo`, an inline log follow)
//! registers the child pid; the handler then forwards SIGINT to that child and
//! **returns** instead of exiting, letting the main thread reap it and render
//! a real result. [`take_forwarded`] tells the command a signal was the reason
//! its child ended.

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

/// Whether stderr was a TTY at startup — read in the handler (where `isatty`
/// isn't guaranteed safe) to decide if a spinner line may need erasing.
static STDERR_TTY: AtomicBool = AtomicBool::new(false);

/// The process group of the currently-running interruptible build (0 = none).
/// The handler forwards the signal here so an `xcodebuild` spawned into its own
/// group can't keep building detached after the CLI dies.
static BUILD_PGID: AtomicU32 = AtomicU32::new(0);

/// Long-running child processes (log streams, console children) to SIGTERM on
/// the way out. A small fixed pool: registration is best-effort — a session has
/// at most a handful of these at once. (The spawn→register gap is accepted:
/// a signal landing inside it leaks that one child rather than risking a
/// kill on a recycled pid.)
static CHILD_PIDS: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];

/// The forward-only child (0 = none): on INT/TERM/HUP the handler sends it
/// SIGINT and returns instead of exiting (see the module doc).
static FORWARD_PID: AtomicU32 = AtomicU32::new(0);

/// Set when the handler forwarded a signal in forward-only mode, so the
/// command knows its child ended because of a signal.
static FORWARDED: AtomicBool = AtomicBool::new(false);

/// Whether stdin is currently in raw (no-echo) mode, plus the termios pair:
/// the original settings to restore on the way out, and the applied raw
/// settings to re-assert after a SIGCONT. Pointers are written once (leaked
/// boxes) and updated in place while `RAW_ACTIVE` is false.
static RAW_ACTIVE: AtomicBool = AtomicBool::new(false);
static RAW_TERMIOS: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());
static RAW_APPLIED: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());

/// Install the handlers. Called once at CLI startup.
#[allow(clippy::fn_to_numeric_cast_any)] // sighandler_t is how signal(2) takes a handler
pub fn install() {
    let handler = handle as extern "C" fn(libc::c_int);
    let tstp = handle_tstp as extern "C" fn(libc::c_int);
    let cont = handle_cont as extern "C" fn(libc::c_int);
    // Safety: isatty on a constant fd; signal() installs async-signal-safe
    // handlers before any work spawns children or flips terminal modes.
    unsafe {
        STDERR_TTY.store(libc::isatty(libc::STDERR_FILENO) == 1, Ordering::Relaxed);
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGPIPE] {
            install_unless_ignored(sig, handler as libc::sighandler_t);
        }
        install_unless_ignored(libc::SIGTSTP, tstp as libc::sighandler_t);
        libc::signal(libc::SIGCONT, cont as libc::sighandler_t);
    }
}

/// Install `handler` for `sig` unless the process inherited `SIG_IGN` — the
/// POSIX convention protecting background jobs (`nohup`, `cmd &` under a
/// non-job-control shell) from the foreground group's signals.
unsafe fn install_unless_ignored(sig: libc::c_int, handler: libc::sighandler_t) {
    // Safety: caller holds the unsafe context; signal() is async-signal-safe.
    unsafe {
        let prev = libc::signal(sig, handler);
        if prev == libc::SIG_IGN {
            libc::signal(sig, libc::SIG_IGN);
        }
    }
}

/// Record the interruptible build's process group so the handler forwards
/// signals to it. Cleared with [`clear_build_pgid`] once the build's output
/// stream ends (just before the reaping `wait`, so the handler can never
/// signal a recycled group).
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

/// Deregister a child. Call this *before* killing/waiting the child — after
/// the reap its pid can be recycled, and the handler must never signal a
/// stranger.
pub fn unregister_child(slot: Option<usize>) {
    if let Some(i) = slot {
        CHILD_PIDS[i].store(0, Ordering::Release);
    }
}

/// Enter forward-only mode for `pid` (see the module doc). Cleared with
/// [`clear_forward_child`] before the child is reaped.
pub fn set_forward_child(pid: u32) {
    FORWARDED.store(false, Ordering::Release);
    FORWARD_PID.store(pid, Ordering::Release);
}

pub fn clear_forward_child() {
    FORWARD_PID.store(0, Ordering::Release);
}

/// Whether the handler forwarded a signal since [`set_forward_child`] —
/// consumed by the owning command to report "stopped by the user" instead of
/// a child failure.
pub fn take_forwarded() -> bool {
    FORWARDED.swap(false, Ordering::AcqRel)
}

/// Record the termios pair while raw mode is active: `original` to restore on
/// any exit, `applied` to re-assert after SIGCONT. Called by
/// [`crate::cli::rawmode::RawMode::enable`] *before* the terminal is actually
/// flipped, so a signal in the gap restores idempotently.
pub fn set_raw(original: libc::termios, applied: libc::termios) {
    store_termios(&RAW_TERMIOS, original);
    store_termios(&RAW_APPLIED, applied);
    RAW_ACTIVE.store(true, Ordering::Release);
}

fn store_termios(slot: &AtomicPtr<libc::termios>, value: libc::termios) {
    let ptr = slot.load(Ordering::Acquire);
    if ptr.is_null() {
        slot.store(Box::into_raw(Box::new(value)), Ordering::Release);
    } else {
        // Safety: the pointer is a leaked box only this (main-thread) writer
        // updates, and only while RAW_ACTIVE is false; the handler only reads
        // it while RAW_ACTIVE is true.
        unsafe { *ptr = value };
    }
}

pub fn clear_raw() {
    RAW_ACTIVE.store(false, Ordering::Release);
}

/// Run `f` with SIGINT ignored, restoring the previous disposition after.
/// For interactive foreground children (lldb) that own the terminal's Ctrl-C
/// semantics: the debugger uses SIGINT to break into the debuggee, and the
/// CLI dying underneath it would orphan lldb against the shell's prompt.
pub fn with_sigint_ignored<T>(f: impl FnOnce() -> T) -> T {
    // Safety: swapping one disposition around a foreground child on the main
    // thread, restored before anything else observes it.
    unsafe {
        let prev = libc::signal(libc::SIGINT, libc::SIG_IGN);
        let result = f();
        libc::signal(libc::SIGINT, prev);
        result
    }
}

/// Suspend the CLI as if the user pressed Ctrl-Z on a cooked terminal: the
/// TSTP handler restores the terminal, stops the process, and SIGCONT
/// re-asserts raw mode. Used by the session's `^Z` key (raw mode eats the
/// real one).
pub fn suspend_self() {
    // Safety: kill(2) on our own pid with a stop signal.
    unsafe {
        libc::kill(std::process::id().cast_signed(), libc::SIGTSTP);
    }
}

/// Erase-the-spinner-line escape, written to a TTY stderr on the way out.
const CLEAR_LINE: &[u8] = b"\r\x1b[K";

/// The handler: erase any in-flight spinner line, restore the terminal,
/// forward the signal to the build's process group, SIGTERM registered
/// children, and exit `128 + signo`. In forward-only mode, forward SIGINT to
/// the registered child and return instead. Async-signal-safe calls only.
extern "C" fn handle(sig: libc::c_int) {
    let forward = FORWARD_PID.load(Ordering::Acquire);
    if forward != 0 && matches!(sig, libc::SIGINT | libc::SIGTERM | libc::SIGHUP) {
        FORWARDED.store(true, Ordering::Release);
        // Safety: kill(2) on a recorded child pid; SIGINT is the finalizer
        // (recordVideo writes the moov atom on it, log streams just exit).
        unsafe {
            libc::kill(forward.cast_signed(), libc::SIGINT);
        }
        return;
    }
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
    if forward != 0 {
        // A SIGPIPE while a forward-only child runs: let it finalize, then die.
        unsafe {
            libc::kill(forward.cast_signed(), libc::SIGINT);
        }
    }
    for slot in &CHILD_PIDS {
        let pid = slot.swap(0, Ordering::AcqRel);
        if pid != 0 {
            // Safety: kill(2) on a recorded child pid (deregistration happens
            // before reaping, so the pid can't have been recycled).
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

/// SIGTSTP: hand the shell a cooked terminal, stop with the default
/// disposition, and re-arm on resume. Async-signal-safe: tcsetattr, signal,
/// kill.
#[allow(clippy::fn_to_numeric_cast_any)] // sighandler_t is how signal(2) takes a handler
extern "C" fn handle_tstp(_sig: libc::c_int) {
    if RAW_ACTIVE.load(Ordering::Acquire) {
        let ptr = RAW_TERMIOS.load(Ordering::Acquire);
        if !ptr.is_null() {
            // Safety: restoring the stashed termios.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, ptr);
            }
        }
    }
    // Safety: flip to the default stop disposition and deliver it. SIGTSTP is
    // blocked while its own handler runs, so the re-raise stays pending until
    // explicitly unblocked — unblock with SIG_DFL in place (the stop happens
    // there), then re-block and re-arm once execution resumes after SIGCONT.
    unsafe {
        libc::signal(libc::SIGTSTP, libc::SIG_DFL);
        libc::kill(std::process::id().cast_signed(), libc::SIGTSTP);
        let mut mask: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut mask);
        libc::sigaddset(&raw mut mask, libc::SIGTSTP);
        libc::sigprocmask(libc::SIG_UNBLOCK, &raw const mask, std::ptr::null_mut());
        // Stopped here; resumed by SIGCONT.
        libc::sigprocmask(libc::SIG_BLOCK, &raw const mask, std::ptr::null_mut());
        let tstp = handle_tstp as extern "C" fn(libc::c_int);
        libc::signal(libc::SIGTSTP, tstp as libc::sighandler_t);
    }
}

/// SIGCONT: re-assert raw mode if a session was suspended mid-raw, so `fg`
/// resumes with a working keymap.
extern "C" fn handle_cont(_sig: libc::c_int) {
    if RAW_ACTIVE.load(Ordering::Acquire) {
        let ptr = RAW_APPLIED.load(Ordering::Acquire);
        if !ptr.is_null() {
            // Safety: re-applying the raw termios the main thread stashed.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, ptr);
            }
        }
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

    #[test]
    fn forward_mode_round_trips() {
        set_forward_child(777);
        assert_eq!(FORWARD_PID.load(Ordering::Acquire), 777);
        assert!(!take_forwarded()); // nothing forwarded yet
        FORWARDED.store(true, Ordering::Release);
        assert!(take_forwarded());
        assert!(!take_forwarded()); // consumed
        clear_forward_child();
        assert_eq!(FORWARD_PID.load(Ordering::Acquire), 0);
    }
}
