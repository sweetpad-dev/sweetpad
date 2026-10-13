//! Apple's compiled asset catalog, `Assets.car`, with the named colors,
//! images and symbols that UIKit's and SwiftUI's lookups by name read.
//!
//! The file is a BOM store, NeXTSTEP's bill-of-materials format: numbered
//! blocks, named variables that point at them, and B+trees, all big-endian.
//! CoreUI reads seven variables:
//! - `CARHEADER`: the format's versions and the rendition count;
//! - `EXTENDED_METADATA`: the platform and deployment target;
//! - `KEYFORMAT`: the attributes a rendition key holds, in order;
//! - `APPEARANCEKEYS`: appearance names, like `UIAppearanceDark`, to ids;
//! - `FACETKEYS`: each asset's name to its identifier, element and part;
//! - `RENDITIONS`: each key to its rendition, a CoreUI structured image
//!   (little-endian, like everything inside the blocks);
//! - `BITMAPKEYS`: each asset's identifier to the values its renditions'
//!   keys take, which is how CoreUI finds the masters a symbol has.
//!
//! A color's rendition holds its components as an `RLOC` payload, and an
//! image's its pixels, premultiplied BGRA compressed with gzip, which CoreUI
//! calls "zip". A vector image is drawn into one of those per scale, and its
//! PDF or SVG is kept as a rendition of its own. A custom symbol is an SVG
//! per weight and size its template draws, with the metrics that set it in
//! a line of text. The layout follows what Xcode 27's actool writes, worked
//! out from its output with `assetutil --info` and checked in the simulator.
//! Xcode compresses images with formats of its own, packs small ones into an
//! atlas and draws bitmaps of symbols too; CoreUI does without all three.

use std::collections::BTreeSet;
use std::io::Write as _;

use super::raster::Rgba;
use super::vector::Format;

/// The element every asset Xcode compiles has.
const ELEMENT: u16 = 85;

/// The deployment target, 2019, that Xcode gives a symbol's keys.
const GLYPH_DEPLOYMENT: u16 = 5;

/// The most entries a tree leaf holds before the tree gets another.
const LEAF_CAPACITY: usize = 256;

/// What a rendition is, as CoreUI's part attribute says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Part {
    /// A vector image's PDF or SVG, kept for each scale of a set that
    /// preserves vector data.
    Vector = 42,
    /// One master of a symbol.
    Glyph = 59,
    Image = 181,
    Color = 217,
    /// The sizes an app icon has for an idiom, each with its index.
    MultiSized = 218,
    /// One size of an app icon.
    Icon = 220,
}

/// The appearances a rendition can be for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Appearance {
    #[default]
    Any,
    Dark,
    HighContrastAny,
    HighContrastDark,
    Light,
    /// An app icon's tinted variant.
    Tinted,
}

impl Appearance {
    /// CoreUI's name and id for the appearance, which UIKit and AppKit
    /// spell their own ways.
    fn id(self, family: Family) -> (&'static str, u16) {
        match (family, self) {
            (Family::Ios, Appearance::Any) => ("UIAppearanceAny", 0),
            (Family::Ios, Appearance::Dark) => ("UIAppearanceDark", 1),
            (Family::Ios, Appearance::HighContrastAny) => ("UIAppearanceHighContrastAny", 2),
            (Family::Ios, Appearance::HighContrastDark) => ("UIAppearanceHighContrastDark", 3),
            (Family::Ios, Appearance::Light) => ("UIAppearanceLight", 4),
            (_, Appearance::Tinted) => ("ISAppearanceTintable", 10),
            (Family::Mac, Appearance::Any) => ("NSAppearanceNameSystem", 0),
            (Family::Mac, Appearance::Dark) => ("NSAppearanceNameDarkAqua", 1),
            (Family::Mac, Appearance::HighContrastAny) => {
                ("NSAppearanceNameAccessibilitySystem", 3)
            }
            (Family::Mac, Appearance::HighContrastDark) => {
                ("NSAppearanceNameAccessibilityDarkAqua", 6)
            }
            (Family::Mac, Appearance::Light) => ("NSAppearanceNameAqua", 8),
        }
    }
}

/// How an image is drawn: as it is, tinted, or as the context decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendering {
    Original,
    Template,
    Automatic,
}

/// Where an image's pixels came from, which its flags record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// Drawn from a PDF or SVG at the rendition's scale, rather than read
    /// from a bitmap file.
    pub vector: bool,
    /// The set preserves vector data, for drawing at other sizes. Xcode
    /// flags a set's bitmaps with it whatever their files are.
    pub preserved: bool,
}

/// A color's color space, with CoreUI's ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ColorSpace {
    Srgb = 1,
    GrayGamma22 = 2,
    DisplayP3 = 3,
    ExtendedSrgb = 4,
    ExtendedLinearSrgb = 5,
    ExtendedGray = 6,
}

/// The attributes that pick one rendition of an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub appearance: Appearance,
    pub scale: u16,
    /// CoreUI's idiom: 0 universal, 1 iPhone, 2 iPad, 3 TV, 4 CarPlay,
    /// 5 Watch, 6 marketing.
    pub idiom: u16,
}

/// An image's pixels as CoreUI keeps them: premultiplied BGRA, row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    bgra: Vec<u8>,
}

impl Bitmap {
    /// From pixels whose color isn't multiplied by their alpha, like a
    /// PNG's.
    pub fn from_rgba(image: &Rgba) -> Bitmap {
        let mut bgra = Vec::with_capacity(image.pixels.len());
        for px in image.pixels.chunks(4) {
            let a = u16::from(px[3]);
            let premultiply = |c: u8| byte((u16::from(c) * a + 127) / 255);
            bgra.extend_from_slice(&[
                premultiply(px[2]),
                premultiply(px[1]),
                premultiply(px[0]),
                px[3],
            ]);
        }
        Bitmap {
            width: image.width,
            height: image.height,
            bgra,
        }
    }

    /// From RGBA pixels whose color is multiplied by their alpha already,
    /// as a renderer draws them.
    pub fn from_premultiplied(width: u32, height: u32, rgba: &[u8]) -> Bitmap {
        let bgra = rgba
            .chunks(4)
            .flat_map(|px| [px[2], px[1], px[0], px[3]])
            .collect();
        Bitmap {
            width,
            height,
            bgra,
        }
    }

    fn opaque(&self) -> bool {
        self.bgra.chunks(4).all(|px| px[3] == 255)
    }

