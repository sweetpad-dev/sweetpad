//! A custom symbol's template, the SVG the SF Symbols app exports, read into
//! the masters actool compiles. A template draws a symbol at 100 points in
//! up to 27 groups, one per weight and size, like `Regular-M`, next to
//! guides: a baseline and cap line per size and margins beside the outline.
//! Each master becomes a standalone SVG cropped to its outline and scaled to
//! the platform's reference size, which is what CoreUI draws and
//! interpolates between, with its metrics measured against those guides.

use std::fmt::Write as _;
use std::mem::{Discriminant, discriminant};

use resvg::tiny_skia::PathSegment;
use resvg::usvg;
use usvg::roxmltree::{Document, Node};

use super::car::Glyph;

/// How a platform sizes symbols, as actool keeps them for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sizing {
    /// The point size a master is kept at.
    pub point_size: u32,
    /// Points per template unit at that size.
    pub scale: f64,
    /// The point sizes Xcode draws a symbol's bitmaps at, which the masters
    /// it draws them from list.
    pub sizes: &'static [u32],
    /// The glyph sizes of the regular masters it draws them from.
    pub drawn: &'static [u16],
}

impl Sizing {
    pub fn of(platform: &str) -> Sizing {
        match platform {
            "macosx" => Sizing {
                point_size: 13,
                scale: 0.13,
                sizes: &[11, 13, 17],
                drawn: &[2, 3],
            },
            // Watch faces set 17 points of SF Compact, whose caps are a
            // little lower than SF Pro's.
            "watchos" | "watchsimulator" => Sizing {
                point_size: 17,
                scale: 0.160_878,
                sizes: &[17],
                drawn: &[2],
            },
            _ => Sizing {
                point_size: 17,
                scale: 0.17,
                sizes: &[15, 17, 20],
                drawn: &[2],
            },
        }
    }
}

/// The space, in template units, that Xcode records after each margin.
const PADDING: f64 = 6.835_937_5;

const WEIGHTS: [&str; 9] = [
    "Ultralight",
    "Thin",
    "Light",
    "Regular",
    "Medium",
    "Semibold",
    "Bold",
    "Heavy",
    "Black",
];

