//! JPEG and HEIF images, which the car keeps as they are. What it records
//! beside the file comes from its headers: its size in pixels, before any
//! rotation, and its EXIF orientation, 1 to 8.

/// A photo's format, size and orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Photo {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub orientation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Format {
    Jpeg,
    Heif,
}

impl Photo {
    /// The photo in `data`, when it's a JPEG or HEIF image whose size its
    /// headers give.
    pub fn read(data: &[u8]) -> Option<Photo> {
        if data.starts_with(&[0xff, 0xd8]) {
            jpeg(data)
        } else {
            heif(data)
        }
    }
}

/// A JPEG's size, from its frame header, and the orientation its EXIF
/// segment gives.
fn jpeg(data: &[u8]) -> Option<Photo> {
    let mut orientation = 1;
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xff {
            return None;
        }
        let marker = data[i + 1];
        // Fill bytes, and the markers with no length.
        if marker == 0xff {
            i += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd9) {
            i += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[i + 2], data[i + 3]]));
        let segment = data.get(i + 4..i + 2 + length)?;
        match marker {
            0xe1 => {
                if let Some(tiff) = segment.strip_prefix(b"Exif\0\0") {
                    orientation = exif_orientation(tiff).unwrap_or(1);
                }
            }
            // The start of frame markers; C4, C8 and CC are other tables.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let size =
                    |at: usize| u32::from(u16::from_be_bytes([segment[at], segment[at + 1]]));
                if segment.len() < 5 {
                    return None;
                }
                return Some(Photo {
                    format: Format::Jpeg,
                    width: size(3),
                    height: size(1),
                    orientation,
                });
            }
            _ => {}
        }
        i += 2 + length;
    }
    None
}