    /// The BGRA of the pixel `x` across and `y` down.
    #[cfg(test)]
    pub(super) fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let at = (y * self.width as usize + x) * 4;
        self.bgra[at..at + 4].try_into().unwrap()
    }
}

/// One master of a custom symbol: a weight and size its template draws,
/// as a standalone SVG in points at the platform's reference point size,
/// with the metrics that set it on a line of text, in points too.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// 1 ultralight to 9 black.
    pub weight: u16,
    /// 1 small, 2 medium, 3 large.
    pub size: u16,
    pub svg: Vec<u8>,
    /// The reference point size: 17, or 13 on macOS.
    pub point_size: u32,
    /// The point sizes Xcode draws the symbol's bitmaps at, listed by the
    /// masters it draws them from, like 15, 17 and 20.
    pub sizes: &'static [u32],
    /// How far the outline reaches below the baseline.
    pub baseline: f32,
    /// The cap height.
    pub capline: f32,
    /// The space the template's margin guides leave beside the outline.
    pub left_margin: f32,
    pub right_margin: f32,
    /// A space Xcode records after each margin, the same for every symbol
    /// at a point size.
    pub padding: f32,
    /// The template's version, from its "Template v.4.0" note, or 0.
    pub template_version: f32,
    /// Whether the template is a variable one, whose ultralight, regular
    /// and black masters CoreUI interpolates into the other weights.
    pub interpolatable: bool,
}

/// The platform a catalog is for, which decides the key format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// iOS, tvOS, watchOS and visionOS, whose keys name the idiom.
    Ios,
    /// macOS, whose keys don't.
    Mac,
}

impl Family {
    /// CoreUI's attribute ids, in the order a rendition key holds them:
    /// appearance 7, localization 13, scale 12, idiom 15, subtype 16,
    /// identifier 17, element 1, part 2, size 3, layer 11, with app icons
    /// dimension 9, an icon's size index, and with symbols dimension 9,
    /// deployment target 25, glyph weight 26 and glyph size 27.
    fn key_format(self, kinds: Kinds) -> &'static [u32] {
        match (self, kinds.glyphs, kinds.icons) {
            (Family::Ios, false, false) => &[7, 13, 12, 15, 16, 17, 1, 2],
            (Family::Ios, false, true) => &[7, 13, 12, 15, 16, 9, 17, 1, 2],
            (Family::Ios, true, _) => &[7, 13, 12, 15, 16, 26, 27, 9, 25, 17, 1, 2],
            (Family::Mac, false, false) => &[7, 13, 1, 2, 3, 17, 11, 12],
            (Family::Mac, false, true) => &[7, 13, 1, 2, 3, 17, 9, 11, 12],
            (Family::Mac, true, _) => &[7, 13, 1, 2, 3, 17, 9, 11, 12, 25, 26, 27],
        }
    }
}

/// The kinds of rendition a catalog has that widen its keys.
#[derive(Debug, Clone, Copy)]
struct Kinds {
    glyphs: bool,
    icons: bool,
}

/// The attributes of a rendition's key, the ones the stand-in sets.
#[derive(Debug, Clone, Copy, Default)]
struct Key {
    appearance: Appearance,
    scale: u16,
    idiom: u16,
    /// A device class within the idiom, like 1792 for the iPhone XR.
    subtype: u16,
    /// An app icon size's index.
    dimension: u16,
    identifier: u16,
    part: u16,
    deployment: u16,
    weight: u16,
    glyph_size: u16,
}

impl Key {
    fn bytes(&self, family: Family, kinds: Kinds) -> Vec<u8> {
        family
            .key_format(kinds)
            .iter()
            .flat_map(|attribute| {
                let value: u16 = match attribute {
                    7 => self.appearance.id(family).1,
                    12 => self.scale,
                    15 => self.idiom,
                    16 => self.subtype,
                    9 => self.dimension,
                    17 => self.identifier,
                    1 => ELEMENT,
                    2 => self.part,
                    25 => self.deployment,
                    26 => self.weight,
                    27 => self.glyph_size,
                    _ => 0,
                };
                value.to_le_bytes()
            })
            .collect()
    }
}

/// An asset's entry in `FACETKEYS`.
#[derive(Debug)]
struct Facet {
    name: String,
    identifier: u16,
    part: Part,
    symbol: bool,
}

/// An `Assets.car` being put together.
#[derive(Debug)]
pub struct Car {
    family: Family,
    /// The platform's name as the metadata spells it, like `ios`.
    platform: String,
    deployment_target: String,
    facets: Vec<Facet>,
    renditions: Vec<(Key, Vec<u8>)>,
    appearances: BTreeSet<Appearance>,
}

impl Car {
    pub fn new(family: Family, platform: &str, deployment_target: &str) -> Car {
        Car {
            family,
            platform: platform.to_string(),
            deployment_target: deployment_target.to_string(),
            facets: Vec::new(),
            renditions: Vec::new(),
            appearances: BTreeSet::from([Appearance::Any]),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.renditions.is_empty()
    }

    /// Add a color or image by name and return its identifier, or none when
    /// an asset of that name is in already.
    pub fn facet(&mut self, name: &str, part: Part) -> Option<u16> {
        self.add_facet(name, part, false)
    }

    /// Add a symbol by name, like [`Car::facet`]. Its part is the glyph's,
    /// as Xcode gives a symbol it draws no bitmaps of.
    pub fn symbol(&mut self, name: &str) -> Option<u16> {
        self.add_facet(name, Part::Glyph, true)
    }

    fn add_facet(&mut self, name: &str, part: Part, symbol: bool) -> Option<u16> {
        if self.facets.iter().any(|f| f.name == name) {
            return None;
        }
        let identifier = u16::try_from(self.facets.len() + 1).ok()?;
        self.facets.push(Facet {
            name: name.to_string(),
            identifier,
            part,
            symbol,
        });
        Some(identifier)
    }

    /// A color's rendition: its components in `space`, alpha last.
    pub fn color(
        &mut self,
        name: &str,
        identifier: u16,
        variant: Variant,
        space: ColorSpace,
        components: &[f64],
    ) {
        let mut payload = b"RLOC".to_vec();
        for value in [1, space as u32, count(components.len())] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        for c in components {
            payload.extend_from_slice(&c.to_le_bytes());
        }
        self.color_rendition(name, identifier, variant, &payload);
    }

    fn color_rendition(&mut self, name: &str, identifier: u16, variant: Variant, payload: &[u8]) {
        let info = [
            info(1004, &[&0u32.to_le_bytes(), &0f32.to_le_bytes()]),
            info(1006, &[&1u32.to_le_bytes()]),
        ];
        let header = Header {
            name,
            layout: 1009,
            flags: 0,
            width: 0,
            height: 0,
            scale: 0,
            pixel_format: 0,
            color_space: 0,
        };
        self.add(
            Key {
                part: Part::Color as u16,
                ..key(identifier, variant)
            },
            header.rendition(&info, payload),
        );
    }

    /// A rendition of a color that names a system color, like
    /// `systemRedColor`, which UIKit resolves by that name. Xcode also
    /// records the color's value as a fallback; this records black.
    pub fn system_color(&mut self, name: &str, identifier: u16, variant: Variant, system: &str) {
        let mut payload = b"RLOC".to_vec();
        // Bit 8 of the color space says a system color's name follows.
        for value in [1, ColorSpace::Srgb as u32 | 0x100, 4] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        for c in [0f64, 0.0, 0.0, 1.0] {
            payload.extend_from_slice(&c.to_le_bytes());
        }
        payload.extend_from_slice(b"RLOC");
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&count(system.len()).to_le_bytes());
        payload.extend_from_slice(system.as_bytes());
        self.color_rendition(name, identifier, variant, &payload);
    }