const SIZES: [&str; 3] = ["S", "M", "L"];

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// The masters a template draws, in weight then size order.
pub fn masters(bytes: &[u8], sizing: Sizing) -> Result<Vec<Glyph>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    // Templates from Sketch and Illustrator open with a DTD.
    let options = usvg::roxmltree::ParsingOptions {
        allow_dtd: true,
        ..usvg::roxmltree::ParsingOptions::default()
    };
    let document = Document::parse_with_options(text, options).map_err(|e| e.to_string())?;
    let tree = super::vector::svg(bytes)?;
    let viewport = viewport(&document, &tree);
    let template_version = template_version(&document);
    let element = |id: &str| {
        document
            .descendants()
            .find(|n| n.is_element() && n.attribute("id") == Some(id))
    };
    let measured = |id: &str| -> Option<Outline> {
        let Some(usvg::Node::Group(group)) = tree.node_by_id(id) else {
            return None;
        };
        let to_template = viewport.invert()?;
        let (left, top, right, bottom) = control_bounds(group, to_template)?;
        Some(Outline {
            left,
            top,
            right,
            bottom,
            transform: to_template.pre_concat(group.abs_transform()),
        })
    };
    let regular_medium = measured("Regular-M");
    // A variable template draws an ultralight, a regular and a black master
    // that CoreUI interpolates into the other weights, which takes outlines
    // drawn with the same commands.
    let interpolatable = template_version >= 3.0
        && ["Ultralight-S", "Regular-S", "Black-S"]
            .map(|id| match tree.node_by_id(id) {
                Some(usvg::Node::Group(group)) => Some(commands(group)),
                _ => None,
            })
            .windows(2)
            .all(|pair| pair[0].is_some() && pair[0] == pair[1]);
    let mut glyphs = Vec::new();
    for (weight, weight_name) in (1..).zip(WEIGHTS) {
        for (size, size_name) in (1..).zip(SIZES) {
            let id = format!("{weight_name}-{size_name}");
            let (Some(outline), Some(group)) = (measured(&id), element(&id)) else {
                continue;
            };
            let guide = |name: &str| element(name).and_then(|n| Guide::of(n, &tree));
            let baseline = guide(&format!("Baseline-{size_name}")).map_or(outline.bottom, |g| g.y);
            let capline = guide(&format!("Capline-{size_name}")).map_or(baseline, |g| g.y);
            // A variable template has margins for each master; older ones
            // have one pair, drawn beside the regular medium master, or as
            // bars as wide as the margin.
            let own = |side: &str| guide(&format!("{side}-margin-{id}"));
            let (left, right, against) = match (own("left"), own("right")) {
                (Some(l), Some(r)) => (Some(l), Some(r), &outline),
                _ => (
                    guide("left-margin"),
                    guide("right-margin"),
                    regular_medium.as_ref().unwrap_or(&outline),
                ),
            };
            let left_margin = left.map_or(0.0, |g| g.width.unwrap_or(against.left - g.x));
            let right_margin = right.map_or(0.0, |g| g.width.unwrap_or(g.x - against.right));
            let points = |units: f64| points(units, sizing.scale);
            glyphs.push(Glyph {
                weight,
                size,
                svg: standalone(group, &outline, sizing.scale).into_bytes(),
                point_size: sizing.point_size,
                sizes: &[],
                baseline: points(outline.bottom - baseline),
                capline: points(baseline - capline),
                left_margin: points(left_margin),
                right_margin: points(right_margin),
                padding: points(PADDING),
                template_version,
                interpolatable,
            });
        }
    }
    if glyphs.is_empty() {
        return Err("the template draws no weight of the symbol".into());
    }
    // Xcode draws bitmaps from the regular masters of the sizes it draws
    // them at, or from the regular small one of a variable template that
    // has none of those, and those masters list the point sizes.
    let regular = |size: u16| glyphs.iter().position(|g| g.weight == 4 && g.size == size);
    let mut listing: Vec<usize> = sizing.drawn.iter().filter_map(|s| regular(*s)).collect();
    if listing.is_empty() && interpolatable {
        listing.extend(regular(1));
    }
    for i in listing {
        glyphs[i].sizes = sizing.sizes;
    }
    Ok(glyphs)
}

/// A master's outline: its bounds in template units, and the transform from
/// its group's coordinates into the template's.
struct Outline {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
    transform: usvg::Transform,
}

/// The kinds of segment a group's outlines are drawn with, path by path.
fn commands(group: &usvg::Group) -> Vec<Vec<Discriminant<PathSegment>>> {
    let mut out = Vec::new();
    for node in group.children() {
        match node {
            usvg::Node::Group(inner) => out.extend(commands(inner)),
            usvg::Node::Path(path) => {
                out.push(path.data().segments().map(|s| discriminant(&s)).collect());
            }
            _ => {}
        }
    }
    out
}

/// A path's bounds, control points and all, which is how Core Graphics
/// measures a path, with a stroked path's reaching half the stroke's width
/// further, in its own coordinates.
fn path_bounds(path: &usvg::Path) -> (f64, f64, f64, f64) {
    let r = path.data().bounds();
    let half = path
        .stroke()
        .map_or(0.0, |s| f64::from(s.width().get()) / 2.0);
    (
        f64::from(r.left()) - half,
        f64::from(r.top()) - half,
        f64::from(r.right()) + half,
        f64::from(r.bottom()) + half,
    )
}