/// The orientation tag of an EXIF block's first image directory.
fn exif_orientation(tiff: &[u8]) -> Option<u32> {
    let big = match tiff.get(..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        let bytes = [*tiff.get(at)?, *tiff.get(at + 1)?];
        Some(if big {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let u32_at = |at: usize| {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    };
    let directory = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(directory)?);
    (0..count)
        .map(|n| directory + 2 + 12 * n)
        .find(|&entry| u16_at(entry) == Some(0x0112))
        .and_then(|entry| u16_at(entry + 8))
        .map(u32::from)
        .filter(|o| (1..=8).contains(o))
}

/// One box of an ISO media file: its type and contents.
fn boxes(mut data: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
        let kind = data.get(4..8)?;
        let (start, end) = match size {
            0 => (8, data.len()),
            1 => {
                let large = u64::from_be_bytes(data.get(8..16)?.try_into().ok()?);
                (16, usize::try_from(large).ok()?)
            }
            size => (8, size),
        };
        let contents = data.get(start..end)?;
        data = &data[end..];
        Some((kind, contents))
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
    boxes(data).find(|(k, _)| *k == kind).map(|(_, c)| c)
}

/// A HEIF image's size, from the `ispe` property of its primary item, and
/// its orientation, from that item's `irot` rotation.
fn heif(data: &[u8]) -> Option<Photo> {
    let brands = child(data, b"ftyp")?;
    let heif_brand = brands
        .chunks(4)
        .any(|b| matches!(b, b"heic" | b"heix" | b"mif1" | b"msf1" | b"heim" | b"heis"));
    if !heif_brand {
        return None;
    }
    // `meta` and the boxes in it that start with a version and flags.
    let meta = child(data, b"meta")?.get(4..)?;
    let primary = child(meta, b"pitm")?;
    let primary = match primary.first()? {
        0 => u32::from(u16::from_be_bytes(primary.get(4..6)?.try_into().ok()?)),
        _ => u32::from_be_bytes(primary.get(4..8)?.try_into().ok()?),
    };
    let iprp = child(meta, b"iprp")?;
    let properties: Vec<(&[u8], &[u8])> = boxes(child(iprp, b"ipco")?).collect();
    let mut width_height = None;
    let mut orientation = 1;
    for index in associations(child(iprp, b"ipma")?, primary)? {
        let Some(&(kind, contents)) = properties.get(index.checked_sub(1)?) else {
            continue;
        };
        match kind {
            b"ispe" => {
                let size = |at: usize| {
                    Some(u32::from_be_bytes(
                        contents.get(at..at + 4)?.try_into().ok()?,
                    ))
                };
                width_height = Some((size(4)?, size(8)?));
            }
            // A turn of a quarter, a half or three quarters anticlockwise.
            b"irot" => {
                orientation = match contents.first()? & 3 {
                    1 => 8,
                    2 => 3,
                    3 => 6,
                    _ => 1,
                };
            }
            _ => {}
        }
    }
    let (width, height) = width_height?;
    Some(Photo {
        format: Format::Heif,
        width,
        height,
        orientation,
    })
}

/// The 1-based indexes of the properties `ipma` gives the item `item`.
fn associations(ipma: &[u8], item: u32) -> Option<Vec<usize>> {
    let version = *ipma.first()?;
    let wide_indexes = ipma.get(3)? & 1 == 1;
    let count = u32::from_be_bytes(ipma.get(4..8)?.try_into().ok()?);
    let mut at = 8;
    for _ in 0..count {
        let id = if version < 1 {
            at += 2;
            u32::from(u16::from_be_bytes(ipma.get(at - 2..at)?.try_into().ok()?))
        } else {
            at += 4;
            u32::from_be_bytes(ipma.get(at - 4..at)?.try_into().ok()?)
        };
        let n = usize::from(*ipma.get(at)?);
        at += 1;
        let mut indexes = Vec::with_capacity(n);
        for _ in 0..n {
            let index = if wide_indexes {
                at += 2;
                usize::from(u16::from_be_bytes(ipma.get(at - 2..at)?.try_into().ok()?) & 0x7fff)
            } else {
                at += 1;
                usize::from(ipma.get(at - 1)? & 0x7f)
            };
            indexes.push(index);
        }
        if id == item {
            return Some(indexes);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(marker: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![0xff, marker];
        out.extend_from_slice(&u16::try_from(data.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn a_jpeg_gives_its_frame_size_and_exif_orientation() {
        // A big-endian directory with one entry: orientation 6.
        let exif = [
            &b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01"[..],
            &[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0],
            &[0, 0, 0, 0],
        ]
        .concat();
        let jpeg = [
            vec![0xff, 0xd8],
            segment(0xe0, b"JFIF\0\x01\x01\0\0\x48\0\x48\0\0"),
            segment(0xe1, &exif),
            segment(0xdb, &[0; 65]),
            segment(0xc2, &[8, 0, 80, 0, 120, 3]),
            vec![0xff, 0xda],
        ]
        .concat();
        assert_eq!(
            Photo::read(&jpeg),
            Some(Photo {
                format: Format::Jpeg,
                width: 120,
                height: 80,
                orientation: 6
            })
        );
        assert_eq!(Photo::read(&jpeg[..40]), None);
    }

    fn mp4_box(kind: &[u8], contents: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(contents.len() + 8)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(contents);
        out
    }

    #[test]
    fn a_heif_gives_its_primary_items_size_and_rotation() {
        let full =
            |kind: &[u8], contents: &[u8]| mp4_box(kind, &[&[0, 0, 0, 0][..], contents].concat());
        let ispe = |w: u32, h: u32| full(b"ispe", &[w.to_be_bytes(), h.to_be_bytes()].concat());
        // Item 1 is a thumbnail; item 2, the primary, has properties 2 and 3.
        let ipco = mp4_box(
            b"ipco",
            &[ispe(32, 32), ispe(4032, 3024), mp4_box(b"irot", &[3])].concat(),
        );
        let ipma = full(b"ipma", &[0, 0, 0, 2, 0, 1, 1, 0x81, 0, 2, 2, 0x82, 3]);
        let meta = full(
            b"meta",
            &[
                full(b"pitm", &[0, 2]),
                mp4_box(b"iprp", &[ipco, ipma].concat()),
            ]
            .concat(),
        );
        let file = [mp4_box(b"ftyp", b"heic\0\0\0\0mif1heic"), meta].concat();
        assert_eq!(
            Photo::read(&file),
            Some(Photo {
                format: Format::Heif,
                width: 4032,
                height: 3024,
                orientation: 6
            })
        );
    }
}
