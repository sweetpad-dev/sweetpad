//! Icon Composer source layers compiled as portable static appearances.
//! CoreUI can use these icon renditions on older OS versions too. Interactive
//! glass lighting is deliberately reported as a remaining rendering limit.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::car::{Appearance, Bitmap, Car, Part, Variant};
use super::catalog::Catalog;
use super::raster::{self, Rgba};
use super::vector::{Drawing, Format};
use super::write;

const SIZE: u32 = 1024;

pub(super) fn compile<'a>(
    catalogs: &'a [Catalog],
    primary: Option<&str>,
    platform: &str,
    output: &Path,
    written: &mut Vec<PathBuf>,
    car: &mut Car,
    compiled: &mut HashSet<&'a Path>,
) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for asset in catalogs
        .iter()
        .flat_map(Catalog::assets)
        .map(|(_, a)| a)
        .filter(|a| a.dir.extension().is_some_and(|e| e == "icon"))
    {
        if !matches!(platform, "iphoneos" | "iphonesimulator") {
            continue;
        }
        let json = asset.dir.join("icon.json");
        let data: Value = serde_json::from_slice(
            &std::fs::read(&json).map_err(|e| format!("{}: {e}", json.display()))?,
        )
        .map_err(|e| format!("{}: {e}", json.display()))?;
        let identifier = car
            .facet(&asset.name, Part::Icon)
            .ok_or("too many icon assets")?;
        for (appearance, label) in [
            (Appearance::Any, "light"),
            (Appearance::Dark, "dark"),
            (Appearance::Tinted, "tinted"),
        ] {
            let image =
                render(&data, &asset.dir, label).map_err(|e| format!("{}: {e}", json.display()))?;
            let bitmap = Bitmap::from_rgba(&image);
            // Native actool retains both idioms even for an iPhone-only target.
            for idiom in [1, 2] {
                car.icon(
                    &format!("{}-{label}.png", asset.name),
                    identifier,
                    Variant {
                        appearance,
                        scale: 1,
                        idiom,
                    },
                    0,
                    1,
                    &bitmap,
                );
                if appearance == Appearance::Any {
                    car.multisized(&asset.name, identifier, idiom, 0, &[(SIZE, 1)]);
                }
            }
            if appearance == Appearance::Any && primary == Some(asset.name.as_str()) {
                for (suffix, pixels) in [("60x60@2x", 120), ("76x76@2x~ipad", 152)] {
                    let path = output.join(format!("{}{suffix}.png", asset.name));
                    write(
                        &path,
                        &raster::encode(&raster::resize(&image, pixels, pixels))?,
                    )?;
                    written.push(path);
                }
            }
        }
        names.insert(asset.name.clone());
        compiled.insert(asset.dir.as_path());
    }
    if !names.is_empty() {
        println!(
            "/* com.apple.actool.document.warnings */\nwarning: Icon Composer static light, dark and tinted appearances were compiled; interactive glass lighting is not reproduced."
        );
    }
    Ok(names)
}

fn specialized<'a>(node: &'a Value, key: &str, appearance: &str) -> Option<&'a Value> {
    let variants = node.get(format!("{key}-specializations"))?.as_array()?;
    variants
        .iter()
        .rev()
        .find(|v| v["appearance"].as_str() == Some(appearance))
        .or_else(|| {
            variants
                .iter()
                .rev()
                .find(|v| v.get("appearance").is_none())
        })
        .and_then(|v| v.get("value"))
}

fn property<'a>(node: &'a Value, key: &str, appearance: &str) -> Option<&'a Value> {
    specialized(node, key, appearance).or_else(|| node.get(key))
}

