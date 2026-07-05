//! `Content-Length`-framed JSON-RPC message codec, shared by the BSP server's
//! stdio loop (toward sourcekit-lsp), its control-socket client (toward the
//! extension), and the `sweetpad vscode` CLI client.

use std::io::{BufRead, Write};

/// Largest frame we'll accept. Real JSON-RPC bodies here top out at tens of
/// kilobytes (build-target/source lists); the cap stops a hostile or corrupt
/// `Content-Length` from forcing a giant zero-filled allocation (and a
/// `read_exact` that waits forever for a body that never comes) before a
/// single body byte is read.
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Largest accepted header line. Real headers are tens of bytes; without a
/// bound, a peer that streams bytes with no newline grows the header buffer
/// without limit — the exact allocation blowup [`MAX_FRAME_BYTES`] exists to
/// stop, just before the body instead of inside it.
const MAX_HEADER_BYTES: u64 = 64 * 1024;

/// Read one `Content-Length`-framed JSON-RPC message. `Ok(None)` on clean EOF.
pub fn read_message(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut raw = Vec::new();
        // UFCS so `take` binds to `impl Read for &mut R` (a plain method call
        // auto-derefs and tries to move the reader itself).
        let mut limited = std::io::Read::take(&mut *reader, MAX_HEADER_BYTES);
        let n = limited
            .read_until(b'\n', &mut raw)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        if raw.last() != Some(&b'\n') && n as u64 == MAX_HEADER_BYTES {
            return Err(format!(
                "header line exceeds the {MAX_HEADER_BYTES}-byte cap"
            ));
        }
        let line = String::from_utf8_lossy(&raw);
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        // Header names are case-insensitive; don't die on a client that
        // doesn't send the canonical casing. A malformed value is a hard
        // error (the frame boundary is unrecoverable without it).
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            let parsed = value
                .trim()
                .parse()
                .map_err(|e| format!("bad Content-Length {:?}: {e}", value.trim()))?;
            content_length = Some(parsed);
        }
    }
    let len = content_length.ok_or("message without Content-Length")?;
    if len > MAX_FRAME_BYTES {
        return Err(format!(
            "Content-Length {len} exceeds the {MAX_FRAME_BYTES}-byte frame cap"
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

pub fn write_message(writer: &mut impl Write, body: &str) -> Result<(), String> {
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}