/// The bounds of a group's paths in template units: each path's bounds
/// carried through its transform, which is how actool measures an outline.
fn control_bounds(
    group: &usvg::Group,
    to_template: usvg::Transform,
) -> Option<(f64, f64, f64, f64)> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for node in group.children() {
        let rect = match node {
            usvg::Node::Group(inner) => control_bounds(inner, to_template),
            usvg::Node::Path(path) => {
                let ts = to_template.pre_concat(path.abs_transform());
                let (l, t, r, b) = path_bounds(path);
                let corners = [(l, t), (r, t), (l, b), (r, b)].map(|(x, y)| {
                    (
                        f64::from(ts.sx) * x + f64::from(ts.kx) * y + f64::from(ts.tx),
                        f64::from(ts.ky) * x + f64::from(ts.sy) * y + f64::from(ts.ty),
                    )
                });
                let xs = corners.map(|c| c.0);
                let ys = corners.map(|c| c.1);
                Some((
                    xs.into_iter().fold(f64::INFINITY, f64::min),
                    ys.into_iter().fold(f64::INFINITY, f64::min),
                    xs.into_iter().fold(f64::NEG_INFINITY, f64::max),
                    ys.into_iter().fold(f64::NEG_INFINITY, f64::max),
                ))
            }
            _ => None,
        };
        if let Some((l, t, r, b)) = rect {
            bounds = Some(bounds.map_or((l, t, r, b), |(bl, bt, br, bb)| {
                (bl.min(l), bt.min(t), br.max(r), bb.max(b))
            }));
        }
    }
    bounds
}

/// A guide line, or a margin bar, in template units.
struct Guide {
    x: f64,
    y: f64,
    /// A bar's width, stroke and all, which is the margin it marks.
    width: Option<f64>,
}

impl Guide {
    fn of(node: Node, tree: &usvg::Tree) -> Option<Guide> {
        let number = |name: &str| -> f64 {
            node.attribute(name)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0.0)
        };
        let (x, y, width) = match node.tag_name().name() {
            "line" => (number("x1"), number("y1"), None),
            "rect" => (number("x"), number("y"), Some(number("width"))),
            _ => return None,
        };
        // A bar's stroke, which it may take from its group, is in its width.
        let stroke = node
            .attribute("id")
            .and_then(|id| match tree.node_by_id(id) {
                Some(usvg::Node::Path(path)) => path.stroke().map(|s| f64::from(s.width().get())),
                _ => None,
            })
            .unwrap_or(0.0);
        let ts = ancestor_transform(node);
        Some(Guide {
            x: ts.a * x + ts.c * y + ts.e,
            y: ts.b * x + ts.d * y + ts.f,
            width: width.map(|w| (w + stroke) * ts.a.hypot(ts.b)),
        })
    }
}

/// The transform from an element's coordinates into the template's: its own
/// `transform` and its ancestors'.
fn ancestor_transform(node: Node) -> svgtypes::Transform {
    let mut ts = svgtypes::Transform::default();
    for n in node.ancestors().filter(Node::is_element) {
        if n.tag_name().name() == "svg" {
            break;
        }
        let own: svgtypes::Transform = n
            .attribute("transform")
            .and_then(|t| t.parse().ok())
            .unwrap_or_default();
        ts = multiply(&own, &ts);
    }
    ts
}

/// `outer` applied after `inner`.
fn multiply(outer: &svgtypes::Transform, inner: &svgtypes::Transform) -> svgtypes::Transform {
    svgtypes::Transform::new(
        outer.a * inner.a + outer.c * inner.b,
        outer.b * inner.a + outer.d * inner.b,
        outer.a * inner.c + outer.c * inner.d,
        outer.b * inner.c + outer.d * inner.d,
        outer.a * inner.e + outer.c * inner.f + outer.e,
        outer.b * inner.e + outer.d * inner.f + outer.f,
    )
}

/// The transform usvg applies from template units to its canvas: none,
/// unless the template's `viewBox` differs from its size.
fn viewport(document: &Document, tree: &usvg::Tree) -> usvg::Transform {
    let root = document.root_element();
    let Some(view_box) = root
        .attribute("viewBox")
        .and_then(|v| v.parse::<svgtypes::ViewBox>().ok())
    else {
        return usvg::Transform::identity();
    };
    let size = tree.size();
    #[allow(clippy::cast_possible_truncation)] // usvg works in f32
    let (sx, sy) = (
        size.width() / view_box.w as f32,
        size.height() / view_box.h as f32,
    );
    let stretch = root
        .attribute("preserveAspectRatio")
        .is_some_and(|a| a.trim() == "none");
    let (sx, sy) = if stretch {
        (sx, sy)
    } else {
        (sx.min(sy), sx.min(sy))
    };
    #[allow(clippy::cast_possible_truncation)]
    let (x, y, w, h) = (
        view_box.x as f32,
        view_box.y as f32,
        view_box.w as f32,
        view_box.h as f32,
    );
    usvg::Transform::from_row(
        sx,
        0.0,
        0.0,
        sy,
        (size.width() - w * sx) / 2.0 - x * sx,
        (size.height() - h * sy) / 2.0 - y * sy,
    )
}

