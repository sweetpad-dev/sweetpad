//! `Content-Length`-framed JSON-RPC message codec, shared by the stdio loop
//! (toward sourcekit-lsp) and the control-socket client (toward the extension).

use std::io::{BufRead, Write};

/// Read one `Content-Length`-framed JSON-RPC message. `Ok(None)` on clean EOF.
pub(crate) fn read_message(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let len = content_length.ok_or("message without Content-Length")?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

pub(crate) fn write_message(writer: &mut impl Write, body: &str) -> Result<(), String> {
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len()).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}
