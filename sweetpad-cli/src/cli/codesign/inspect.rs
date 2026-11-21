//! Reading signatures without a private key: the requirements probe Swift
//! Build uses before and after signing a copied Swift runtime library, and
//! the code directory hash and designated requirement a bundle seals nested
//! code with.

use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use super::bundle::Layout;
use super::{blobs, crypto, macho};

pub(super) fn display(path: &Path) -> Result<(), String> {
    let (executable, text) = if path.is_dir() {
        let layout = Layout::of(path)?;
        let code = layout.code(path)?;
        let text = if code.detached {
            signature_requirements(&detached_signature(&layout.root)?)
        } else {
            requirements(&code.bytes)
        }?;
        (layout.root.join(code.name), text)
    } else {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        (path.to_path_buf(), requirements(&data)?)
    };
    eprintln!("Executable={}", executable.display());
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())
}

pub(super) fn requirements(data: &[u8]) -> Result<String, String> {
    let slices = macho::split(data)?;
    let slice = preferred(&slices).ok_or("empty universal executable")?;
    let layout = macho::Layout::parse(&slice.data)?;
    let signature = layout
        .signature(&slice.data)
        .ok_or("code object is not signed at all")?;
    signature_requirements(signature)
}

fn detached_signature(root: &Path) -> Result<Vec<u8>, String> {
    let folder = root.join("_CodeSignature");
    let read = |name| {
        std::fs::read(folder.join(name))
            .map_err(|e| format!("couldn't read {}: {e}", folder.join(name).display()))
    };
    Ok(blobs::superblob(vec![
        (blobs::SLOT_CODE_DIRECTORY, read("CodeDirectory")?),
        (blobs::SLOT_REQUIREMENTS, read("CodeRequirements")?),
    ]))
}

fn signature_requirements(signature: &[u8]) -> Result<String, String> {
    let mut text = String::new();
    let mut designated = false;
    if let Some(set) = slot(signature, blobs::SLOT_REQUIREMENTS)? {
        for (kind, expression) in requirement_set(set)? {
            let name = match kind {
                1 => "host",
                2 => "guest",
                3 => "designated",
                4 => "library",
                5 => "plugin",
                _ => return Err(format!("unsupported requirement type {kind}")),
            };
            designated |= kind == 3;
            let _ = writeln!(text, "{name} => {expression}");
        }
    }
    if !designated {
        let hash = cdhash(signature)?;
        let _ = writeln!(text, "# designated => cdhash H\"{}\"", crypto::hex(&hash));
    }
    Ok(text)
}

/// The slice `codesign` reads a universal file by on an Apple silicon Mac:
/// the arm64 one, else the first.
fn preferred(slices: &[macho::Slice]) -> Option<&macho::Slice> {
    slices
        .iter()
        .find(|s| s.arch.is_some_and(|a| a.0 == macho::CPU_TYPE_ARM64))
        .or_else(|| slices.first())
}

