//! Named AppKit colors retain their catalog name and Xcode's fallback.
//! The bundled generic gray profile is the NSICC payload captured by the
//! MacControls oracle, not a host color-profile path needed at compile time.
use super::{Element, Encoder, Id, Value};

impl Encoder<'_> {
    pub(super) fn named_color(&mut self, name: &str) -> Result<Id, String> {
        self.color(&Element {
            name: "color".into(),
            attributes: [
                ("name", name),
                ("catalog", "System"),
                ("colorSpace", "catalog"),
            ]
            .map(|(key, value)| (key.into(), value.into()))
            .into(),
            children: Vec::new(),
            text: String::new(),
        })
    }

    pub(super) fn color(&mut self, element: &Element) -> Result<Id, String> {
        if element.attr("red").is_some() {
            return self.literal_rgb(element);
        }
        if element.attr("colorSpace") == Some("calibratedWhite") {
            let white = super::number(element, "white", 0.0)?;
            let alpha = super::number(element, "alpha", 1.0)?;
            let white = format!("{white:.10}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string();
            let components = if alpha.to_bits() == 1.0_f64.to_bits() {
                white
            } else {
                format!("{white} {alpha}")
            };
            return Ok(self.graph.shared(
                "NSColor",
                vec![
                    ("NSColorSpace", Value::Int(3)),
                    (
                        "NSWhite",
                        Value::Data(format!("{components}\0").into_bytes()),
                    ),
                ],
            ));
        }
        let name = element
            .attr("name")
            .ok_or("an AppKit color has no catalog name")?;
        if let Some(color) = self.named_colors.get(name).cloned() {
            return self.asset_color(name, &color);
        }
        if let Some(fallback) = match name {
            "textInsertionPointColor" => Some("systemBlueColor"),
            "_sourceListBackgroundColor" => Some("controlBackgroundColor"),
            "separatorColor" => Some("gridColor"),
            _ => None,
        } {
            return self.color_alias(name, fallback);
        }
        if matches!(name, "systemRedColor" | "systemBlueColor" | "linkColor") {
            return Ok(self.rgb_system_color(element, name));
        }
        let (white, components) = match name {
            "textBackgroundColor" | "headerColor" => ("1", "1 1"),
            "labelColor" | "textColor" | "controlTextColor" | "headerTextColor"
            | "selectedTextColor" => ("0", "0 1"),
            "controlBackgroundColor" | "controlColor" | "selectedTextBackgroundColor" => {
                ("0.602715373", "0.6666666667 1")
            }
            "secondaryLabelColor" => ("0.2637968361", "0.3333333333 1"),
            "gridColor" => ("0.4246723652", "0.5 1"),
            other => return Err(format!("unsupported AppKit color {other}")),
        };
        let profile = self.graph.shared(
            "NSData",
            vec![(
                "NS.bytes",
                Value::Data(include_bytes!("generic-gray-gamma22.icc").to_vec()),
            )],
        );
        let space = self.graph.shared(
            "NSColorSpace",
            vec![
                ("NSID", Value::Int(9)),
                ("NSModel", Value::Int(0)),
                ("NSICC", profile.into()),
            ],
        );
        let fallback = self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(3)),
                ("NSWhite", Value::Data(format!("{white}\0").into_bytes())),
                ("NSCustomColorSpace", space.into()),
                ("NSComponents", Value::Data(components.as_bytes().to_vec())),
                ("NSLinearExposure", Value::Data(b"1".to_vec())),
            ],
        );
        let catalog = self
            .graph
            .string(element.attr("catalog").unwrap_or("System"));
        let name = self.graph.string(name);
        Ok(self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(6)),
                ("NSCatalogName", catalog.into()),
                ("NSColorName", name.into()),
                ("NSColor", fallback.into()),
            ],
        ))
    }

    #[allow(clippy::cast_possible_truncation)] // ColorSync consumes single-precision source components.
    fn asset_color(&mut self, name: &str, color: &Element) -> Result<Id, String> {
        let input = ["red", "green", "blue"]
            .map(|key| super::number(color, key, 0.0))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        let profile: serde_json::Value =
            serde_json::from_str(include_str!("color-profile.json")).unwrap();
        let output = profile
            .as_array()
            .unwrap()
            .iter()
            .find(|p| {
                p["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(&input)
                    .all(|(a, b)| (a.as_f64().unwrap() as f32).to_bits() == (*b as f32).to_bits())
            })
            .ok_or("this named-color fallback needs a calibrated RGB profile sample")?;
        let alpha = super::number(color, "alpha", 1.0)?;
        let mut components = input
            .iter()
            .map(|v| color_component(*v))
            .collect::<Vec<_>>();
        components.push(color_component(alpha));
        let mut rgb = output["output"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| color_component(v.as_f64().unwrap()))
            .collect::<Vec<_>>();
        if alpha.to_bits() != 1.0_f64.to_bits() {
            rgb.push(color_component(alpha));
        }
        let profile = self.graph.shared(
            "NSData",
            vec![(
                "NS.bytes",
                Value::Data(include_bytes!("generic-rgb.icc").to_vec()),
            )],
        );
        let space = self.graph.shared(
            "NSColorSpace",
            vec![("NSID", Value::Int(7)), ("NSICC", profile.into())],
        );
        let fallback = self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(1)),
                (
                    "NSRGB",
                    Value::Data(format!("{}\0", rgb.join(" ")).into_bytes()),
                ),
                ("NSCustomColorSpace", space.into()),
                (
                    "NSComponents",
                    Value::Data(components.join(" ").into_bytes()),
                ),
                ("NSLinearExposure", Value::Data(b"0".to_vec())),
            ],
        );
        let name = self.graph.string(name);
        let catalog = self.graph.string("#$assets-mainBundleID");
        Ok(self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(6)),
                ("NSCatalogName", catalog.into()),
                ("NSColorName", name.into()),
                ("NSColor", fallback.into()),
            ],
        ))
    }

    fn color_alias(&mut self, name: &str, fallback: &str) -> Result<Id, String> {
        let fallback = self.named_color(fallback)?;
        let catalog = self.graph.string("System");
        let name = self.graph.string(name);
        Ok(self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(6)),
                ("NSCatalogName", catalog.into()),
                ("NSColorName", name.into()),
                ("NSColor", fallback.into()),
            ],
        ))
    }

    #[allow(clippy::float_cmp)]
    fn literal_rgb(&mut self, element: &Element) -> Result<Id, String> {
        let [r, g, b, a] = ["red", "green", "blue", "alpha"]
            .map(|key| super::number(element, key, if key == "alpha" { 1.0 } else { 0.0 }))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .unwrap();
        let rgb = format!("{r} {g} {b}");
        let rgba = format!("{rgb} {a}");
        let encoded = if a == 1.0 { rgb } else { rgba.clone() };
        let mut fields = vec![
            ("NSColorSpace", Value::Int(1)),
            ("NSRGB", Value::Data(format!("{encoded}\0").into_bytes())),
        ];
        if element.attr("colorSpace") != Some("calibratedRGB") {
            if element.attr("customColorSpace") != Some("sRGB")
                || r != g
                || g != b
                || (r != 0.0 && r != 1.0)
            {
                return Err(
                    "this AppKit RGB literal needs an sRGB-to-calibrated color transform".into(),
                );
            }
            let profile = self.graph.shared(
                "NSData",
                vec![(
                    "NS.bytes",
                    Value::Data(include_bytes!("generic-rgb.icc").to_vec()),
                )],
            );
            let space = self.graph.shared(
                "NSColorSpace",
                vec![("NSID", Value::Int(7)), ("NSICC", profile.into())],
            );
            fields.extend([
                ("NSCustomColorSpace", space.into()),
                ("NSComponents", Value::Data(rgba.into_bytes())),
                ("NSLinearExposure", Value::Data(b"1".to_vec())),
            ]);
        }
        Ok(self.graph.shared("NSColor", fields))
    }

    fn rgb_system_color(&mut self, element: &Element, name: &str) -> Id {
        let (rgb, components) = if name == "systemRedColor" {
            ("0.9859541655 0 0.02694000863", "1 0 0 1")
        } else {
            ("0 0 0.9981992245", "0 0 1 1")
        };
        let profile = self.graph.shared(
            "NSData",
            vec![(
                "NS.bytes",
                Value::Data(include_bytes!("generic-rgb.icc").to_vec()),
            )],
        );
        let space = self.graph.shared(
            "NSColorSpace",
            vec![("NSID", Value::Int(7)), ("NSICC", profile.into())],
        );
        let fallback = self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(1)),
                ("NSRGB", Value::Data(format!("{rgb}\0").into_bytes())),
                ("NSCustomColorSpace", space.into()),
                ("NSComponents", Value::Data(components.as_bytes().to_vec())),
                ("NSLinearExposure", Value::Data(b"1".to_vec())),
            ],
        );
        let catalog = self
            .graph
            .string(element.attr("catalog").unwrap_or("System"));
        let name = self.graph.string(name);
        self.graph.shared(
            "NSColor",
            vec![
                ("NSColorSpace", Value::Int(6)),
                ("NSCatalogName", catalog.into()),
                ("NSColorName", name.into()),
                ("NSColor", fallback.into()),
            ],
        )
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn color_component(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let precision = (9 - value.abs().log10().floor() as i32).max(0) as usize;
    let text = format!("{value:.precision$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        text
    }
}