    /// An image's rendition, from the file `name` at `variant.scale`.
    pub fn image(
        &mut self,
        name: &str,
        identifier: u16,
        variant: Variant,
        image: &Bitmap,
        rendering: Rendering,
        source: Source,
    ) {
        let rendering = match rendering {
            Rendering::Original => 0,
            Rendering::Template => 0x08,
            Rendering::Automatic => 0x10,
        };
        // Bit 2 says the pixels were drawn from vector data, and bit 8
        // that the set keeps vector data.
        let source =
            if source.vector { 0x04 } else { 0 } | if source.preserved { 0x100 } else { 0 };
        let key = Key {
            part: Part::Image as u16,
            ..key(identifier, variant)
        };
        self.bitmap(key, name, image, rendering | source);
    }

    /// One size of the app icon `identifier`, from the file `name`: `index`
    /// is the size's in [`Car::multisized`]'s list, and `subtype` the device
    /// class it's for, or 0.
    pub fn icon(
        &mut self,
        name: &str,
        identifier: u16,
        variant: Variant,
        subtype: u16,
        index: u16,
        image: &Bitmap,
    ) {
        let key = Key {
            part: Part::Icon as u16,
            subtype,
            dimension: index,
            ..key(identifier, variant)
        };
        self.bitmap(key, name, image, 0);
    }

    /// The sizes the app icon `name` has for an idiom and device class, in
    /// points, each with its index.
    pub fn multisized(
        &mut self,
        name: &str,
        identifier: u16,
        idiom: u16,
        subtype: u16,
        sizes: &[(u32, u16)],
    ) {
        let mut payload = b"SISM".to_vec();
        for value in [1, count(sizes.len())] {
            payload.extend_from_slice(&u32::to_le_bytes(value));
        }
        for &(points, index) in sizes {
            for value in [points, points, u32::from(index)] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
        }
        let info = [
            info(1004, &[&0u32.to_le_bytes(), &0f32.to_le_bytes()]),
            info(1006, &[&1u32.to_le_bytes()]),
        ];
        let header = Header {
            name,
            layout: 1010,
            flags: 0,
            width: 0,
            height: 0,
            scale: 0,
            pixel_format: 0,
            color_space: 0,
        };
        let variant = Variant {
            appearance: Appearance::Any,
            scale: 1,
            idiom,
        };
        self.add(
            Key {
                part: Part::MultiSized as u16,
                subtype,
                ..key(identifier, variant)
            },
            header.rendition(&info, &payload),
        );
    }

