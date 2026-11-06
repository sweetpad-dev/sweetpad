//! Resolve names against the system-font profile recorded with the native oracles.
//! Missing names use the system font, as ibtool does before archiving NSFont.
use super::{Element, Encoder, Id, Value};

pub(super) fn named_font(name: &str) -> (&str, i64) {
    let flags = match name {
        ".AppleSystemUIFont" => Some(1044),
        ".AppleSystemUIFontBold" => Some(2072),
        ".AppleSystemUIFontThin" => Some(5396),
        ".AppleSystemUIFontLight" => Some(5652),
        ".AppleSystemUIFontMedium" => Some(5908),
        ".AppleSystemUIFontHeavy" => Some(6424),
        ".AppleSystemUIFontBlack" => Some(6680),
        _ => None,
    };
    if let Some(flags) = flags {
        return (name, flags);
    }
    include_str!("font-names.tsv")
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(alias, _)| *alias == name)
        .map_or((".AppleSystemUIFont", 1044), |(_, name)| (name, 16))
}

impl Encoder<'_> {
    pub(super) fn font(&mut self, element: &Element) -> Result<Id, String> {
        if let Some(style) = element.attr("textStyle") {
            return self.text_style_font(element, style);
        }
        if element.attr("metaFont").is_none()
            && let Some(name) = element.attr("name")
        {
            let (name, flags) = named_font(name);
            let name = self.graph.string(name);
            let size = super::number(element, "size", 13.0)?;
            return Ok(self.graph.shared(
                "NSFont",
                vec![
                    ("NSName", name.into()),
                    ("NSSize", Value::Double(size)),
                    ("NSfFlags", Value::Int(flags)),
                ],
            ));
        }
        let (name, size, flags) = match element.attr("metaFont").unwrap_or("system") {
            "system" => (".AppleSystemUIFont", 13.0, 1044),
            "systemBold" => (".AppleSystemUIFontBold", 13.0, 2072),
            "smallSystem" => (".AppleSystemUIFont", 11.0, 3100),
            "smallSystemBold" => (".AppleSystemUIFontBold", 11.0, 3357),
            "menu" => (".AppleSystemUIFont", 13.0, 1301),
            "message" => (".AppleSystemUIFont", 13.0, 1558),
            "cellTitle" => (".AppleSystemUIFont", 12.0, 4883),
            "systemThin" => (".AppleSystemUIFontThin", f64::from_bits(1), 5396),
            other => return Err(format!("unsupported AppKit font {other}")),
        };
        let name = self.graph.string(name);
        let font = self.graph.object("NSFont");
        self.graph.set(font, "NSName", name);
        self.graph
            .set(font, "NSSize", super::number(element, "size", size)?);
        self.graph.set(font, "NSfFlags", i64::from(flags));
        if super::yes(element, "usesAppearanceFont") {
            self.graph.set(font, "NSFontUsesAppearanceFontSize", true);
        }
        Ok(self.graph.intern(font))
    }

    fn text_style_font(&mut self, element: &Element, style: &str) -> Result<Id, String> {
        let (usage, size, bold) = match style {
            "subheadline" => ("UICTFontTextStyleSubhead", 11.0, false),
            "headline" => ("UICTFontTextStyleHeadline", 13.0, true),
            "callout" => ("UICTFontTextStyleCallout", 12.0, false),
            "title3" => ("UICTFontTextStyleTitle3", 15.0, false),
            other => return Err(format!("unsupported AppKit text style {other}")),
        };
        let size = super::number(element, "size", size)?;
        let mut entries = Vec::new();
        let key = self.graph.string("NSCTFontSizeCategoryAttribute");
        let category = self
            .graph
            .shared("NSNumber", vec![("NS.intval", Value::Int(3))]);
        entries.push((key, category));
        let key = self.graph.string("NSCTFontUIUsageAttribute");
        let usage = self.graph.string(usage);
        entries.push((key, usage));
        let key = self.graph.string("NSFontSizeAttribute");
        let value = self.graph.number(size);
        entries.push((key, value));
        let attributes = self.graph.dictionary("NSDictionary", &entries);
        let descriptor = self.graph.shared(
            "NSFontDescriptor",
            vec![
                ("NSFontDescriptorAttributes", attributes.into()),
                ("NSFontDescriptorOptions", Value::Int(2_147_517_444)),
            ],
        );
        let name = self.graph.string(if bold {
            ".AppleSystemUIFontBold"
        } else {
            ".AppleSystemUIFont"
        });
        Ok(self.graph.shared(
            "NSFont",
            vec![
                ("NSName", name.into()),
                ("NSSize", Value::Double(size)),
                ("NSfFlags", Value::Int(16)),
                ("NSTextStyleDescriptor", descriptor.into()),
                ("NSHasWidth", Value::Bool(false)),
            ],
        ))
    }
}
