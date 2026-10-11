//! A catalog's vector images, PDF and SVG, drawn into bitmaps the way actool
//! draws them: one per scale, sized from the drawing's own size in points.
//! PDFs go through hayro, SVGs through resvg.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::TRANSPARENT;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings};
use resvg::{tiny_skia, usvg};

use super::car::Bitmap;

/// A vector image's file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Svg,
}

impl Format {
    /// The format a file's extension names, if it's a vector one.
    pub fn of(filename: &str) -> Option<Format> {
        let extension = std::path::Path::new(filename).extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("pdf") {
            Some(Format::Pdf)
        } else if extension.eq_ignore_ascii_case("svg") {
            Some(Format::Svg)
        } else {
            None
        }
    }
}

/// A parsed vector image.
pub enum Drawing {
    /// A PDF's first page, the one actool draws.
    Pdf(Pdf),
    Svg(Box<usvg::Tree>),
}

impl Drawing {
    pub fn parse(format: Format, bytes: Vec<u8>) -> Result<Drawing, String> {
        match format {
            Format::Pdf => {
                let pdf = Pdf::new(bytes).map_err(|e| format!("{e:?}"))?;
                if pdf.pages().is_empty() {
                    return Err("the PDF has no pages".into());
                }
                Ok(Drawing::Pdf(pdf))
            }
            Format::Svg => svg(&bytes).map(|tree| Drawing::Svg(Box::new(tree))),
        }
    }

    /// Its size in points: a PDF page's crop box, which actool doesn't
    /// turn by the page's rotation, or an SVG's width and height.
    pub fn size(&self) -> (f32, f32) {
        match self {
            Drawing::Pdf(pdf) => pdf.pages()[0].base_dimensions(),
            Drawing::Svg(tree) => (tree.size().width(), tree.size().height()),
        }
    }

    /// The drawing at `scale` pixels per point, in a bitmap of its size
    /// times the scale in whole pixels: rounded for a PDF, rounded up for
    /// an SVG, as actool sizes them.
    pub fn render(&self, scale: f32) -> Result<Bitmap, String> {
        let (width, height) = self.size();
        let pixels = |points: f32| -> Result<u16, String> {
            let n = match self {
                Drawing::Pdf(_) => (points * scale).round(),
                // Less a little, so a float a hair past a whole number
                // doesn't round up a pixel.
                Drawing::Svg(_) => (points * scale - 0.001).ceil(),
            };
            if n.is_finite() && (1.0..=f32::from(u16::MAX)).contains(&n) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Ok(n as u16)
            } else {
                Err(format!("a size of {points} points can't be drawn"))
            }
        };
        let (w, h) = (pixels(width)?, pixels(height)?);
        match self {
            Drawing::Pdf(pdf) => Ok(render_pdf(pdf, scale, w, h)),
            Drawing::Svg(tree) => render_svg(tree, scale, w, h),
        }
    }
}

/// An SVG read the way CoreSVG reads one: 72 units to the inch, so a
/// point is a pixel, where browsers count 96.
pub fn svg(bytes: &[u8]) -> Result<usvg::Tree, String> {
    let options = usvg::Options {
        dpi: 72.0,
        ..usvg::Options::default()
    };
    usvg::Tree::from_data(bytes, &options).map_err(|e| e.to_string())
}

fn render_pdf(pdf: &Pdf, scale: f32, width: u16, height: u16) -> Bitmap {
    let page = &pdf.pages()[0];
    let mut context = RenderContext::new(width, height);
    // The crop box's corner at the origin, flipped so y runs down.
    let crop = page.intersected_crop_box();
    let s = f64::from(scale);
    let transform = hayro::kurbo::Affine::new([s, 0.0, 0.0, -s, -crop.x0 * s, crop.y1 * s]);
    hayro::render_into(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &mut context,
        transform,
    );
    context.flush();
    let mut pixmap = Pixmap::new(width, height);
    context.render_with(
        &mut pixmap,
        &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(TRANSPARENT),
            ..RasterizerSettings::default()
        },
    );
    Bitmap::from_premultiplied(
        u32::from(width),
        u32::from(height),
        pixmap.data_as_u8_slice(),
    )
}

