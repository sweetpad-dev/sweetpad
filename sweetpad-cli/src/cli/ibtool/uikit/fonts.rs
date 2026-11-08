//! Font descriptors and text styles recorded by the Xcode probes.

use super::{Element, Encoder, Id, Value, number};

impl Encoder<'_> {
    /// A font's archived descriptor, measured against Xcode's font probes.
    pub(super) fn font(&mut self, description: Option<&Element>) -> Result<Id, String> {
        self.font_with_default(description, 17.0)
    }

    pub(super) fn font_with_default(
        &mut self,
        description: Option<&Element>,
        default_size: f64,
    ) -> Result<Id, String> {
        let kind = description.and_then(|d| d.attr("type")).unwrap_or("system");
        let weight = description.and_then(|d| d.attr("weight"));
        let style = description.and_then(|d| d.attr("style"));
        let custom = description.and_then(|d| d.attr("name"));
        let size = description
            .map(|d| number(d, "pointSize"))
            .transpose()?
            .flatten()
            .unwrap_or(default_size);
        let (name, usage, override_usage, traits) = match (kind, weight) {
            ("system", None | Some("regular")) => (".SFUI-Regular", "CTFontRegularUsage", None, 0),
            ("system", Some("ultraLight")) => {
                (".SFUI-Ultralight", "CTFontUltraLightUsage", None, 16)
            }
            ("system", Some("thin")) => (".SFUI-Thin", "CTFontThinUsage", None, 4),
            ("system", Some("light")) => (".SFUI-Light", "CTFontLightUsage", None, 8),
            ("system", Some("medium")) => (".SFUI-Medium", "CTFontMediumUsage", None, 0),
            ("system", Some("semibold")) => (".SFUI-Semibold", "CTFontDemiUsage", None, 2),
            ("boldSystem", None) | ("system", Some("bold")) => (
                ".SFUI-Bold",
                "CTFontEmphasizedUsage",
                Some("CTFontBoldUsage"),
                2,
            ),
            ("system", Some("heavy")) => (".SFUI-Heavy", "CTFontHeavyUsage", None, 2),
            ("system", Some("black")) => (".SFUI-Black", "CTFontBlackUsage", None, 2),
            ("italicSystem", None) => (".SFUI-RegularItalic", "CTFontObliqueUsage", None, 1),
            _ => {
                return Err(format!(
                    "the font {kind} {} isn't supported",
                    weight.unwrap_or("")
                ));
            }
        };
        let (name, traits, point_size, archive_size) = if let Some(style) = style {
            let (name, traits, size) = match style {
                "UICTFontTextStyleBody" => (".SFUI-Regular", 0, 14.0),
                "UICTFontTextStyleCallout" => (".SFUI-Regular", 0, 13.0),
                "UICTFontTextStyleHeadline" => (".SFUI-Semibold", 2, 14.0),
                "UICTFontTextStyleCaption1" | "UICTFontTextStyleCaption2" => {
                    (".SFUI-Regular", 0, 11.0)
                }
                "UICTFontTextStyleTitle1" => (".SFUI-Regular", 0, 25.0),
                "UICTFontTextStyleTitle2" => (".SFUI-Regular", 0, 19.0),
                "UICTFontTextStyleTitle3" => (".SFUI-Regular", 0, 17.0),
                "UICTFontTextStyleTitle0" => (".SFUI-Regular", 0, 31.0),
                "UICTFontTextStyleSubhead" | "UICTFontTextStyleFootnote" => {
                    (".SFUI-Regular", 0, 12.0)
                }
                other => return Err(format!("the text style {other} isn't supported")),
            };
            (name, traits, 0.0, size)
        } else {
            (
                custom.unwrap_or(name),
                custom.map_or(traits, |name| {
                    // PostScript names preserve the named face; the archive
                    // also records its bold/italic style bits.
                    let name = name.to_ascii_lowercase();
                    (i64::from(name.contains("bold")) * 2)
                        | i64::from(name.contains("italic") || name.contains("oblique"))
                }),
                size,
                size,
            )
        };
        let descriptor = style
            .is_none()
            .then(|| self.font_descriptor(description, usage, override_usage, size));
        let name = self.graph.string(name);
        let mut values = vec![("UIFontName", Value::Object(name))];
        if let Some(descriptor) = descriptor {
            values.push(("UIFontDescriptor", Value::Object(descriptor)));
        }
        if let Some(style) = style {
            let style = self.graph.string(style);
            values.push(("UIIBTextStyle", Value::Object(style)));
        }
        values.extend([
            ("UIFontPointSize", Value::Double(point_size)),
            ("UIFontTextStyleForScaling", Value::Nil),
            ("UIFontPointSizeForScaling", Value::Double(0.0)),
            ("UIFontMaximumPointSizeAfterScaling", Value::Double(0.0)),
            ("UIFontTraits", Value::Int(traits)),
            ("UISystemFont", Value::Bool(custom.is_none())),
            ("NSName", Value::Object(name)),
            ("NSSize", Value::Double(archive_size)),
        ]);
        Ok(self.graph.shared("UIFont", values))
    }

    fn font_descriptor(
        &mut self,
        description: Option<&Element>,
        usage: &str,
        override_usage: Option<&str>,
        size: f64,
    ) -> Id {
        let custom = description.and_then(|d| d.attr("name"));
        let kind = description.and_then(|d| d.attr("type")).unwrap_or("system");
        let mut entries = Vec::new();
        if let Some(name) = custom {
            let key = self.graph.string("NSFontNameAttribute");
            let value = self.graph.string(name);
            entries.push((key, value));
        } else {
            if kind == "italicSystem" {
                let trait_key = self.graph.string("NSCTFontSymbolicTrait");
                let trait_value = self
                    .graph
                    .shared("NSNumber", vec![("NS.intval", Value::Int(1))]);
                let trait_dict = self
                    .graph
                    .dictionary("NSDictionary", &[(trait_key, trait_value)]);
                let key = self.graph.string("NSCTFontTraitsAttribute");
                entries.push((key, trait_dict));
            }
            let key = self.graph.string("NSCTFontUIUsageAttribute");
            let value = self.graph.string(usage);
            entries.push((key, value));
            if let Some(usage) = override_usage {
                let key = self.graph.string("NSCTFontUIUsageOverrideAttribute");
                let value = self.graph.string(usage);
                entries.push((key, value));
            }
        }
        let key = self.graph.string("NSFontSizeAttribute");
        let value = self.graph.number(size);
        entries.push((key, value));
        let attributes = self.graph.dictionary(
            if override_usage.is_some() && custom.is_none() {
                "NSMutableDictionary"
            } else {
                "NSDictionary"
            },
            &entries,
        );
        self.graph.shared(
            "UIFontDescriptor",
            vec![
                ("UIFontDescriptorAttributes", Value::Object(attributes)),
                (
                    "UIFontDescriptorOptions",
                    Value::Int(if custom.is_some() {
                        2_147_483_648
                    } else {
                        2_147_517_444
                    }),
                ),
            ],
        )
    }
}
