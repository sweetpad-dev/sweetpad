//! Questions asked on the controlling terminal, the password with echo off.
//! Nothing falls back to stdin, so a password can't be piped in or echoed.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use super::LOGIN_HINT;

/// Ask `question` on `/dev/tty`, hiding the answer when `hidden` is set.
pub(crate) fn ask(question: &str, hidden: bool) -> Result<String, String> {
    let no_terminal = || format!("Signing in needs an interactive terminal. {LOGIN_HINT}");
    let terminal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open("/dev/tty")
        .map_err(|_| no_terminal())?;
    // SAFETY: isatty on an open descriptor.
    if unsafe { libc::isatty(terminal.as_raw_fd()) } != 1 {
        return Err(no_terminal());
    }
    ask_on(&terminal, question, hidden)
}

/// [`ask`] on an open terminal.
pub(crate) fn ask_on(terminal: &File, question: &str, hidden: bool) -> Result<String, String> {
    let fd = terminal.as_raw_fd();
    let failed = |error: std::io::Error| format!("Couldn't use the terminal: {error}.");
    let mut original: libc::termios = unsafe { std::mem::zeroed() };
    if hidden {
        // SAFETY: tcgetattr fills a termios we own for an open descriptor.
        if unsafe { libc::tcgetattr(fd, &raw mut original) } != 0 {
            return Err(failed(std::io::Error::last_os_error()));
        }
        let mut quiet = original;
        quiet.c_lflag &= !libc::ECHO;
        // The signal handler restores the terminal if Ctrl-C ends the
        // process while echo is off.
        crate::cli::signals::set_raw_on(fd, original, quiet);
        // SAFETY: applying a modified copy of the descriptor's own settings.
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw const quiet) } != 0 {
            crate::cli::signals::clear_raw();
            return Err(failed(std::io::Error::last_os_error()));
        }
    }
    let answer = (|| {
        let mut writer = terminal;
        writer.write_all(question.as_bytes())?;
        writer.flush()?;
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        let mut reader = terminal;
        loop {
            match reader.read(&mut byte) {
                Ok(0) if line.is_empty() => {
                    return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
                }
                Ok(0) => break,
                Ok(_) if byte[0] == b'\n' => break,
                Ok(_) if line.len() < 4096 => line.push(byte[0]),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        Ok(line)
    })();
    if hidden {
        // SAFETY: restoring the settings read above.
        unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &raw const original) };
        crate::cli::signals::clear_raw();
        let mut writer = terminal;
        let _ = writer.write_all(b"\n");
    }
    let line = answer.map_err(failed)?;
    String::from_utf8(line).map_err(|_| "The answer isn't valid UTF-8.".to_string())
}