    /// A bitmap's rendition, keyed by `key`, with the header's `flags`.
    fn bitmap(&mut self, key: Key, name: &str, image: &Bitmap, flags: u32) {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let compressed = gzip
            .write_all(&image.bgra)
            .and_then(|()| gzip.finish())
            .unwrap_or_default();
        let mut payload = b"MLEC".to_vec();
        // Bit 1 says the image is opaque; 2 is CoreUI's zip compression.
        for value in [
            if image.opaque() { 2 } else { 0 },
            2,
            count(compressed.len()),
        ] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&compressed);
        let (w, h) = (image.width, image.height);
        let zero = 0u32.to_le_bytes();
        let info = [
            info(
                1001,
                &[
                    &1u32.to_le_bytes(),
                    &zero,
                    &zero,
                    &w.to_le_bytes(),
                    &h.to_le_bytes(),
                ],
            ),
            info(
                1003,
                &[
                    &1u32.to_le_bytes(),
                    &zero,
                    &zero,
                    &zero,
                    &zero,
                    &w.to_le_bytes(),
                    &h.to_le_bytes(),
                ],
            ),
            info(1004, &[&zero, &1f32.to_le_bytes()]),
            info(1006, &[&1u32.to_le_bytes()]),
            info(1007, &[&(w * 4).to_le_bytes()]),
        ];
        let header = Header {
            name,
            layout: 12,
            flags,
            width: w,
            height: h,
            scale: u32::from(key.scale) * 100,
            pixel_format: u32::from_be_bytes(*b"ARGB"),
            color_space: 1,
        };
        self.add(key, header.rendition(&info, &payload));
    }

    /// A vector image's PDF or SVG, from the file `name`. A set that
    /// preserves vector data keeps one for each scale it has a file for;
    /// any other keeps one for no particular scale.
    pub fn vector(
        &mut self,
        name: &str,
        identifier: u16,
        variant: Variant,
        format: Format,
        data: &[u8],
        preserved: bool,
    ) {
        let header = Header {
            name,
            layout: 9,
            flags: 0x04,
            width: 0,
            height: 0,
            scale: 0,
            pixel_format: u32::from_be_bytes(match format {
                Format::Pdf => *b"PDF ",
                Format::Svg => *b"SVG ",
            }),
            color_space: 0,
        };
        let key = if preserved {
            Key {
                part: Part::Vector as u16,
                ..key(identifier, variant)
            }
        } else {
            Key {
                part: Part::Image as u16,
                scale: 0,
                ..key(identifier, variant)
            }
        };
        self.add(key, header.rendition(&vector_info(), &raw_data(data)));
    }

    /// One master of the symbol `identifier`, from the template file `name`.
    pub fn glyph(&mut self, name: &str, identifier: u16, variant: Variant, glyph: &Glyph) {
        let mut metrics = Vec::new();
        for value in [if glyph.interpolatable { 7 } else { 3 }, glyph.point_size] {
            metrics.extend_from_slice(&u32::to_le_bytes(value));
        }
        for value in [
            glyph.baseline,
            glyph.capline,
            glyph.template_version,
            glyph.left_margin,
            glyph.padding,
            glyph.right_margin,
            glyph.padding,
        ] {
            metrics.extend_from_slice(&value.to_le_bytes());
        }
        // The point sizes, each after its index.
        metrics.extend_from_slice(&count(glyph.sizes.len()).to_le_bytes());
        for (index, size) in (0u32..).zip(glyph.sizes) {
            metrics.extend_from_slice(&index.to_le_bytes());
            metrics.extend_from_slice(&size.to_le_bytes());
        }
        let mut info = vector_info();
        info.push(self::info(1018, &[&metrics]));
        info.push(self::info(
            1019,
            &[
                &1u32.to_le_bytes(),
                &u32::from(glyph.interpolatable).to_le_bytes(),
                &0u32.to_le_bytes(),
            ],
        ));
        let header = Header {
            name,
            layout: 1017,
            flags: 0x04,
            width: 0,
            height: 0,
            scale: 100,
            pixel_format: u32::from_be_bytes(*b"SVG "),
            color_space: 0,
        };
        self.add(
            Key {
                part: Part::Glyph as u16,
                scale: 1,
                deployment: GLYPH_DEPLOYMENT,
                weight: glyph.weight,
                glyph_size: glyph.size,
                ..key(identifier, variant)
            },
            header.rendition(&info, &raw_data(&glyph.svg)),
        );
    }

    fn add(&mut self, key: Key, rendition: Vec<u8>) {
        self.appearances.insert(key.appearance);
        self.renditions.push((key, rendition));
    }

    /// Whether any rendition is a symbol's or an app icon's, which widen
    /// the key format.
    fn kinds(&self) -> Kinds {
        Kinds {
            glyphs: self.facets.iter().any(|f| f.symbol),
            icons: self.facets.iter().any(|f| f.part == Part::Icon),
        }
    }

    /// The file.
    pub fn write(&self, timestamp: u32) -> Vec<u8> {
        let mut bom = Bom::default();
        let mut header = b"RATC".to_vec();
        for value in [1010, 17, timestamp, count(self.renditions.len())] {
            header.extend_from_slice(&value.to_le_bytes());
        }
        header.extend_from_slice(&padded(b"@(#)PROGRAM:CoreUI  PROJECT:CoreUI-1010", 128));
        header.extend_from_slice(&padded(b"sweetpad asset catalog compiler", 256));
        header.extend_from_slice(&[0; 16]);
        for value in [0u32, 2, 1, 2] {
            header.extend_from_slice(&value.to_le_bytes());
        }
        let index = bom.add(header);
        bom.variable("CARHEADER", index);

        let kinds = self.kinds();
        let format = self.family.key_format(kinds);
        let renditions = self
            .renditions
            .iter()
            .map(|(key, rendition)| (key.bytes(self.family, kinds), rendition.clone()))
            .collect();
        bom.tree("RENDITIONS", renditions, Keys::Fixed(2 * format.len()));
        let facets = self
            .facets
            .iter()
            .map(|f| {
                let mut attributes = vec![(1, ELEMENT), (2, f.part as u16), (17, f.identifier)];
                if f.symbol {
                    attributes.push((25, GLYPH_DEPLOYMENT));
                }
                let mut value = Vec::new();
                for v in [0, 0, u16::try_from(attributes.len()).unwrap_or(0)] {
                    value.extend_from_slice(&v.to_le_bytes());
                }
                for (attribute, v) in attributes {
                    value.extend_from_slice(&u16::to_le_bytes(attribute));
                    value.extend_from_slice(&v.to_le_bytes());
                }
                (f.name.as_bytes().to_vec(), value)
            })
            .collect();
        bom.tree("FACETKEYS", facets, Keys::Variable);
        let appearances = self
            .appearances
            .iter()
            .map(|a| {
                let (name, id) = a.id(self.family);
                (name.as_bytes().to_vec(), id.to_le_bytes().to_vec())
            })
            .collect();
        bom.tree("APPEARANCEKEYS", appearances, Keys::Variable);

        let mut key_format = b"tmfk".to_vec();
        for value in [0, count(format.len())].iter().chain(format) {
            key_format.extend_from_slice(&value.to_le_bytes());
        }
        let index = bom.add(key_format);
        bom.variable("KEYFORMAT", index);

        let mut metadata = b"META".to_vec();
        metadata.extend_from_slice(&[0; 256]);
        metadata.extend_from_slice(&padded(self.deployment_target.as_bytes(), 256));
        metadata.extend_from_slice(&padded(self.platform.as_bytes(), 256));
        metadata.extend_from_slice(&padded(b"sweetpad", 256));
        let index = bom.add(metadata);
        bom.variable("EXTENDED_METADATA", index);
        bom.tree("BITMAPKEYS", self.bitmap_keys(kinds), Keys::Inline);
        bom.finish()
    }

    /// Each asset's identifier, and for each attribute of the key format,
    /// a mask with a bit set for each value its renditions take, or every
    /// bit for the attributes CoreUI doesn't narrow by: appearance,
    /// identifier, element and part.
    fn bitmap_keys(&self, kinds: Kinds) -> Vec<(Vec<u8>, Vec<u8>)> {
        let format = self.family.key_format(kinds);
        self.facets
            .iter()
            .map(|facet| {
                let mut masks = vec![0u32; format.len()];
                for (key, _) in self
                    .renditions
                    .iter()
                    .filter(|(k, _)| k.identifier == facet.identifier)
                {
                    let values = key.bytes(self.family, kinds);
                    for (mask, value) in masks.iter_mut().zip(values.chunks(2)) {
                        let value = u16::from_le_bytes([value[0], value[1]]);
                        *mask |= 1u32.checked_shl(u32::from(value)).unwrap_or(u32::MAX);
                    }
                }
                let mut value = Vec::new();
                for v in [1, 0, count(4 + 4 * format.len()), count(format.len())] {
                    value.extend_from_slice(&u32::to_le_bytes(v));
                }
                for (attribute, mask) in format.iter().zip(masks) {
                    let mask = if matches!(attribute, 7 | 17 | 1 | 2) {
                        u32::MAX
                    } else {
                        mask
                    };
                    value.extend_from_slice(&mask.to_le_bytes());
                }
                (u32::from(facet.identifier).to_be_bytes().to_vec(), value)
            })
            .collect()
    }
}

