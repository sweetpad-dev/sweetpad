//! The InjectionNext client/server wire protocol — the bytes the in-app client
//! (`ClientBoot.mm` + `InjectionNext.swift`, MIT © John Holdsworth) speaks over a
//! localhost TCP socket. The CLI plays the *server* role here (the role
//! `InjectionNext.app` normally plays); the app is the TCP client.
//!
//! Framing is native little-endian: an `int` is a 4-byte `int32`; a `string` or
//! `data` is an `int32` length followed by raw bytes; the EOF sentinel is `-1`.
//! A command is an `int32` code optionally followed by a string payload.
//!
//! Validated end-to-end against a real simulator (see `ci/hot-reload-spike`).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// `HOTRELOADING_PORT` — the localhost port the in-app client dials.
pub const PORT: u16 = 8887;
/// `INJECTION_VERSION` the client announces first (validated, not enforced).
pub const INJECTION_VERSION: i32 = 4001;

/// `InjectionCommand` (server → app), from `InjectionClient.h`.
pub mod command {
    pub const LOG: i32 = 0;
    pub const LOAD: i32 = 1;
    pub const INJECT: i32 = 2;
    pub const XCODE_PATH: i32 = 3;
}

/// `InjectionResponse` (app → server), from `InjectionClient.h`.
pub mod response {
    pub const PLATFORM: i32 = 0;
    pub const INJECTED: i32 = 1;
    pub const FAILED: i32 = 2;
    pub const TMP_PATH: i32 = 3;
    pub const UNHIDE: i32 = 4;
    pub const PROJECT_ROOT: i32 = 5;
    pub const DETAIL: i32 = 6;
    pub const BAZEL_TARGET: i32 = 7;
    pub const EXECUTABLE: i32 = 8;
}

/// Write a 4-byte little-endian `int32`.
pub fn write_int(s: &mut TcpStream, v: i32) -> std::io::Result<()> {
    s.write_all(&v.to_le_bytes())
}

/// Write a length-prefixed string (`int32` byte-length + UTF-8 bytes).
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // protocol strings are short paths
pub fn write_string(s: &mut TcpStream, v: &str) -> std::io::Result<()> {
    write_int(s, v.len() as i32)?;
    s.write_all(v.as_bytes())
}

/// Write a command code optionally followed by a string payload.
pub fn write_command(s: &mut TcpStream, cmd: i32, arg: Option<&str>) -> std::io::Result<()> {
    write_int(s, cmd)?;
    if let Some(v) = arg {
        write_string(s, v)?;
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes. `Ok(false)` distinguishes a clean read
/// timeout (the socket's read timeout fired) from EOF/error (`Err`).
fn read_exact_timed(s: &mut TcpStream, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match s.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ));
            }
            Ok(n) => filled += n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return Ok(false);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Read a 4-byte `int32`, or `None` on a read timeout.
pub fn read_int(s: &mut TcpStream) -> std::io::Result<Option<i32>> {
    let mut b = [0u8; 4];
    Ok(read_exact_timed(s, &mut b)?.then(|| i32::from_le_bytes(b)))
}

/// Read a length-prefixed string. Blocks (ignoring intermediate timeouts) until
/// the length and body arrive, since a partial string would desync the stream.
#[allow(clippy::cast_sign_loss)] // length is checked non-negative above
pub fn read_string(s: &mut TcpStream) -> std::io::Result<String> {
    let len = loop {
        if let Some(v) = read_int(s)? {
            break v;
        }
    };
    if len < 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "EOF reading string length",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    let mut filled = 0;
    while filled < buf.len() {
        match s.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed mid-string",
                ));
            }
            Ok(n) => filled += n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
    String::from_utf8(buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}

/// A short read timeout used while draining the handshake (to detect its end).
pub const HANDSHAKE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_and_response_codes_match_the_header() {
        // Spot-check the two we send and the three we act on.
        assert_eq!(command::LOAD, 1);
        assert_eq!(command::XCODE_PATH, 3);
        assert_eq!(response::INJECTED, 1);
        assert_eq!(response::FAILED, 2);
        assert_eq!(response::UNHIDE, 4);
    }

    #[test]
    fn int_is_little_endian_four_bytes() {
        // Mirror the framing the Swift side reads (`int32` LE).
        assert_eq!(4001_i32.to_le_bytes(), [0xa1, 0x0f, 0x00, 0x00]);
    }
}