fn render_svg(tree: &usvg::Tree, scale: f32, width: u16, height: u16) -> Result<Bitmap, String> {
    let mut pixmap = tiny_skia::Pixmap::new(u32::from(width), u32::from(height))
        .ok_or("the image is too large")?;
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(Bitmap::from_premultiplied(
        u32::from(width),
        u32::from(height),
        pixmap.data(),
    ))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A one-page PDF filled red, with a blue square 2 points in from its
    /// bottom-left corner.
    pub(in crate::cli::actool) fn pdf(
        media: [f32; 4],
        crop: Option<[f32; 4]>,
        rotate: u32,
    ) -> Vec<u8> {
        let content = "1 0 0 rg -1000 -1000 3000 3000 re f 0 0 1 rg 2 2 4 4 re f";
        let boxes =
            |name: &str, b: [f32; 4]| format!("/{name} [{} {} {} {}]", b[0], b[1], b[2], b[3]);
        let page = format!(
            "<< /Type /Page /Parent 2 0 R {} {} /Rotate {rotate} /Contents 4 0 R /Resources << >> >>",
            boxes("MediaBox", media),
            crop.map(|c| boxes("CropBox", c)).unwrap_or_default(),
        );
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            page,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn sizes(drawing: &Drawing) -> Vec<(u32, u32)> {
        [1.0, 2.0, 3.0]
            .map(|scale| {
                let bitmap = drawing.render(scale).unwrap();
                (bitmap.width, bitmap.height)
            })
            .to_vec()
    }

    #[test]
    fn a_pdf_is_as_many_pixels_as_its_crop_box_has_points_rounded() {
        let parse = |bytes| Drawing::parse(Format::Pdf, bytes).unwrap();
        // What Xcode 27's actool drew for each.
        let fractional = parse(pdf([0.0, 0.0, 20.5, 10.3], None, 0));
        assert_eq!(sizes(&fractional), [(21, 10), (41, 21), (62, 31)]);
        let cropped = parse(pdf(
            [0.0, 0.0, 100.0, 100.0],
            Some([10.0, 20.0, 40.0, 60.0]),
            0,
        ));
        assert_eq!(sizes(&cropped)[0], (30, 40));
        // actool doesn't turn a page by its rotation.
        let turned = parse(pdf([0.0, 0.0, 30.0, 10.0], None, 90));
        assert_eq!(sizes(&turned)[0], (30, 10));
    }

    #[test]
    fn a_pdf_is_drawn_from_its_crop_box_corner_up() {
        let drawing = Drawing::parse(Format::Pdf, pdf([5.0, 5.0, 25.0, 15.0], None, 0)).unwrap();
        let bitmap = drawing.render(2.0).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (40, 20));
        // Premultiplied BGRA: red, but for the blue square 2 points from the
        // origin, whose top right corner shows in the page's bottom left.
        let pixel = |x: usize, y: usize| bitmap.pixel(x, y);
        assert_eq!(pixel(20, 10), [0, 0, 255, 255]);
        assert_eq!(pixel(0, 19), [255, 0, 0, 255]);
    }

    #[test]
    fn an_svg_rounds_up_and_counts_72_units_an_inch() {
        let svg = |attributes: &str| {
            let text = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" {attributes}><rect width=\"100%\" height=\"100%\" fill=\"red\"/></svg>"
            );
            Drawing::parse(Format::Svg, text.into_bytes()).unwrap()
        };
        // What Xcode 27's actool drew for each.
        assert_eq!(
            sizes(&svg("width=\"20.5\" height=\"10.3\"")),
            [(21, 11), (41, 21), (62, 31)]
        );
        assert_eq!(
            sizes(&svg("viewBox=\"0 0 24.6 12.2\"")),
            [(25, 13), (50, 25), (74, 37)]
        );
        assert_eq!(sizes(&svg("width=\"1in\" height=\"20pt\""))[0], (72, 20));
        let bitmap = svg("width=\"4\" height=\"4\"").render(1.0).unwrap();
        assert_eq!(bitmap.pixel(1, 1), [0, 0, 255, 255]);
    }

    #[test]
    fn a_vector_image_is_known_by_its_extension() {
        assert_eq!(Format::of("logo.PDF"), Some(Format::Pdf));
        assert_eq!(Format::of("logo.svg"), Some(Format::Svg));
        assert_eq!(Format::of("logo.png"), None);
        assert!(Drawing::parse(Format::Pdf, b"%PDF-1.4".to_vec()).is_err());
    }
}