/// A key for a variant of the asset `identifier`.
fn key(identifier: u16, variant: Variant) -> Key {
    Key {
        appearance: variant.appearance,
        scale: variant.scale,
        idiom: variant.idiom,
        identifier,
        ..Key::default()
    }
}

/// The info list of a rendition that holds a PDF or SVG.
fn vector_info() -> Vec<Vec<u8>> {
    vec![
        info(1004, &[&0u32.to_le_bytes(), &1f32.to_le_bytes()]),
        info(1006, &[&1u32.to_le_bytes()]),
    ]
}

/// A file kept as it is in a `RAWD` payload. Xcode compresses a large SVG
/// with LZFSE, flag 1; flag 0 is a file as it is, as Xcode keeps a PDF and a
/// small SVG.
fn raw_data(data: &[u8]) -> Vec<u8> {
    let mut payload = b"DWAR".to_vec();
    for value in [0, count(data.len())] {
        payload.extend_from_slice(&u32::to_le_bytes(value));
    }
    payload.extend_from_slice(data);
    payload
}

/// A structured image's header, before its info list and payload.
struct Header<'a> {
    name: &'a str,
    layout: u16,
    flags: u32,
    width: u32,
    height: u32,
    /// The scale times 100.
    scale: u32,
    pixel_format: u32,
    color_space: u32,
}

impl Header<'_> {
    fn rendition(&self, info: &[Vec<u8>], payload: &[u8]) -> Vec<u8> {
        let info: Vec<u8> = info.concat();
        let mut out = b"ISTC".to_vec();
        for value in [
            1,
            self.flags,
            self.width,
            self.height,
            self.scale,
            self.pixel_format,
            self.color_space,
            0,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.layout.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&padded(self.name.as_bytes(), 128));
        for value in [count(info.len()), 1, 0, count(payload.len())] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&info);
        out.extend_from_slice(payload);
        out
    }
}