/// The template's version, from the note it carries, like "Template v.4.0".
fn template_version(document: &Document) -> f32 {
    let Some(note) = document
        .descendants()
        .find(|n| n.attribute("id") == Some("template-version"))
    else {
        return 0.0;
    };
    let text: String = note
        .descendants()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    text.split_once('v')
        .map(|(_, rest)| rest.trim_start_matches(['.', ' ']))
        .and_then(|rest| {
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            rest[..end].parse().ok()
        })
        .unwrap_or(0.0)
}

/// A length in template units, in points at the reference size.
#[allow(clippy::cast_possible_truncation)] // the format keeps 32 bits
fn points(units: f64, scale: f64) -> f32 {
    (units * scale) as f32
}

/// A master's group as an SVG of its own: its outline's bounds in points as
/// the view box, and the group's children drawn through a transform that
/// scales them to points and moves the outline to the origin.
fn standalone(group: Node, outline: &Outline, scale: f64) -> String {
    let t = outline.transform;
    let (sx, ky, kx, sy, tx, ty) = (
        f64::from(t.sx),
        f64::from(t.ky),
        f64::from(t.kx),
        f64::from(t.sy),
        f64::from(t.tx),
        f64::from(t.ty),
    );
    let matrix = [
        sx * scale,
        ky * scale,
        kx * scale,
        sy * scale,
        (tx - outline.left) * scale,
        (ty - outline.top) * scale,
    ];
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg version=\"1.1\" \
         xmlns=\"{SVG_NAMESPACE}\" viewBox=\"0 0 {} {}\"><g id=\"s\" \
         transform=\"matrix({})\">",
        number((outline.right - outline.left) * scale),
        number((outline.bottom - outline.top) * scale),
        matrix.map(number).join(" "),
    );
    for child in group.children() {
        write_node(child, &mut out, 0);
    }
    out.push_str("</g></svg>");
    out
}

/// A number the way Xcode writes one, like C's `%g`: six significant
/// digits, and an exponent when it's very small or very large.
fn number(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".into();
    }
    #[allow(clippy::cast_possible_truncation)] // a few hundred at most
    let exponent = value.abs().log10().floor() as i32;
    let trim = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            text.to_string()
        }
    };
    if !(-4..6).contains(&exponent) {
        let text = format!("{value:.5e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        return format!("{}e{exponent}", trim(mantissa));
    }
    let digits = usize::try_from(5 - exponent).unwrap_or(0);
    let text = trim(&format!("{value:.digits$}"));
    if text == "-0" { "0".into() } else { text }
}