fn color(text: &str) -> Result<[f64; 4], String> {
    let (space, components) = text
        .split_once(':')
        .ok_or_else(|| format!("invalid icon color {text:?}"))?;
    let values: Vec<f64> = components
        .split(',')
        .map(|n| n.parse::<f64>().map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    if values.iter().any(|v| !v.is_finite()) {
        return Err("non-finite icon color".into());
    }
    match (space, values.as_slice()) {
        ("srgb" | "extended-srgb", [r, g, b, a]) => Ok([*r, *g, *b, *a]),
        ("gray" | "extended-gray", [g, a]) => Ok([*g, *g, *g, *a]),
        ("display-p3", [r, g, b, a]) => {
            let linear = |c: f64| {
                if c <= 0.040_45 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            let gamma = |c: f64| {
                if c <= 0.003_130_8 {
                    c * 12.92
                } else {
                    1.055 * c.powf(1.0 / 2.4) - 0.055
                }
            };
            let (r, g, b) = (linear(*r), linear(*g), linear(*b));
            Ok([
                gamma(1.224_745 * r - 0.224_904 * g),
                gamma(-0.042_058 * r + 1.042_081 * g),
                gamma(-0.019_642 * r - 0.078_655 * g + 1.098_537 * b),
                *a,
            ])
        }
        _ => Err(format!("unsupported icon color {text:?}")),
    }
}

fn fill(value: Option<&Value>, y: f64, appearance: &str) -> Result<[f64; 4], String> {
    let Some(value) = value else {
        return Ok([0.0; 4]);
    };
    if let Some(kind) = value.as_str() {
        // Static system backgrounds omit the live material's bevel/lighting.
        let gray = match kind {
            "system-light" => 1.0 - 0.08 * y,
            "system-dark" => 0.20 - 0.13 * y,
            "none" => return Ok([0.0; 4]),
            other => return Err(format!("unsupported Icon Composer fill {other:?}")),
        };
        return Ok([gray, gray, gray, 1.0]);
    }
    if let Some(solid) = value["solid"].as_str() {
        return color(solid);
    }
    if let Some(gradient) = value["linear-gradient"].as_array() {
        if gradient.len() != 2 {
            return Err("an icon gradient needs two colors".into());
        }
        let a = color(gradient[0].as_str().ok_or("invalid gradient color")?)?;
        let b = color(gradient[1].as_str().ok_or("invalid gradient color")?)?;
        return Ok(std::array::from_fn(|i| a[i] * (1.0 - y) + b[i] * y));
    }
    if let Some(automatic) = value["automatic-gradient"].as_str() {
        let mut c = color(automatic)?;
        // Static fallback; the live material's lighting remains an explicit limit.
        if appearance == "dark" {
            c[..3].fill(0.08 * (1.0 - y));
        }
        return Ok(c);
    }
    Err("unsupported Icon Composer fill".into())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // bounded 8-bit channels
fn byte(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn position(node: &Value) -> (f64, [f64; 2]) {
    let p = &node["position"];
    (
        p["scale"].as_f64().unwrap_or(1.0),
        [
            p["translation-in-points"][0].as_f64().unwrap_or(0.0),
            p["translation-in-points"][1].as_f64().unwrap_or(0.0),
        ],
    )
}

fn blend(dst: &mut [u8], src: &[u8], opacity: f64, mode: &str) -> Result<(), String> {
    let sa = f64::from(src[3]) / 255.0 * opacity;
    let da = f64::from(dst[3]) / 255.0;
    let a = sa + da * (1.0 - sa);
    if a == 0.0 {
        return Ok(());
    }
    for i in 0..3 {
        let (s, d) = (f64::from(src[i]) / 255.0, f64::from(dst[i]) / 255.0);
        let b = match mode {
            "normal" => s,
            "multiply" => s * d,
            "screen" => s + d - s * d,
            "lighten" => s.max(d),
            "plus-darker" => (s + d - 1.0).max(0.0),
            other => return Err(format!("unsupported icon blend mode {other:?}")),
        };
        dst[i] = byte(((1.0 - sa) * da * d + (1.0 - da) * sa * s + sa * da * b) / a);
    }
    dst[3] = byte(a);
    Ok(())
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // sizes checked before allocation
fn image(path: &Path, scale: f64) -> Result<Rgba, String> {
    if !scale.is_finite() || scale <= 0.0 || scale > 100.0 {
        return Err("invalid icon image scale".into());
    }
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(format) = path.to_str().and_then(Format::of) {
        let drawing = Drawing::parse(format, data)?;
        let size = drawing.size();
        if f64::from(size.0.max(size.1)) * scale > 8192.0 {
            return Err("icon layer exceeds 8192 pixels".into());
        }
        drawing.render(scale as f32).map(|b| b.rgba())
    } else {
        let image = raster::decode(&data)?;
        let (w, h) = (
            f64::from(image.width) * scale,
            f64::from(image.height) * scale,
        );
        if !(1.0..=8192.0).contains(&w) || !(1.0..=8192.0).contains(&h) {
            return Err("invalid icon layer size".into());
        }
        Ok(raster::resize(&image, w.round() as u32, h.round() as u32))
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clipped canvas coordinates
fn layer(
    canvas: &mut Rgba,
    node: &Value,
    group: &Value,
    dir: &Path,
    appearance: &str,
) -> Result<(), String> {
    if property(node, "hidden", appearance).and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let name = node["image-name"]
        .as_str()
        .ok_or("an icon layer needs an image-name")?;
    let (gs, gt) = position(group);
    let (ls, lt) = position(node);
    if gt
        .iter()
        .chain(&lt)
        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0)
    {
        return Err("invalid icon layer translation".into());
    }
    let mut image = image(&dir.join("Assets").join(name), gs * ls)?;
    if let Some(fill_value) = property(node, "fill", appearance)
        .filter(|fill| !matches!(fill.as_str(), Some("none" | "automatic")))
    {
        for y in 0..image.height {
            let color = fill(
                Some(fill_value),
                f64::from(y) / f64::from(image.height),
                appearance,
            )?;
            for x in 0..image.width {
                let at = ((y * image.width + x) * 4) as usize;
                for (i, c) in color[..3].iter().enumerate() {
                    image.pixels[at + i] = byte(*c);
                }
                image.pixels[at + 3] = byte(f64::from(image.pixels[at + 3]) / 255.0 * color[3]);
            }
        }
    }
    let left =
        ((f64::from(SIZE) - f64::from(image.width)) / 2.0 + gt[0] + lt[0] * gs).round() as i64;
    let top =
        ((f64::from(SIZE) - f64::from(image.height)) / 2.0 + gt[1] + lt[1] * gs).round() as i64;
    let mode = property(node, "blend-mode", appearance)
        .and_then(Value::as_str)
        .unwrap_or("normal");
    let opacity = property(node, "opacity", appearance)
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    for y in 0..image.height {
        for x in 0..image.width {
            let (cx, cy) = (left + i64::from(x), top + i64::from(y));
            if cx < 0 || cy < 0 || cx >= i64::from(SIZE) || cy >= i64::from(SIZE) {
                continue;
            }
            let to = ((cy as u32 * SIZE + cx as u32) * 4) as usize;
            let from = ((y * image.width + x) * 4) as usize;
            blend(
                &mut canvas.pixels[to..to + 4],
                &image.pixels[from..from + 4],
                opacity,
                mode,
            )?;
        }
    }
    Ok(())
}

fn render(data: &Value, dir: &Path, appearance: &str) -> Result<Rgba, String> {
    let mut canvas = Rgba {
        width: SIZE,
        height: SIZE,
        pixels: vec![0; (SIZE * SIZE * 4) as usize],
    };
    for y in 0..SIZE {
        let color = fill(
            property(data, "fill", appearance),
            f64::from(y) / f64::from(SIZE),
            appearance,
        )?;
        for x in 0..SIZE {
            let at = ((y * SIZE + x) * 4) as usize;
            for (i, c) in color.iter().enumerate() {
                canvas.pixels[at + i] = byte(*c);
            }
        }
    }
    // Icon Composer stores the editor's frontmost group/layer first.
    for group in data["groups"]
        .as_array()
        .ok_or("an icon needs a groups array")?
        .iter()
        .rev()
    {
        if property(group, "hidden", appearance).and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let mut group_canvas = Rgba {
            width: SIZE,
            height: SIZE,
            pixels: vec![0; canvas.pixels.len()],
        };
        for node in group["layers"]
            .as_array()
            .ok_or("an icon group needs layers")?
            .iter()
            .rev()
        {
            layer(&mut group_canvas, node, group, dir, appearance)?;
        }
        let mode = property(group, "blend-mode", appearance)
            .and_then(Value::as_str)
            .unwrap_or("normal");
        let opacity = property(group, "opacity", appearance)
            .and_then(Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        for (dst, src) in canvas
            .pixels
            .chunks_exact_mut(4)
            .zip(group_canvas.pixels.chunks_exact(4))
        {
            blend(dst, src, opacity, mode)?;
        }
    }
    if appearance == "tinted" {
        for pixel in canvas.pixels.chunks_exact_mut(4) {
            let luminance = (0.2126 * f64::from(pixel[0])
                + 0.7152 * f64::from(pixel[1])
                + 0.0722 * f64::from(pixel[2]))
                / 255.0;
            pixel[..3].fill(byte(luminance));
        }
    }
    Ok(canvas)
}
