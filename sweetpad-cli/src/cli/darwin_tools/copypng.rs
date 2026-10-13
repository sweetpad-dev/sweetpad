//! `copypng [-compress] [-strip-PNG-text] SRC DST`, which Swift Build runs
//! for each loose PNG of an iOS target (`COMPRESS_PNG_FILES`, on by default).
//!
//! Xcode's runs `pngcrush -iphone`, which rewrites the image as Apple's CgBI
//! PNG variant, premultiplied BGRA that only Apple's decoders read. UIKit
//! reads a standard PNG just as well, so this copies the file, leaving out
//! its text chunks when asked to.

use std::path::Path;

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The chunks `-strip-PNG-text` leaves out.
const TEXT_CHUNKS: &[&[u8; 4]] = &[b"tEXt", b"zTXt", b"iTXt"];

pub(super) fn main(args: &[String]) -> Result<(), String> {
    let mut strip_text = false;
    let mut files = Vec::new();
    for arg in args {
        match arg.as_str() {
            // Xcode's leaves the image as it is without `-compress`, and
            // this does either way.
            "" | "-compress" => {}
            "-strip-PNG-text" => strip_text = true,
            "-skip-PNGs" => strip_text = false,
            flag if flag.starts_with('-') => {
                println!("WARNING: Ignoring unknown option {flag}");
            }
            file => files.push(Path::new(file)),
        }
    }
    let [source, destination] = files[..] else {
        return Err("expected a source and a destination file".to_string());
    };
    if !source.is_file() {
        return Err(format!("Could not find file: {}", source.display()));
    }
    let data = super::read(source)?;
    let out = if strip_text {
        without_text(&data).ok_or_else(|| {
            format!(
                "caught libpng error: {} is not a PNG file",
                source.display()
            )
        })?
    } else {
        data
    };
    super::write(destination, &out)
}

/// `png` without its text chunks, or `None` when it isn't a PNG.
fn without_text(png: &[u8]) -> Option<Vec<u8>> {
    let mut rest = png.strip_prefix(SIGNATURE)?;
    let mut out = SIGNATURE.to_vec();
    while !rest.is_empty() {
        let length = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        // Length, type, data and CRC.
        let chunk = rest.get(..12 + length)?;
        if !TEXT_CHUNKS.iter().any(|kind| &chunk[4..8] == *kind) {
            out.extend_from_slice(chunk);
        }
        rest = &rest[12 + length..];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(kind: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        // The CRC isn't checked here.
        out.extend_from_slice(&[0; 4]);
        out
    }

    #[test]
    fn text_chunks_are_left_out_and_the_rest_kept_in_order() {
        let header = chunk(*b"IHDR", &[0; 13]);
        let data = chunk(*b"IDAT", b"pixels");
        let end = chunk(*b"IEND", b"");
        let png = [
            SIGNATURE.to_vec(),
            header.clone(),
            chunk(*b"tEXt", b"Comment\0x"),
            data.clone(),
            chunk(*b"iTXt", b"y"),
            end.clone(),
        ]
        .concat();
        assert_eq!(
            without_text(&png).unwrap(),
            [SIGNATURE.to_vec(), header, data, end].concat()
        );
    }

    #[test]
    fn a_file_that_isnt_a_png_is_refused() {
        assert_eq!(without_text(b"GIF89a"), None);
        assert_eq!(without_text(&[SIGNATURE, &[0, 0, 0, 9]].concat()), None);
    }
}