/// An SVG element and what's under it, the way actool copies a master: a
/// `use` becomes a group around the element it points at, which the copy
/// has no other way to reach, and masks, clip paths and filters, which
/// point at elements the copy leaves out, are dropped, as are the editors'
/// own elements and attributes, like Inkscape's.
fn write_node(node: Node, out: &mut String, depth: usize) {
    const NOT_DRAWN: [&str; 7] = [
        "mask", "clipPath", "defs", "symbol", "title", "desc", "metadata",
    ];
    const REFERENCES: [&str; 3] = ["mask", "clip-path", "filter"];
    if node.is_text() {
        out.push_str(&escape(node.text().unwrap_or_default()));
        return;
    }
    if depth > 32
        || !node.is_element()
        || node.tag_name().namespace() != Some(SVG_NAMESPACE)
        || NOT_DRAWN.contains(&node.tag_name().name())
    {
        return;
    }
    let name = node.tag_name().name();
    let attributes = node
        .attributes()
        .filter(|a| a.namespace().is_none() && !REFERENCES.contains(&a.name()));
    if name == "use" {
        let target = node
            .attribute(("http://www.w3.org/1999/xlink", "href"))
            .or_else(|| node.attribute("href"))
            .and_then(|href| href.strip_prefix('#'))
            .and_then(|id| {
                node.document()
                    .descendants()
                    .find(|n| n.is_element() && n.attribute("id") == Some(id))
            });
        let Some(target) = target else { return };
        out.push_str("<g");
        let mut transform = node.attribute("transform").unwrap_or_default().to_string();
        let offset = |name: &str| {
            node.attribute(name)
                .and_then(|v| v.trim().parse::<f64>().ok())
        };
        let (x, y) = (offset("x"), offset("y"));
        if x.is_some() || y.is_some() {
            let _ = write!(
                transform,
                " translate({} {})",
                number(x.unwrap_or(0.0)),
                number(y.unwrap_or(0.0))
            );
        }
        for attribute in attributes.filter(|a| {
            !matches!(
                a.name(),
                "x" | "y" | "width" | "height" | "href" | "transform" | "id"
            )
        }) {
            let _ = write!(
                out,
                " {}=\"{}\"",
                attribute.name(),
                escape(attribute.value())
            );
        }
        if !transform.trim().is_empty() {
            let _ = write!(out, " transform=\"{}\"", escape(transform.trim()));
        }
        out.push('>');
        write_node(target, out, depth + 1);
        out.push_str("</g>");
        return;
    }
    out.push('<');
    out.push_str(name);
    for attribute in attributes {
        let value = match (name, attribute.name()) {
            ("path", "d") => path_data(attribute.value()),
            _ => None,
        };
        let _ = write!(
            out,
            " {}=\"{}\"",
            attribute.name(),
            escape(value.as_deref().unwrap_or(attribute.value()))
        );
    }
    if node.has_children() {
        out.push('>');
        for child in node.children() {
            write_node(child, out, depth + 1);
        }
        let _ = write!(out, "</{name}>");
    } else {
        out.push_str("/>");
    }
}

