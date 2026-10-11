//! Apple's compiled asset catalog, `Assets.car`, with the named colors and
//! raster images that UIKit's and SwiftUI's lookups by name read.
//!
//! The file is a BOM store, NeXTSTEP's bill-of-materials format: numbered
//! blocks, named variables that point at them, and B+trees, all big-endian.
//! CoreUI reads six variables:
//! - `CARHEADER`: the format's versions and the rendition count;
//! - `EXTENDED_METADATA`: the platform and deployment target;
//! - `KEYFORMAT`: the attributes a rendition key holds, in order;
//! - `APPEARANCEKEYS`: appearance names, like `UIAppearanceDark`, to ids;
//! - `FACETKEYS`: each asset's name to its identifier, element and part;
//! - `RENDITIONS`: each key to its rendition, a CoreUI structured image
//!   (little-endian, like everything inside the blocks).
//!
//! A color's rendition holds its components as an `RLOC` payload, and an
//! image's its pixels, premultiplied BGRA compressed with gzip, which CoreUI
//! calls "zip". The layout follows what Xcode 27's actool writes, worked out
//! from its output with `assetutil --info` and checked in the simulator.
//! Xcode compresses images with formats of its own and packs small ones into
//! an atlas; neither is needed to read them back.

use std::collections::BTreeSet;
use std::io::Write as _;

use super::raster::Rgba;

/// The element every asset Xcode compiles has.
const ELEMENT: u16 = 85;

/// The most entries a tree leaf holds before the tree gets another.
const LEAF_CAPACITY: usize = 256;

/// What kind of asset a facet is, as CoreUI's part attribute says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Part {
    Image = 181,
    Color = 217,
}

/// The appearances a rendition can be for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Appearance {
    Any,
    Dark,
    HighContrastAny,
    HighContrastDark,
    Light,
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
    /// identifier 17, element 1, part 2, size 3, layer 11.
    fn key_format(self) -> &'static [u32] {
        match self {
            Family::Ios => &[7, 13, 12, 15, 16, 17, 1, 2],
            Family::Mac => &[7, 13, 1, 2, 3, 17, 11, 12],
        }
    }
}

/// An `Assets.car` being put together.
#[derive(Debug)]
pub struct Car {
    family: Family,
    /// The platform's name as the metadata spells it, like `ios`.
    platform: String,
    deployment_target: String,
    /// Each asset's name, identifier and part, in the order they're added.
    facets: Vec<(String, u16, Part)>,
    renditions: Vec<(Vec<u8>, Vec<u8>)>,
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

    /// Add an asset by name and return its identifier, or none when an
    /// asset of that name is in already.
    pub fn facet(&mut self, name: &str, part: Part) -> Option<u16> {
        if self.facets.iter().any(|(n, ..)| n == name) {
            return None;
        }
        let identifier = u16::try_from(self.facets.len() + 1).ok()?;
        self.facets.push((name.to_string(), identifier, part));
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
            identifier,
            Part::Color,
            variant,
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
        image: &Rgba,
        rendering: Rendering,
    ) {
        let opaque = image.pixels.chunks(4).all(|px| px[3] == 255);
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
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let compressed = gzip
            .write_all(&bgra)
            .and_then(|()| gzip.finish())
            .unwrap_or_default();
        let mut payload = b"MLEC".to_vec();
        // Bit 1 says the image is opaque; 2 is CoreUI's zip compression.
        for value in [if opaque { 2 } else { 0 }, 2, count(compressed.len())] {
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
            flags: match rendering {
                Rendering::Original => 0,
                Rendering::Template => 0x08,
                Rendering::Automatic => 0x10,
            },
            width: w,
            height: h,
            scale: u32::from(variant.scale) * 100,
            pixel_format: u32::from_le_bytes(*b"BGRA"),
            color_space: 1,
        };
        self.add(
            identifier,
            Part::Image,
            variant,
            header.rendition(&info, &payload),
        );
    }

    fn add(&mut self, identifier: u16, part: Part, variant: Variant, rendition: Vec<u8>) {
        self.appearances.insert(variant.appearance);
        let key = self
            .family
            .key_format()
            .iter()
            .flat_map(|attribute| {
                let value: u16 = match attribute {
                    7 => variant.appearance.id(self.family).1,
                    12 => variant.scale,
                    15 => variant.idiom,
                    17 => identifier,
                    1 => ELEMENT,
                    2 => part as u16,
                    _ => 0,
                };
                value.to_le_bytes()
            })
            .collect();
        self.renditions.push((key, rendition));
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

        let key_size = 2 * self.family.key_format().len();
        bom.tree("RENDITIONS", self.renditions.clone(), Some(key_size));
        let facets = self
            .facets
            .iter()
            .map(|(name, identifier, part)| {
                let mut value = Vec::new();
                for v in [0, 0, 3, 1, ELEMENT, 2, *part as u16, 17, *identifier] {
                    value.extend_from_slice(&v.to_le_bytes());
                }
                (name.as_bytes().to_vec(), value)
            })
            .collect();
        bom.tree("FACETKEYS", facets, None);
        let appearances = self
            .appearances
            .iter()
            .map(|a| {
                let (name, id) = a.id(self.family);
                (name.as_bytes().to_vec(), id.to_le_bytes().to_vec())
            })
            .collect();
        bom.tree("APPEARANCEKEYS", appearances, None);

        let format = self.family.key_format();
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
        bom.finish()
    }
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
    fn tree(
        &mut self,
        name: &'static str,
        mut entries: Vec<(Vec<u8>, Vec<u8>)>,
        key_size: Option<usize>,
    ) {
        const NODE_SIZE: usize = 4096;
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
                    (value, self.add(key.clone()))
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
        header.push(0);
        header.extend_from_slice(&key_size.map_or(u32::MAX, count).to_be_bytes());
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

        fn variable(&self, name: &str) -> &'a [u8] {
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
        car.image("logo@2x.png", id, variant, &image, Rendering::Template);
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
}
