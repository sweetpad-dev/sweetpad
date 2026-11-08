//! The requirements probe Swift Build uses before and after signing a copied
//! Swift runtime library. This reads signatures without loading a private key.

use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use super::{blobs, crypto, macho, plist};

pub(super) fn display(path: &Path) -> Result<(), String> {
    let executable = if path.is_dir() {
        let info = std::fs::read(path.join("Info.plist")).map_err(|e| e.to_string())?;
        let info = plist::read_info(&info)?;
        path.join(plist::string(&info, "CFBundleExecutable").ok_or("bundle has no executable")?)
    } else {
        path.to_path_buf()
    };
    let data = std::fs::read(&executable).map_err(|e| e.to_string())?;
    let text = requirements(&data)?;
    eprintln!("Executable={}", executable.display());
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())
}

pub(super) fn requirements(data: &[u8]) -> Result<String, String> {
    let slices = macho::split(data)?;
    let slice = slices
        .iter()
        .find(|s| s.arch.is_some_and(|a| a.0 == 0x0100_000c))
        .or_else(|| slices.first())
        .ok_or("empty universal executable")?;
    let layout = macho::Layout::parse(&slice.data)?;
    let signature = layout
        .signature(&slice.data)
        .ok_or("code object is not signed at all")?;
    let mut text = String::new();
    let mut designated = false;
    if let Some(set) = slot(signature, blobs::SLOT_REQUIREMENTS)? {
        if word(set, 0)? != blobs::REQUIREMENTS {
            return Err("invalid requirements blob".into());
        }
        for index in 0..word(set, 8)? as usize {
            let kind = word(set, 12 + 8 * index)?;
            let at = word(set, 16 + 8 * index)? as usize;
            let len = word(set, at + 4)? as usize;
            let requirement = set.get(at..at + len).ok_or("truncated requirement")?;
            if word(requirement, 0)? != blobs::REQUIREMENT || word(requirement, 8)? != 1 {
                return Err("unsupported requirement format".into());
            }
            let name = match kind {
                1 => "host",
                2 => "guest",
                3 => "designated",
                4 => "library",
                5 => "plugin",
                _ => return Err(format!("unsupported requirement type {kind}")),
            };
            let mut reader = Reader {
                data: &requirement[12..],
                at: 0,
            };
            let expression = reader.expression(0, 0)?;
            if reader.at != reader.data.len() {
                return Err("trailing requirement data".into());
            }
            designated |= kind == 3;
            let _ = writeln!(text, "{name} => {expression}");
        }
    }
    if !designated {
        let directory = slot(signature, blobs::SLOT_CODE_DIRECTORY)?
            .ok_or("signature has no code directory")?;
        let hash = match directory.get(37) {
            Some(1) => crypto::sha1(directory).to_vec(),
            Some(2 | 3) => crypto::sha256(directory)[..20].to_vec(),
            _ => return Err("unsupported code directory hash".into()),
        };
        let _ = writeln!(text, "# designated => cdhash H\"{}\"", crypto::hex(&hash));
    }
    Ok(text)
}

fn word(bytes: &[u8], at: usize) -> Result<u32, String> {
    let bytes = bytes.get(at..at + 4).ok_or("truncated signature")?;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| "truncated signature")?,
    ))
}

fn slot(signature: &[u8], wanted: u32) -> Result<Option<&[u8]>, String> {
    if word(signature, 0)? != blobs::EMBEDDED_SIGNATURE {
        return Err("invalid signature blob".into());
    }
    for index in 0..word(signature, 8)? as usize {
        if word(signature, 12 + 8 * index)? == wanted {
            let at = word(signature, 16 + 8 * index)? as usize;
            let len = word(signature, at + 4)? as usize;
            return signature
                .get(at..at + len)
                .map(Some)
                .ok_or("truncated signature slot".into());
        }
    }
    Ok(None)
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn word(&mut self) -> Result<u32, String> {
        let value = word(self.data, self.at)?;
        self.at += 4;
        Ok(value)
    }

    fn bytes(&mut self) -> Result<&[u8], String> {
        let len = self.word()? as usize;
        let data = self
            .data
            .get(self.at..self.at + len)
            .ok_or("truncated requirement string")?;
        self.at += len.next_multiple_of(4);
        if self.at > self.data.len() {
            return Err("truncated requirement padding".into());
        }
        Ok(data)
    }

    fn text(&mut self, dotted: bool) -> Result<String, String> {
        let text =
            std::str::from_utf8(self.bytes()?).map_err(|_| "non-UTF-8 requirement string")?;
        if !text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (dotted && b == b'.'))
        {
            Ok(text.to_string())
        } else {
            Ok(format!("{text:?}"))
        }
    }

    fn cert(&mut self) -> Result<String, String> {
        Ok(match self.word()? {
            0 => "leaf".to_string(),
            u32::MAX => "root".to_string(),
            n => n.to_string(),
        })
    }

    fn match_value(&mut self) -> Result<String, String> {
        match self.word()? {
            0 => Ok(" /* exists */".into()),
            1 => Ok(format!(" = {}", self.text(false)?)),
            n => Err(format!("unsupported requirement match {n}")),
        }
    }

    // Precedence preserves grouping while allowing associative conjunctions.
    fn expression(&mut self, parent: u8, depth: u8) -> Result<String, String> {
        if depth > 64 {
            return Err("requirement nesting limit exceeded".into());
        }
        let op = self.word()?;
        Ok(match op {
            0 => "never".into(),
            1 => "always".into(),
            2 => format!("identifier {}", self.text(false)?),
            3 => "anchor apple".into(),
            15 => "anchor apple generic".into(),
            4 => {
                let cert = self.cert()?;
                format!("certificate {cert} = H\"{}\"", crypto::hex(self.bytes()?))
            }
            6 | 7 => {
                let level = if op == 6 { 2 } else { 1 };
                let left = self.expression(level, depth + 1)?;
                let right = self.expression(level, depth + 1)?;
                let text = format!("{left} {} {right}", if op == 6 { "and" } else { "or" });
                if level < parent {
                    format!("({text})")
                } else {
                    text
                }
            }
            8 => format!("cdhash H\"{}\"", crypto::hex(self.bytes()?)),
            9 => format!("! {}", self.expression(3, depth + 1)?),
            11 => {
                let cert = self.cert()?;
                let field = self.text(true)?;
                format!("certificate {cert}[{field}]{}", self.match_value()?)
            }
            14 => {
                let cert = self.cert()?;
                let oid = self.bytes()?;
                let (&first, rest) = oid.split_first().ok_or("empty requirement OID")?;
                let mut fields = vec![
                    u64::from(first.min(80) / 40),
                    u64::from(first - first.min(80) / 40 * 40),
                ];
                let mut value = 0u64;
                for &byte in rest {
                    value = value
                        .checked_mul(128)
                        .and_then(|n| n.checked_add(u64::from(byte & 127)))
                        .ok_or("requirement OID overflow")?;
                    if byte & 128 == 0 {
                        fields.push(value);
                        value = 0;
                    }
                }
                if rest.last().is_some_and(|b| b & 128 != 0) {
                    return Err("truncated requirement OID".into());
                }
                let oid = fields
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(".");
                format!("certificate {cert}[field.{oid}]{}", self.match_value()?)
            }
            _ => return Err(format!("unsupported requirement opcode {op}")),
        })
    }
}