/// Path data with its numbers to six significant digits, as CoreSVG writes
/// it back for actool, so the outline CoreUI draws is the one Xcode's would
/// be. None when the data doesn't parse, to keep it as it is.
fn path_data(d: &str) -> Option<String> {
    use svgtypes::PathSegment as S;
    let mut out = String::new();
    let mut previous = None;
    for segment in svgtypes::PathParser::from(d) {
        let (letter, numbers): (char, Vec<f64>) = match segment.ok()? {
            S::MoveTo { abs, x, y } => (if abs { 'M' } else { 'm' }, vec![x, y]),
            S::LineTo { abs, x, y } => (if abs { 'L' } else { 'l' }, vec![x, y]),
            S::HorizontalLineTo { abs, x } => (if abs { 'H' } else { 'h' }, vec![x]),
            S::VerticalLineTo { abs, y } => (if abs { 'V' } else { 'v' }, vec![y]),
            S::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => (if abs { 'C' } else { 'c' }, vec![x1, y1, x2, y2, x, y]),
            S::SmoothCurveTo { abs, x2, y2, x, y } => {
                (if abs { 'S' } else { 's' }, vec![x2, y2, x, y])
            }
            S::Quadratic { abs, x1, y1, x, y } => (if abs { 'Q' } else { 'q' }, vec![x1, y1, x, y]),
            S::SmoothQuadratic { abs, x, y } => (if abs { 'T' } else { 't' }, vec![x, y]),
            S::EllipticalArc {
                abs,
                rx,
                ry,
                x_axis_rotation,
                large_arc,
                sweep,
                x,
                y,
            } => (
                if abs { 'A' } else { 'a' },
                vec![
                    rx,
                    ry,
                    x_axis_rotation,
                    f64::from(u8::from(large_arc)),
                    f64::from(u8::from(sweep)),
                    x,
                    y,
                ],
            ),
            S::ClosePath { abs } => (if abs { 'Z' } else { 'z' }, Vec::new()),
        };
        if previous != Some(letter) || numbers.is_empty() {
            out.push(letter);
        }
        for (i, value) in numbers.into_iter().enumerate() {
            let text = number(value);
            // A minus sign is separator enough, as CoreSVG writes it.
            if (i > 0 || previous == Some(letter)) && !text.starts_with('-') {
                out.push(' ');
            }
            out.push_str(&text);
        }
        previous = Some(letter);
    }
    Some(out)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/actool/symbols")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// A master as Xcode 27's actool wrote it: weight, size, baseline,
    /// cap line, margins, listed sizes, and the view box and transform of
    /// its SVG.
    struct Expected {
        weight: u16,
        size: u16,
        baseline: f32,
        capline: f32,
        margins: (f32, f32),
        sizes: &'static [u32],
        view_box: [f64; 2],
        matrix: [f64; 6],
    }

    fn numbers(text: &str) -> Vec<f64> {
        text.split([' ', ','])
            .filter(|t| !t.is_empty())
            .map(|t| t.parse().unwrap())
            .collect()
    }

    fn check(glyphs: &[Glyph], expected: &[Expected]) {
        assert_eq!(glyphs.len(), expected.len());
        for (glyph, e) in glyphs.iter().zip(expected) {
            let what = format!("{}-{}", glyph.weight, glyph.size);
            assert_eq!((glyph.weight, glyph.size), (e.weight, e.size));
            let close = |a: f32, b: f32| (a - b).abs() < 2e-3;
            assert!(
                close(glyph.baseline, e.baseline),
                "{what} baseline {}",
                glyph.baseline
            );
            assert!(
                close(glyph.capline, e.capline),
                "{what} capline {}",
                glyph.capline
            );
            assert!(
                close(glyph.left_margin, e.margins.0),
                "{what} left {}",
                glyph.left_margin
            );
            assert!(
                close(glyph.right_margin, e.margins.1),
                "{what} right {}",
                glyph.right_margin
            );
            assert_eq!(glyph.sizes, e.sizes, "{what} sizes");
            let svg = String::from_utf8(glyph.svg.clone()).unwrap();
            let attribute = |name: &str| {
                let start = svg.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
                svg[start..start + svg[start..].find('"').unwrap()].to_string()
            };
            let view_box = numbers(&attribute("viewBox"));
            let matrix = numbers(
                attribute("transform")
                    .trim_start_matches("matrix(")
                    .trim_end_matches(')'),
            );
            for (a, b) in view_box[2..]
                .iter()
                .zip(e.view_box)
                .chain(matrix.iter().zip(e.matrix))
            {
                assert!((a - b).abs() < 2e-3, "{what}: {svg}");
            }
        }
    }

    #[test]
    fn a_variable_templates_masters_measure_as_xcode_measures_them() {
        let glyphs = masters(&fixture("variable.svg"), Sizing::of("iphonesimulator")).unwrap();
        // Its three masters are drawn differently, so CoreUI can't
        // interpolate them, and Xcode draws no bitmaps of them.
        assert!(
            glyphs
                .iter()
                .all(|g| !g.interpolatable && (g.template_version - 4.0).abs() < 1e-6)
        );
        check(
            &glyphs,
            &[
                Expected {
                    weight: 1,
                    size: 1,
                    baseline: 0.34,
                    capline: 11.978,
                    margins: (0.85, 0.935),
                    sizes: &[],
                    view_box: [13.94, 12.07],
                    matrix: [0.17, 0.0, 0.0, 0.17, -85.85, -106.59],
                },
                // Its outline is measured by its control points.
                Expected {
                    weight: 4,
                    size: 1,
                    baseline: 0.68,
                    capline: 11.978,
                    margins: (1.4875, 1.36),
                    sizes: &[],
                    view_box: [13.6, 10.2],
                    matrix: [0.17, 0.0, 0.0, 0.17, -239.7, -108.8],
                },
                // Its group is scaled, and its outline stroked.
                Expected {
                    weight: 9,
                    size: 1,
                    baseline: 1.1832,
                    capline: 11.978,
                    margins: (4.3792, -1.7238),
                    sizes: &[],
                    view_box: [16.2996, 13.6612],
                    matrix: [0.1734, 0.0, 0.0, 0.1666, -500.779, -103.292],
                },
            ],
        );
        let regular = String::from_utf8(glyphs[1].svg.clone()).unwrap();
        assert!(!regular.contains("inkscape") && !regular.contains("connector-curvature"));
        assert!(regular.contains(r#"fill-rule="evenodd""#), "{regular}");
    }

    #[test]
    fn masters_drawn_alike_are_interpolated() {
        let text = String::from_utf8(fixture("variable.svg")).unwrap().replace(
            "M1410 700 C1410 640 1490 640 1490 700 Z M1430 690 h40 v-20 h-40 z",
            "M1410 700 L1490 700 L1450 630 Z",
        );
        let glyphs = masters(text.as_bytes(), Sizing::of("iphonesimulator")).unwrap();
        assert!(glyphs.iter().all(|g| g.interpolatable));
        // Xcode draws bitmaps from the regular small master then.
        let listing: Vec<(u16, &[u32])> = glyphs.iter().map(|g| (g.weight, g.sizes)).collect();
        assert_eq!(
            listing,
            [(1, &[][..]), (4, &[15, 17, 20][..]), (9, &[][..])]
        );
    }

    #[test]
    fn an_older_templates_masters_measure_as_xcode_measures_them() {
        let glyphs = masters(&fixture("legacy.svg"), Sizing::of("iphonesimulator")).unwrap();
        assert!(
            glyphs
                .iter()
                .all(|g| !g.interpolatable && g.template_version.abs() < 1e-6)
        );
        // The margin bars are as wide as their stroke too.
        check(
            &glyphs,
            &[
                Expected {
                    weight: 4,
                    size: 1,
                    baseline: 0.0,
                    capline: 11.978,
                    margins: (1.5839, 1.5839),
                    sizes: &[],
                    view_box: [10.2, 11.9],
                    matrix: [0.17, 0.0, 0.0, 0.17, 0.0, 0.0],
                },
                Expected {
                    weight: 4,
                    size: 2,
                    baseline: 1.36,
                    capline: 11.978,
                    margins: (1.5839, 1.5839),
                    sizes: &[15, 17, 20],
                    view_box: [12.24, 14.28],
                    matrix: [0.17, 0.0, 0.0, 0.17, 0.0, 0.0],
                },
                Expected {
                    weight: 4,
                    size: 3,
                    baseline: -2.21,
                    capline: 11.978,
                    margins: (1.5839, 1.5839),
                    sizes: &[],
                    view_box: [10.2, 11.9],
                    matrix: [0.17, 0.0, 0.0, 0.17, -0.34, -0.51],
                },
            ],
        );
        // A use becomes the outline it points at; its mask stays out.
        let small = String::from_utf8(glyphs[0].svg.clone()).unwrap();
        assert!(small.contains("<path") && !small.contains("<use") && !small.contains("mask"));
    }

    #[test]
    fn macos_keeps_masters_at_13_points() {
        let glyphs = masters(&fixture("legacy.svg"), Sizing::of("macosx")).unwrap();
        assert!(glyphs.iter().all(|g| g.point_size == 13));
        assert!((glyphs[0].capline - 9.1597).abs() < 1e-3);
        assert!((glyphs[0].padding - 0.888_671_9).abs() < 1e-6);
        // Xcode draws a Mac's bitmaps at the large size too.
        let listing: Vec<&[u32]> = glyphs.iter().map(|g| g.sizes).collect();
        assert_eq!(listing, [&[][..], &[11, 13, 17], &[11, 13, 17]]);
    }

    #[test]
    fn numbers_are_written_as_c_writes_them_with_g() {
        let written: Vec<String> = [
            0.17,
            -239.6999,
            1474.7712,
            0.000_012_5,
            -3.009_88e-13,
            2.0,
            1e7,
            -0.0,
        ]
        .map(number)
        .to_vec();
        assert_eq!(
            written,
            [
                "0.17",
                "-239.7",
                "1474.77",
                "1.25e-5",
                "-3.00988e-13",
                "2",
                "1e7",
                "0"
            ]
        );
        assert_eq!(
            path_data("M1410.5,700 c0-60.123456 80,-60 80,0 Z").unwrap(),
            "M1410.5 700c0-60.1235 80-60 80 0Z"
        );
    }
}