/// One entry of a structured image's info list.
fn info(kind: u32, fields: &[&[u8]]) -> Vec<u8> {
    let body: Vec<u8> = fields.concat();
    let mut out = kind.to_le_bytes().to_vec();
    out.extend_from_slice(&count(body.len()).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// `text` cut or padded with NULs to `len` bytes, keeping a NUL at the end.
fn padded(text: &[u8], len: usize) -> Vec<u8> {
    let mut out = text[..text.len().min(len - 1)].to_vec();
    out.resize(len, 0);
    out
}

/// A length or count as the format's 32-bit field. What goes in a catalog
/// is far below 4 GiB.
#[allow(clippy::cast_possible_truncation)] // see above
fn count(n: usize) -> u32 {
    n as u32
}

/// A premultiplied channel, which is at most 255.
#[allow(clippy::cast_possible_truncation)] // see above
fn byte(value: u16) -> u8 {
    value as u8
}

/// How a tree keeps its keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keys {
    /// In blocks of their own, of any size.
    Variable,
    /// In blocks of their own, and in the nodes too, all of one size.
    Fixed(usize),
    /// As the 32-bit number itself, in place of a block number.
    Inline,
}

/// A BOM store being put together: blocks, and variables that name them.
#[derive(Default)]
struct Bom {
    /// Every block but the null block 0.
    blocks: Vec<Vec<u8>>,
    variables: Vec<(&'static str, u32)>,
}

impl Bom {
    fn add(&mut self, data: Vec<u8>) -> u32 {
        self.blocks.push(data);
        count(self.blocks.len())
    }

    fn variable(&mut self, name: &'static str, index: u32) {
        self.variables.push((name, index));
    }

    /// A B+tree of `entries`, sorted by key bytes, which is how CoreUI looks
    /// them up. Leaves hold up to [`LEAF_CAPACITY`] entries and link to their
    /// neighbors; past one leaf, a root branch points at each, separated by
    /// each leaf's last key. A tree with keys of one size keeps them in its
    /// nodes too, after the entries, and a node is that much longer.
    fn tree(&mut self, name: &'static str, mut entries: Vec<(Vec<u8>, Vec<u8>)>, keys: Keys) {
        const NODE_SIZE: usize = 4096;
        let key_size = match keys {
            Keys::Fixed(size) => Some(size),
            Keys::Variable | Keys::Inline => None,
        };
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let node =
            |is_leaf: bool, pairs: &[(u32, u32)], last: u32, keys: &[&[u8]], fwd: u32, bwd: u32| {
                let mut out = Vec::new();
                out.extend_from_slice(&u16::from(is_leaf).to_be_bytes());
                out.extend_from_slice(
                    &u16::try_from(pairs.len()).unwrap_or(u16::MAX).to_be_bytes(),
                );
                out.extend_from_slice(&fwd.to_be_bytes());
                out.extend_from_slice(&bwd.to_be_bytes());
                for (value, key) in pairs {
                    out.extend_from_slice(&value.to_be_bytes());
                    out.extend_from_slice(&key.to_be_bytes());
                }
                out.extend_from_slice(&last.to_be_bytes());
                let mut size = NODE_SIZE;
                if let Some(key_size) = key_size {
                    for key in keys {
                        out.extend_from_slice(key);
                    }
                    size += key_size * keys.len();
                }
                out.resize(size.max(out.len()), 0);
                out
            };
        let chunks: Vec<&[(Vec<u8>, Vec<u8>)]> = if entries.is_empty() {
            vec![&[]]
        } else {
            entries.chunks(LEAF_CAPACITY).collect()
        };
        // Each leaf's entries, then the leaves themselves once their
        // neighbors' block numbers are known.
        let mut leaves = Vec::new();
        for chunk in &chunks {
            let pairs: Vec<(u32, u32)> = chunk
                .iter()
                .map(|(key, value)| {
                    let value = self.add(value.clone());
                    let key = match keys {
                        Keys::Inline => key
                            .get(..4)
                            .and_then(|k| k.try_into().ok())
                            .map_or(0, u32::from_be_bytes),
                        Keys::Fixed(_) | Keys::Variable => self.add(key.clone()),
                    };
                    (value, key)
                })
                .collect();
            leaves.push(pairs);
        }
        let first = count(self.blocks.len()) + 1;
        let leaf_blocks: Vec<u32> = (0..leaves.len()).map(|i| first + count(i)).collect();
        for (i, (pairs, chunk)) in leaves.iter().zip(&chunks).enumerate() {
            let fwd = leaf_blocks.get(i + 1).copied().unwrap_or(0);
            let bwd = if i == 0 { 0 } else { leaf_blocks[i - 1] };
            let keys: Vec<&[u8]> = chunk.iter().map(|(k, _)| k.as_slice()).collect();
            self.add(node(true, pairs, 0, &keys, fwd, bwd));
        }
        let root = if leaves.len() == 1 {
            leaf_blocks[0]
        } else {
            let separators: Vec<(u32, u32)> = leaves[..leaves.len() - 1]
                .iter()
                .zip(&leaf_blocks)
                .map(|(pairs, block)| (*block, pairs.last().map_or(0, |p| p.1)))
                .collect();
            let keys: Vec<&[u8]> = chunks[..chunks.len() - 1]
                .iter()
                .filter_map(|chunk| chunk.last().map(|(k, _)| k.as_slice()))
                .collect();
            let last = *leaf_blocks.last().unwrap_or(&0);
            self.add(node(false, &separators, last, &keys, 0, 0))
        };
        let mut header = b"tree".to_vec();
        for value in [1, root, count(NODE_SIZE), count(entries.len())] {
            header.extend_from_slice(&value.to_be_bytes());
        }
        header.push(u8::from(keys == Keys::Inline));
        let key_size = match keys {
            Keys::Fixed(size) => count(size),
            Keys::Variable => u32::MAX,
            Keys::Inline => 0,
        };
        header.extend_from_slice(&key_size.to_be_bytes());
        header.extend_from_slice(&0u32.to_be_bytes());
        let index = self.add(header);
        self.variable(name, index);
    }

    /// The store: a 512-byte header area, the blocks 16-byte aligned, the
    /// variables, then the block index with two empty free-list entries.
    fn finish(self) -> Vec<u8> {
        let align = |out: &mut Vec<u8>| out.resize(out.len().next_multiple_of(16), 0);
        let mut out = vec![0u8; 512];
        let mut addresses = vec![(0u32, 0u32)];
        for block in &self.blocks {
            align(&mut out);
            addresses.push((count(out.len()), count(block.len())));
            out.extend_from_slice(block);
        }
        align(&mut out);
        let variables_offset = count(out.len());
        let mut variables = count(self.variables.len()).to_be_bytes().to_vec();
        for (name, index) in &self.variables {
            variables.extend_from_slice(&index.to_be_bytes());
            variables.push(u8::try_from(name.len()).unwrap_or(u8::MAX));
            variables.extend_from_slice(name.as_bytes());
        }
        out.extend_from_slice(&variables);
        align(&mut out);
        let index_offset = count(out.len());
        let mut index = count(addresses.len()).to_be_bytes().to_vec();
        for (address, length) in &addresses {
            index.extend_from_slice(&address.to_be_bytes());
            index.extend_from_slice(&length.to_be_bytes());
        }
        index.extend_from_slice(&2u32.to_be_bytes());
        index.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&index);
        let mut header = b"BOMStore".to_vec();
        for value in [
            1,
            count(self.blocks.len()),
            index_offset,
            count(index.len()),
            variables_offset,
            count(variables.len()),
        ] {
            header.extend_from_slice(&value.to_be_bytes());
        }
        out[..header.len()].copy_from_slice(&header);
        out
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A tree's entries in leaf order, as `(key, value)`.
    pub(in crate::cli::actool) type Entries<'a> = Vec<(&'a [u8], &'a [u8])>;

    /// A BOM store read back: blocks by index, and variables by name.
    pub(in crate::cli::actool) struct Store<'a> {
        data: &'a [u8],
        index: Vec<(usize, usize)>,
        variables: Vec<(String, usize)>,
    }

    fn be(data: &[u8], at: usize) -> usize {
        u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize
    }

    impl<'a> Store<'a> {
        pub(in crate::cli::actool) fn read(data: &'a [u8]) -> Store<'a> {
            assert_eq!(&data[..8], b"BOMStore");
            let (index_at, variables_at) = (be(data, 16), be(data, 24));
            let index = (0..be(data, index_at))
                .map(|i| {
                    (
                        be(data, index_at + 4 + 8 * i),
                        be(data, index_at + 8 + 8 * i),
                    )
                })
                .collect();
            let mut variables = Vec::new();
            let mut at = variables_at + 4;
            for _ in 0..be(data, variables_at) {
                let len = data[at + 4] as usize;
                variables.push((
                    String::from_utf8(data[at + 5..at + 5 + len].to_vec()).unwrap(),
                    be(data, at),
                ));
                at += 5 + len;
            }
            Store {
                data,
                index,
                variables,
            }
        }

        fn block(&self, i: usize) -> &'a [u8] {
            let (at, len) = self.index[i];
            &self.data[at..at + len]
        }

        pub(in crate::cli::actool) fn variable(&self, name: &str) -> &'a [u8] {
            let (_, i) = self.variables.iter().find(|(n, _)| n == name).unwrap();
            self.block(*i)
        }

        /// A tree's entries in leaf order, and its root node.
        pub(in crate::cli::actool) fn tree(&self, name: &str) -> (Entries<'a>, &'a [u8]) {
            let header = self.variable(name);
            assert_eq!(&header[..4], b"tree");
            let root = self.block(be(header, 8));
            let mut leaf = root;
            if root[1] == 0 {
                leaf = self.block(be(root, 12));
            }
            let mut out = Vec::new();
            loop {
                let n = u16::from_be_bytes([leaf[2], leaf[3]]) as usize;
                for i in 0..n {
                    out.push((
                        self.block(be(leaf, 16 + 8 * i)),
                        self.block(be(leaf, 12 + 8 * i)),
                    ));
                }
                match be(leaf, 4) {
                    0 => break,
                    next => leaf = self.block(next),
                }
            }
            (out, root)
        }

        /// A tree whose keys are numbers in place of block numbers, as
        /// `(key, value)` in leaf order.
        pub(in crate::cli::actool) fn inline_tree(&self, name: &str) -> Vec<(u32, &'a [u8])> {
            let header = self.variable(name);
            assert_eq!(header[20], 1, "{name} keeps its keys inline");
            let leaf = self.block(be(header, 8));
            let n = u16::from_be_bytes([leaf[2], leaf[3]]) as usize;
            (0..n)
                .map(|i| {
                    let key = u32::try_from(be(leaf, 16 + 8 * i)).unwrap();
                    (key, self.block(be(leaf, 12 + 8 * i)))
                })
                .collect()
        }
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    const ANY: Variant = Variant {
        appearance: Appearance::Any,
        scale: 1,
        idiom: 0,
    };

    #[test]
    fn a_color_is_written_as_xcode_writes_it() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        let id = car.facet("Brand", Part::Color).unwrap();
        // Xcode reads decimal components as 32-bit floats.
        let components = [0.8f32, 0.4, 0.2, 1.0].map(f64::from);
        car.color("Brand", id, ANY, ColorSpace::Srgb, &components);
        let data = car.write(0);
        let store = Store::read(&data);
        // What Xcode 27's actool wrote for the same color.
        let rendition = hex(concat!(
            "495354430100000000000000000000000000000000000000000000000000000000000000",
            "f10300004272616e64000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000",
            "000000000000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000001c0000000100000000000000",
            "30000000ec030000080000000000000000000000ee0300000400000001000000524c4f43",
            "010000000100000004000000000000a09999e93f000000a09999d93f000000a09999c93f",
            "000000000000f03f",
        ));
        let (renditions, _) = store.tree("RENDITIONS");
        assert_eq!(renditions.len(), 1);
        assert_eq!(renditions[0].1, rendition.as_slice());
        assert_eq!(
            renditions[0].0,
            hex("000000000100000000000100550 0d900"
                .replace(' ', "")
                .as_str())
        );
        let (facets, _) = store.tree("FACETKEYS");
        assert_eq!(
            facets,
            [(
                b"Brand".as_slice(),
                hex("000000000300010055000200d90011000100").as_slice()
            )]
        );
        assert_eq!(
            store.variable("KEYFORMAT"),
            hex(
                "746d666b0000000008000000070000000d0000000c0000000f00000010000000110000000100000002000000"
            )
        );
        let header = store.variable("CARHEADER");
        assert_eq!(
            &header[..20],
            hex("52415443f2030000110000000000000001000000")
        );
        assert_eq!(&header[420..], hex("00000000020000000100000002000000"));
    }

    #[test]
    fn a_big_tree_is_a_branch_over_linked_leaves() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        for i in 0..600 {
            let id = car.facet(&format!("C{i:03}"), Part::Color).unwrap();
            car.color("C", id, ANY, ColorSpace::Srgb, &[0.0, 0.0, 0.0, 1.0]);
        }
        let data = car.write(0);
        let store = Store::read(&data);
        let (renditions, root) = store.tree("RENDITIONS");
        assert_eq!(renditions.len(), 600);
        let keys: Vec<&[u8]> = renditions.iter().map(|(k, _)| *k).collect();
        assert!(
            keys.windows(2).all(|w| w[0] < w[1]),
            "keys are sorted by bytes"
        );
        // A branch of three leaves: two separators, then the last leaf.
        assert_eq!((root[1], u16::from_be_bytes([root[2], root[3]])), (0, 2));
        let first_leaf = store.block(be(root, 12));
        let separator = store.block(be(root, 16));
        assert_eq!(separator, keys[LEAF_CAPACITY - 1]);
        // Keys of one size are in the node too, after the entries.
        assert_eq!(
            &root[12 + 8 * 2 + 4..12 + 8 * 2 + 4 + 16],
            keys[LEAF_CAPACITY - 1]
        );
        assert_eq!(
            &first_leaf[12 + 8 * 256 + 4..12 + 8 * 256 + 4 + 16],
            keys[0]
        );
        let (facets, _) = store.tree("FACETKEYS");
        assert_eq!(facets.len(), 600);
    }

    #[test]
    fn an_image_keeps_its_premultiplied_pixels() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        let id = car.facet("Logo", Part::Image).unwrap();
        let image = Rgba {
            width: 1,
            height: 2,
            pixels: vec![255, 0, 0, 255, 255, 255, 255, 128],
        };
        let variant = Variant { scale: 2, ..ANY };
        car.image(
            "logo@2x.png",
            id,
            variant,
            &Bitmap::from_rgba(&image),
            Rendering::Template,
            Source {
                vector: false,
                preserved: false,
            },
        );
        let data = car.write(0);
        let store = Store::read(&data);
        let (renditions, _) = store.tree("RENDITIONS");
        let rendition = renditions[0].1;
        assert_eq!(&rendition[..4], b"ISTC");
        assert_eq!(
            u32::from_le_bytes(rendition[8..12].try_into().unwrap()),
            0x08
        );
        let info_len = u32::from_le_bytes(rendition[168..172].try_into().unwrap()) as usize;
        let payload = &rendition[184 + info_len..];
        assert_eq!(&payload[..4], b"MLEC");
        assert_eq!(&payload[4..12], [0, 0, 0, 0, 2, 0, 0, 0], "not opaque, zip");
        let mut pixels = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::GzDecoder::new(&payload[16..]),
            &mut pixels,
        )
        .unwrap();
        assert_eq!(pixels, [0, 0, 255, 255, 128, 128, 128, 128]);
    }

    #[test]
    fn appearances_are_listed_by_name() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        let id = car.facet("Brand", Part::Color).unwrap();
        let dark = Variant {
            appearance: Appearance::Dark,
            ..ANY
        };
        car.color("Brand", id, dark, ColorSpace::Srgb, &[0.0, 0.0, 0.0, 1.0]);
        let data = car.write(0);
        let (appearances, _) = Store::read(&data).tree("APPEARANCEKEYS");
        assert_eq!(
            appearances,
            [
                (b"UIAppearanceAny".as_slice(), [0u8, 0].as_slice()),
                (b"UIAppearanceDark".as_slice(), [1u8, 0].as_slice())
            ]
        );
        let mut car = Car::new(Family::Mac, "macosx", "14.0");
        let id = car.facet("Brand", Part::Color).unwrap();
        let light = Variant {
            appearance: Appearance::Light,
            ..ANY
        };
        car.color("Brand", id, light, ColorSpace::Srgb, &[0.0, 0.0, 0.0, 1.0]);
        let data = car.write(0);
        let store = Store::read(&data);
        let (appearances, _) = store.tree("APPEARANCEKEYS");
        assert_eq!(
            appearances,
            [
                (b"NSAppearanceNameAqua".as_slice(), [8u8, 0].as_slice()),
                (b"NSAppearanceNameSystem".as_slice(), [0u8, 0].as_slice())
            ]
        );
        let (renditions, _) = store.tree("RENDITIONS");
        assert_eq!(
            renditions[0].0[..2],
            [8, 0],
            "macOS keys lead with the appearance too"
        );
    }

    fn le(data: &[u8], at: usize) -> usize {
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
    }

    /// The regular medium master of the `legacy.svg` fixture, with the
    /// metrics Xcode 27's actool wrote for it.
    fn legacy_regular_medium() -> Glyph {
        Glyph {
            weight: 4,
            size: 2,
            svg: b"<svg/>".to_vec(),
            point_size: 17,
            sizes: &[15, 17, 20],
            baseline: f32::from_bits(0x3fae_147b),
            capline: f32::from_bits(0x413f_a6b5),
            left_margin: f32::from_bits(0x3fca_be30),
            right_margin: f32::from_bits(0x3fca_be30),
            padding: f32::from_bits(0x3f94_c000),
            template_version: 0.0,
            interpolatable: false,
        }
    }

    #[test]
    fn a_glyph_is_written_as_xcode_writes_it() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        let id = car.symbol("Legacy").unwrap();
        car.glyph("legacy.svg", id, ANY, &legacy_regular_medium());
        let data = car.write(0);
        let store = Store::read(&data);
        let (renditions, _) = store.tree("RENDITIONS");
        let (key, rendition) = renditions[0];
        // Xcode's, for the symbol it numbered 0x5a0c.
        assert_eq!(
            key,
            hex("0000000001000000000004000200000005000100 55003b00"
                .replace(' ', "")
                .as_str())
        );
        let mut header = b"ISTC".to_vec();
        header.extend_from_slice(&hex(
            "0100000004000000000000000000000064000000204756530000000000000000f9030000",
        ));
        header.extend_from_slice(&padded(b"legacy.svg", 128));
        header.extend_from_slice(&hex("780000000100000000000000"));
        assert_eq!(&rendition[..180], header.as_slice());
        let info = hex(concat!(
            "ec03000008000000000000000000803fee0300000400000001000000fa03000040000000",
            "03000000110000007b14ae3fb5a63f410000000030beca3f00c0943f30beca3f00c0943f",
            "03000000000000000f00000001000000110000000200000014000000",
            "fb0300000c000000010000000000000000000000",
        ));
        assert_eq!(&rendition[184..184 + info.len()], info.as_slice());
        // The SVG as it is, which CoreUI reads as Xcode keeps a small one.
        let payload = &rendition[184 + info.len()..];
        assert_eq!(&payload[..12], hex("445741520000000006000000"));
        assert_eq!(&payload[12..], b"<svg/>");
        // Xcode gives a symbol it draws no bitmaps of the glyph's part, and
        // a deployment target.
        let (facets, _) = store.tree("FACETKEYS");
        assert_eq!(
            facets[0].1,
            hex("0000000004000100550002003b001100010019000500")
        );
        assert_eq!(
            store.variable("KEYFORMAT"),
            hex(concat!(
                "746d666b000000000c000000070000000d0000000c0000000f000000100000001a000000",
                "1b0000000900000019000000110000000100000002000000"
            ))
        );
    }

    #[test]
    fn bitmap_keys_list_the_values_each_asset_takes() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        let brand = car.facet("Brand", Part::Color).unwrap();
        car.color("Brand", brand, ANY, ColorSpace::Srgb, &[0.0, 0.0, 0.0, 1.0]);
        let symbol = car.symbol("Variable").unwrap();
        for weight in [1, 4, 9] {
            let glyph = Glyph {
                weight,
                size: 1,
                sizes: &[],
                ..legacy_regular_medium()
            };
            car.glyph("variable.svg", symbol, ANY, &glyph);
        }
        let data = car.write(0);
        let keys = Store::read(&data).inline_tree("BITMAPKEYS");
        let masks = |value: &[u8]| {
            (0..value.len() / 4)
                .map(|i| u32::try_from(le(value, 4 * i)).unwrap())
                .collect::<Vec<_>>()
        };
        let all = u32::MAX;
        assert_eq!((keys[0].0, keys[1].0), (brand.into(), symbol.into()));
        // What Xcode 27's actool wrote for a color and for the fixture
        // `variable.svg`: weights 1, 4 and 9 at the small size, at 2019.
        assert_eq!(
            masks(keys[0].1),
            [1, 0, 52, 12, all, 1, 2, 1, 1, 1, 1, 1, 1, all, all, all]
        );
        assert_eq!(
            masks(keys[1].1),
            [1, 0, 52, 12, all, 1, 2, 1, 1, 530, 2, 1, 32, all, all, all]
        );
    }

    #[test]
    fn a_vector_image_is_kept_for_no_scale_unless_its_set_preserves_it() {
        let mut car = Car::new(Family::Ios, "ios", "17.0");
        let plain = car.facet("Plain", Part::Image).unwrap();
        let kept = car.facet("Kept", Part::Image).unwrap();
        let at_2x = Variant { scale: 2, ..ANY };
        car.vector("plain.pdf", plain, at_2x, Format::Pdf, b"%PDF", false);
        car.vector("kept.svg", kept, at_2x, Format::Svg, b"<svg/>", true);
        let data = car.write(0);
        let (renditions, _) = Store::read(&data).tree("RENDITIONS");
        let key = |r: &(&[u8], &[u8])| {
            (0..8)
                .map(|i| u16::from_le_bytes([r.0[2 * i], r.0[2 * i + 1]]))
                .collect::<Vec<_>>()
        };
        // Scale, identifier and part: a preserved one is kept for its own
        // scale, as part 42.
        let summary: Vec<(u16, u16, u16, &[u8])> = renditions
            .iter()
            .map(|r| {
                let k = key(r);
                let info = le(r.1, 168);
                (k[2], k[5], k[7], &r.1[184 + info..])
            })
            .collect();
        assert_eq!(summary.len(), 2);
        assert!(summary.contains(&(0, plain, 181, &b"DWAR\0\0\0\0\x04\0\0\0%PDF"[..])));
        assert!(summary.contains(&(2, kept, 42, &b"DWAR\0\0\0\0\x06\0\0\0<svg/>"[..])));
        let header = |r: &&(&[u8], &[u8])| {
            (
                le(r.1, 8),
                le(r.1, 24),
                u16::from_le_bytes([r.1[36], r.1[37]]),
            )
        };
        assert!(renditions.iter().all(|r| {
            let (flags, _, layout) = header(&r);
            flags == 4 && layout == 9
        }));
    }

    #[test]
    fn a_mac_cars_keys_widen_for_symbols_too() {
        let mut car = Car::new(Family::Mac, "macosx", "14.0");
        let id = car.symbol("Legacy").unwrap();
        car.glyph("legacy.svg", id, ANY, &legacy_regular_medium());
        let data = car.write(0);
        assert_eq!(
            Store::read(&data).variable("KEYFORMAT"),
            hex(concat!(
                "746d666b000000000c000000070000000d000000010000000200000003000000110000",
                "0009000000 0b0000000c000000190000001a0000001b000000"
            )
            .replace(' ', "")
            .as_str())
        );
    }
}