/// What a bundle seals nested code at `path` with: its code directory hash
/// and its designated requirement as `codesign` prints it. An ad hoc
/// signature's is its code directory hash, every slice's for universal
/// code, the preferred slice's first.
pub(super) fn designated(path: &Path) -> Result<(Vec<u8>, String), String> {
    let unsigned = || "code object is not signed at all".to_string();
    let data = if path.is_dir() {
        let layout = Layout::of(path)?;
        let code = layout.code(path)?;
        if code.detached {
            let signature = detached_signature(&layout.root)?;
            let hash = cdhash(&signature)?;
            let set = slot(&signature, blobs::SLOT_REQUIREMENTS)?.ok_or_else(unsigned)?;
            let explicit = requirement_set(set)?
                .into_iter()
                .find_map(|(kind, text)| (kind == 3).then_some(text));
            let text = explicit.unwrap_or_else(|| format!("cdhash H\"{}\"", crypto::hex(&hash)));
            return Ok((hash, text));
        }
        code.bytes.into_owned()
    } else {
        std::fs::read(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?
    };
    let slices = macho::split(&data).map_err(|_| unsigned())?;
    let first = preferred(&slices).ok_or_else(unsigned)?;
    let mut ordered = vec![first];
    ordered.extend(slices.iter().filter(|s| !std::ptr::eq(*s, first)));
    let mut hashes = Vec::new();
    let mut explicit = None;
    for slice in ordered {
        let layout = macho::Layout::parse(&slice.data)?;
        let signature = layout.signature(&slice.data).ok_or_else(unsigned)?;
        hashes.push(cdhash(signature)?);
        if explicit.is_none()
            && let Some(set) = slot(signature, blobs::SLOT_REQUIREMENTS)?
        {
            explicit = requirement_set(set)?
                .into_iter()
                .find_map(|(kind, text)| (kind == 3).then_some(text));
        }
    }
    let text = explicit.unwrap_or_else(|| {
        hashes
            .iter()
            .map(|hash| format!("cdhash H\"{}\"", crypto::hex(hash)))
            .collect::<Vec<_>>()
            .join(" or ")
    });
    Ok((hashes.swap_remove(0), text))
}

/// The requirements of a requirement set blob, by kind, as text.
fn requirement_set(set: &[u8]) -> Result<Vec<(u32, String)>, String> {
    if word(set, 0)? != blobs::REQUIREMENTS {
        return Err("invalid requirements blob".into());
    }
    let mut out = Vec::new();
    for index in 0..word(set, 8)? as usize {
        let kind = word(set, 12 + 8 * index)?;
        let at = word(set, 16 + 8 * index)? as usize;
        let len = word(set, at + 4)? as usize;
        let requirement = set.get(at..at + len).ok_or("truncated requirement")?;
        if word(requirement, 0)? != blobs::REQUIREMENT || word(requirement, 8)? != 1 {
            return Err("unsupported requirement format".into());
        }
        let mut reader = Reader {
            data: &requirement[12..],
            at: 0,
        };
        let expression = reader.expression(0, 0)?;
        if reader.at != reader.data.len() {
            return Err("trailing requirement data".into());
        }
        out.push((kind, expression));
    }
    Ok(out)
}

/// Each slice's code directory hash type and hash, the preferred slice's
/// first: what a notarization ticket is filed under.
pub(super) fn cdhashes(executable: &Path) -> Result<Vec<(u8, Vec<u8>)>, String> {
    let data = std::fs::read(executable)
        .map_err(|e| format!("couldn't read {}: {e}", executable.display()))?;
    let slices = macho::split(&data).map_err(|e| format!("{}: {e}", executable.display()))?;
    let first = preferred(&slices).and_then(|s| s.arch);
    let mut ordered: Vec<&macho::Slice> = slices.iter().filter(|s| s.arch == first).collect();
    ordered.extend(slices.iter().filter(|s| s.arch != first));
    let mut out = Vec::new();
    for slice in ordered {
        let layout = macho::Layout::parse(&slice.data)?;
        let signature = layout
            .signature(&slice.data)
            .ok_or_else(|| format!("{} isn't signed", executable.display()))?;
        out.push(ticket_hash(signature)?);
    }
    Ok(out)
}

/// The hash type and hash a signature's notarization ticket is filed
/// under: its code directory's, as [`cdhash`] picks it.
pub(super) fn ticket_hash(signature: &[u8]) -> Result<(u8, Vec<u8>), String> {
    let directory = best_directory(signature)?;
    Ok((
        directory.get(37).copied().unwrap_or(0),
        directory_hash(directory)?,
    ))
}

/// Each blob of `signature`, with its slot, in index order.
pub(super) fn signature_blobs(signature: &[u8]) -> Result<Vec<(u32, Vec<u8>)>, String> {
    if word(signature, 0)? != blobs::EMBEDDED_SIGNATURE {
        return Err("invalid signature blob".into());
    }
    (0..word(signature, 8)? as usize)
        .map(|index| {
            let slot = word(signature, 12 + 8 * index)?;
            let at = word(signature, 16 + 8 * index)? as usize;
            let len = word(signature, at + 4)? as usize;
            let blob = signature
                .get(at..at + len)
                .ok_or("truncated signature slot")?;
            Ok((slot, blob.to_vec()))
        })
        .collect()
}

/// The hash of the signature's code directory: of its SHA-256 one when it
/// has SHA-1 and SHA-256 ones, truncated to 20 bytes.
fn cdhash(signature: &[u8]) -> Result<Vec<u8>, String> {
    directory_hash(best_directory(signature)?)
}

/// The signature's SHA-256 code directory when it has one, else its only.
fn best_directory(signature: &[u8]) -> Result<&[u8], String> {
    let mut best: Option<&[u8]> = None;
    for wanted in [
        blobs::SLOT_CODE_DIRECTORY,
        0x1000,
        0x1001,
        0x1002,
        0x1003,
        0x1004,
    ] {
        if let Some(directory) = slot(signature, wanted)?
            && best.is_none_or(|b| b.get(37) == Some(&1))
        {
            best = Some(directory);
        }
    }
    best.ok_or_else(|| "signature has no code directory".to_string())
}

fn directory_hash(directory: &[u8]) -> Result<Vec<u8>, String> {
    match directory.get(37) {
        Some(1) => Ok(crypto::sha1(directory).to_vec()),
        Some(2 | 3) => Ok(crypto::sha256(directory)[..20].to_vec()),
        _ => Err("unsupported code directory hash".into()),
    }
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
