//! The adapter's stdio, taken over before anything else can write to it.
//!
//! The editor speaks DAP over the process's stdin and stdout, so one stray
//! `println!` corrupts the stream. The rest of the CLI writes freely: notes
//! and warnings go to stderr, app logs and the build log to stdout, and
//! spawned tools inherit both. Rather than teach every one of them about DAP,
//! the protocol moves to private copies of fds 0 and 1, and fds 1 and 2
//! become pipes whose lines the session turns into `output` events. Fd 0
//! becomes `/dev/null`, so no child can read the editor's messages.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// The protocol's private ends, and the read ends of what the process writes
/// to its own stdout and stderr.
pub(super) struct Takeover {
    /// The editor's messages.
    pub input: File,
    /// Where messages to the editor go.
    pub output: File,
    /// Lines written to fd 1: app logs and anything else meant as output.
    pub stdout: File,
    /// Lines written to fd 2: the CLI's notes, warnings and errors.
    pub stderr: File,
}

/// Move the protocol off fds 0 and 1 and capture fds 1 and 2. The private
/// copies are close-on-exec, so no spawned tool holds the editor's pipes
/// open after this process exits.
pub(super) fn take_over() -> io::Result<Takeover> {
    use std::io::Write as _;
    // Nothing buffered for the old fd 1 may land in the new pipe.
    let _ = io::stdout().flush();
    let input = dup_cloexec(libc::STDIN_FILENO)?;
    let output = dup_cloexec(libc::STDOUT_FILENO)?;
    let null = File::open("/dev/null")?;
    redirect(null.as_raw_fd(), libc::STDIN_FILENO)?;
    let (stdout_read, stdout_write) = pipe_cloexec()?;
    let (stderr_read, stderr_write) = pipe_cloexec()?;
    redirect(stdout_write.as_raw_fd(), libc::STDOUT_FILENO)?;
    redirect(stderr_write.as_raw_fd(), libc::STDERR_FILENO)?;
    Ok(Takeover {
        input: File::from(input),
        output: File::from(output),
        stdout: File::from(stdout_read),
        stderr: File::from(stderr_read),
    })
}

/// A close-on-exec duplicate of `fd`, numbered above the standard three.
fn dup_cloexec(fd: RawFd) -> io::Result<OwnedFd> {
    // Safety: fcntl on a descriptor this process owns; the result is checked.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    // Safety: `copy` is a fresh descriptor nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

/// A pipe whose two ends are close-on-exec; [`redirect`] makes the copy on a
/// standard fd inheritable again.
fn pipe_cloexec() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // Safety: `fds` has room for the two descriptors pipe writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // Safety: both descriptors were just created and are owned here.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [&read, &write] {
        // Safety: setting a flag on a descriptor this process owns.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok((read, write))
}

/// Point standard fd `target` at `source`. dup2 leaves the copy inheritable,
/// so spawned tools still write into the captured pipes.
fn redirect(source: RawFd, target: RawFd) -> io::Result<()> {
    // Safety: both are open descriptors; dup2 replaces `target` atomically.
    if unsafe { libc::dup2(source, target) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
