//! PNG in and out, and the downscale that makes an app icon's sizes out of
//! its 1024-point original.

use std::io::Cursor;

/// An image as 8-bit RGBA, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("an indexed image wasn't expanded".into()),
    };
    let mut pixels = Vec::with_capacity(frame.width as usize * frame.height as usize * 4);
    for row in buffer.chunks(frame.line_size).take(frame.height as usize) {
        for px in row[..frame.width as usize * channels].chunks(channels) {
            pixels.extend_from_slice(&match *px {
                [g] => [g, g, g, 255],
                [g, a] => [g, g, g, a],
                [r, g, b] => [r, g, b, 255],
                [r, g, b, a] => [r, g, b, a],
                _ => unreachable!("chunks of {channels}"),
            });
        }
    }
    Ok(Rgba {
        width: frame.width,
        height: frame.height,
        pixels,
    })
}

/// The image as a PNG: RGB when every pixel is opaque, RGBA otherwise.
pub fn encode(image: &Rgba) -> Result<Vec<u8>, String> {
    let opaque = image.pixels.chunks(4).all(|px| px[3] == 255);
    let (color, data) = if opaque {
        let rgb: Vec<u8> = image
            .pixels
            .chunks(4)
            .flat_map(|px| [px[0], px[1], px[2]])
            .collect();
        (png::ColorType::Rgb, rgb)
    } else {
        (png::ColorType::Rgba, image.pixels.clone())
    };
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&data).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(out)
}

/// The image at `width` by `height`, each pixel the average of the source
/// area it covers. Colors are averaged weighted by their alpha, so a
/// transparent neighbor doesn't darken an edge.
pub fn resize(image: &Rgba, width: u32, height: u32) -> Rgba {
    let columns = coverage(image.width, width);
    let rows = coverage(image.height, height);
    let src_width = image.width as usize;
    let premultiplied: Vec<[f64; 4]> = image
        .pixels
        .chunks(4)
        .map(|px| {
            let a = f64::from(px[3]) / 255.0;
            [
                f64::from(px[0]) * a,
                f64::from(px[1]) * a,
                f64::from(px[2]) * a,
                a,
            ]
        })
        .collect();
    // Across first, then down.
    let mut across = vec![[0.0f64; 4]; width as usize * image.height as usize];
    for y in 0..image.height as usize {
        for (x, weights) in columns.iter().enumerate() {
            let mut sum = [0.0f64; 4];
            for &(sx, w) in weights {
                let px = premultiplied[y * src_width + sx];
                for c in 0..4 {
                    sum[c] += px[c] * w;
                }
            }
            across[y * width as usize + x] = sum;
        }
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for weights in &rows {
        for x in 0..width as usize {
            let mut sum = [0.0f64; 4];
            for &(sy, w) in weights {
                let px = across[sy * width as usize + x];
                for c in 0..4 {
                    sum[c] += px[c] * w;
                }
            }
            let a = sum[3];
            if a > 0.0 {
                pixels.extend_from_slice(&[
                    byte(sum[0] / a),
                    byte(sum[1] / a),
                    byte(sum[2] / a),
                    byte(a * 255.0),
                ]);
            } else {
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Rgba {
        width,
        height,
        pixels,
    }
}

/// For each of `dst` output pixels along one axis, the source pixels it
/// covers and how much of it each one is.
fn coverage(src: u32, dst: u32) -> Vec<Vec<(usize, f64)>> {
    let scale = f64::from(src) / f64::from(dst);
    (0..dst)
        .map(|o| {
            let start = f64::from(o) * scale;
            let end = start + scale;
            let mut weights = Vec::new();
            let mut i = floor_index(start);
            while i < src && f64::from(i) < end {
                let overlap = end.min(f64::from(i) + 1.0) - start.max(f64::from(i));
                if overlap > 0.0 {
                    weights.push((i as usize, overlap / scale));
                }
                i += 1;
            }
            weights
        })
        .collect()
}

/// A channel value rounded to a byte.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0..=255 first
fn byte(value: f64) -> u8 {
    (value.clamp(0.0, 255.0) + 0.5) as u8
}

/// The pixel a coordinate falls in.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // coordinates are within a u32 image
fn floor_index(coordinate: f64) -> u32 {
    coordinate.floor().max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, px: [u8; 4]) -> Rgba {
        Rgba {
            width,
            height,
            pixels: px.repeat((width * height) as usize),
        }
    }

    #[test]
    fn a_png_round_trips() {
        let mut image = solid(3, 2, [10, 20, 30, 255]);
        image.pixels[4..8].copy_from_slice(&[200, 100, 50, 128]);
        assert_eq!(decode(&encode(&image).unwrap()).unwrap(), image);
        let opaque = solid(4, 4, [1, 2, 3, 255]);
        assert_eq!(decode(&encode(&opaque).unwrap()).unwrap(), opaque);
    }

    #[test]
    fn halving_averages_each_square() {
        let mut image = solid(4, 2, [0, 0, 0, 255]);
        // The left 2x2 square is half white.
        for i in [0usize, 4] {
            image.pixels[i * 4..i * 4 + 3].copy_from_slice(&[255, 255, 255]);
        }
        let half = resize(&image, 2, 1);
        assert_eq!(half.pixels, vec![128, 128, 128, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn a_transparent_neighbor_keeps_the_color() {
        let mut image = solid(2, 1, [0, 0, 0, 0]);
        image.pixels[0..4].copy_from_slice(&[255, 0, 0, 255]);
        let one = resize(&image, 1, 1);
        assert_eq!(one.pixels, vec![255, 0, 0, 128]);
    }

    #[test]
    fn an_icon_shrinks_to_any_size() {
        let icon = resize(&solid(1024, 1024, [240, 120, 30, 255]), 152, 152);
        assert_eq!((icon.width, icon.height), (152, 152));
        assert!(icon.pixels.chunks(4).all(|px| px == [240, 120, 30, 255]));
    }
}
