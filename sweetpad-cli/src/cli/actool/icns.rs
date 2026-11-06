//! A macOS app icon as an `.icns` file, the way `iconutil` and actool write
//! one: a big-endian `icns` header, then one entry per size, each a
//! four-letter type, its length and its image. The 16 pixel image is an
//! `ARGB` entry, each channel run-length encoded on its own; the rest are
//! PNGs.

use super::raster::Rgba;

/// The entry type of each size, in points, and scale, in the order Xcode's
/// actool writes them, and whether it's an `ARGB` entry rather than a PNG.
/// It stores the 16 and 128 point sizes, and the car holds the rest.
pub(super) const SLOTS: &[(u32, u32, &[u8; 4], bool)] = &[
    (128, 2, b"ic13", false),
    (16, 2, b"ic11", false),
    (16, 1, b"ic04", true),
    (128, 1, b"ic07", false),
];

/// One size of the icon: its pixels, and the PNG to store for it.
pub(super) struct Image {
    pub points: u32,
    pub scale: u32,
    pub rgba: Rgba,
    pub png: Vec<u8>,
}

/// The `.icns` file of `images`, one entry for each that has a slot.
pub(super) fn icns(images: &[Image]) -> Vec<u8> {
    let mut body = Vec::new();
    for (points, scale, kind, argb) in SLOTS {
        let Some(image) = images
            .iter()
            .find(|i| i.points == *points && i.scale == *scale)
        else {
            continue;
        };
        let data = if *argb {
            argb_entry(&image.rgba)
        } else {
            image.png.clone()
        };
        body.extend_from_slice(*kind);
        body.extend_from_slice(
            &u32::try_from(data.len() + 8)
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        body.extend_from_slice(&data);
    }
    let mut out = b"icns".to_vec();
    out.extend_from_slice(
        &u32::try_from(body.len() + 8)
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    out.extend_from_slice(&body);
    out
}

/// An `ARGB` entry: the alpha, red, green and blue planes, each run-length
/// encoded. Colors aren't premultiplied, but they lose what iconutil's trip
/// through premultiplied pixels loses.
fn argb_entry(image: &Rgba) -> Vec<u8> {
    let mut planes = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for pixel in image.pixels.chunks_exact(4) {
        let alpha = pixel[3];
        planes[0].push(alpha);
        for (plane, &value) in planes[1..].iter_mut().zip(&pixel[..3]) {
            plane.push(round_trip(value, alpha));
        }
    }
    let mut out = b"ARGB".to_vec();
    for plane in &planes {
        out.extend(run_length(plane));
    }
    out
}

/// A color channel premultiplied by `alpha` and back, rounding each way.
fn round_trip(value: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    let premultiplied = (u32::from(value) * u32::from(alpha) + 127) / 255;
    u8::try_from((premultiplied * 255 + u32::from(alpha) / 2) / u32::from(alpha)).unwrap_or(255)
}

/// icns run-length encoding: a byte below 0x80 is followed by that many
/// plus one literal bytes; a byte from 0x80 repeats the next byte that many
/// less 125 times (3 to 130).
fn run_length(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let run = data[i..]
            .iter()
            .take(130)
            .take_while(|&&b| b == data[i])
            .count();
        if run >= 3 {
            out.push(u8::try_from(run + 125).unwrap_or(255));
            out.push(data[i]);
            i += run;
            continue;
        }
        let start = i;
        while i < data.len() && i - start < 128 {
            if i + 2 < data.len() && data[i] == data[i + 1] && data[i] == data[i + 2] {
                break;
            }
            i += 1;
        }
        out.push(u8::try_from(i - start - 1).unwrap_or(0));
        out.extend_from_slice(&data[start..i]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_literals_encode_as_iconutil_does() {
        assert_eq!(run_length(&[7, 7, 7, 7]), [129, 7]);
        assert_eq!(run_length(&[1, 2, 3]), [2, 1, 2, 3]);
        assert_eq!(run_length(&[1, 2, 9, 9, 9]), [1, 1, 2, 128, 9]);
        let long = vec![5u8; 140];
        assert_eq!(run_length(&long), [255, 5, 135, 5]);
    }

    #[test]
    fn a_translucent_pixel_loses_what_iconutil_loses() {
        // iconutil writes (200, 100, 50) at alpha 128 as (199, 100, 50).
        assert_eq!([200, 100, 50].map(|v| round_trip(v, 128)), [199, 100, 50]);
        assert_eq!(round_trip(42, 255), 42);
        assert_eq!(round_trip(42, 0), 0);
    }

    #[test]
    fn entries_follow_actools_order_with_their_lengths() {
        let image = |points, scale| {
            let side = points * scale;
            Image {
                points,
                scale,
                rgba: Rgba {
                    width: side,
                    height: side,
                    pixels: vec![255; (side * side * 4) as usize],
                },
                png: vec![0x89, b'P', b'N', b'G'],
            }
        };
        let file = icns(&[image(16, 1), image(128, 1)]);
        assert_eq!(&file[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(file[4..8].try_into().unwrap()) as usize,
            file.len()
        );
        assert_eq!(&file[8..12], b"ic04");
        assert_eq!(&file[16..20], b"ARGB");
        let argb = u32::from_be_bytes(file[12..16].try_into().unwrap()) as usize;
        assert_eq!(&file[8 + argb..12 + argb], b"ic07");
        assert_eq!(
            u32::from_be_bytes(file[12 + argb..16 + argb].try_into().unwrap()),
            12
        );
    }
}
