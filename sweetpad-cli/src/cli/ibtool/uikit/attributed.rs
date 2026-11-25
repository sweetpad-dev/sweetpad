//! Foundation attributed strings retain each run and its UTF-16 length.
//! NSAttributes / NSAttributeInfo follow Foundation's keyed coding format
//! (swift-corelibs-foundation, NSAttributedString.swift).
use super::controls::enumeration;
use super::{Element, Encoder, Id, Value, number};

pub(super) struct LabelStyle {
    pub line_break: i64,
    pub shadow_color: Option<Id>,
    pub shadow_offset: [f64; 2],
}

struct Run {
    length: usize,
    attributes: Vec<(Id, Id)>,
}

pub(super) struct AttributedText {
    pub archive: Id,
    pub text: String,
    pub font: Option<(Id, f64)>,
    pub color: Option<Id>,
    pub alignment: i64,
}

fn check_attributes(element: &Element, allowed: &[&str]) -> Result<(), String> {
    for (key, _) in &element.attributes {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unsupported <{}> attribute {key}", element.name));
        }
    }
    Ok(())
}

fn run_integer(mut value: usize, bytes: &mut Vec<u8>) {
    while value >= 128 {
        bytes.push(u8::try_from(value & 127).unwrap() | 128);
        value >>= 7;
    }
    bytes.push(u8::try_from(value).unwrap());
}

impl Encoder<'_> {
    pub(super) fn attributed_text(
        &mut self,
        element: &Element,
        label: Option<&LabelStyle>,
    ) -> Result<AttributedText, String> {
        check_attributes(element, &["key"])?;
        let mut text = String::new();
        let mut runs: Vec<Run> = Vec::new();
        let mut font = None;
        let mut color = None;
        let mut alignment = 4;
        for fragment in &element.children {
            if fragment.name != "fragment" {
                return Err(format!(
                    "unsupported attributed string child <{}>",
                    fragment.name
                ));
            }
            check_attributes(fragment, &["content"])?;
            let content = fragment.attr("content").unwrap_or(&fragment.text);
            let mut entries = Vec::new();
            for attributes in &fragment.children {
                if attributes.name != "attributes" {
                    return Err(format!("unsupported fragment child <{}>", attributes.name));
                }
                check_attributes(attributes, &[])?;
                for attribute in &attributes.children {
                    let key = attribute.attr("key").ok_or("a text attribute has no key")?;
                    let value = match (attribute.name.as_str(), key) {
                        (
                            "color",
                            "NSColor"
                            | "NSBackgroundColor"
                            | "NSStrokeColor"
                            | "NSUnderlineColor"
                            | "NSStrikethroughColor",
                        ) => {
                            let value = self.color(attribute)?;
                            if key == "NSColor" && text.is_empty() && !content.is_empty() {
                                color = Some(value);
                            }
                            value
                        }
                        ("font", "NSFont") => {
                            check_attributes(attribute, &["key", "size", "name"])?;
                            let size = number(attribute, "size")?
                                .ok_or("an attributed font has no size")?;
                            if !size.is_finite() || size <= 0.0 {
                                return Err(
                                    "an attributed font must have a positive finite size".into()
                                );
                            }
                            let value = self.font_with_default(Some(attribute), size)?;
                            if !content.is_empty() {
                                font.get_or_insert((value, size));
                            }
                            value
                        }
                        ("paragraphStyle", "NSParagraphStyle") => {
                            let (value, paragraph_alignment) =
                                self.attributed_paragraph(attribute, label)?;
                            if text.is_empty() && !content.is_empty() {
                                alignment = paragraph_alignment;
                            }
                            value
                        }
                        _ => {
                            return Err(format!(
                                "unsupported text attribute <{}> {key}",
                                attribute.name
                            ));
                        }
                    };
                    let key = self.graph.string(key);
                    if entries.iter().any(|(existing, _)| *existing == key) {
                        return Err("duplicate attributed string attribute".into());
                    }
                    entries.push((key, value));
                }
            }
            if !content.is_empty() {
                let length = content.encode_utf16().count();
                text.push_str(content);
                entries.sort_by_key(|(key, _)| *key);
                if let Some(previous) = runs.last_mut()
                    && previous.attributes == entries
                {
                    previous.length += length;
                } else {
                    runs.push(Run {
                        length,
                        attributes: entries,
                    });
                }
            }
        }

        Ok(AttributedText {
            archive: self.archive_runs(&text, runs, label),
            text,
            font,
            color,
            alignment,
        })
    }

    fn archive_runs(&mut self, text: &str, mut runs: Vec<Run>, label: Option<&LabelStyle>) -> Id {
        // UILabel applies its shadow across all distinct runs, even when the
        // default offset is the only shadow property. UITextView does not.
        if let Some(style) = label.filter(|s| {
            runs.len() > 1 || s.shadow_color.is_some() || !matches!(s.shadow_offset, [0.0, -1.0])
        }) {
            let [x, y] = style.shadow_offset;
            let shadow = self.graph.object("NSShadow");
            self.graph.set(
                shadow,
                "NSShadowColor",
                style.shadow_color.map_or(Value::Nil, Value::Object),
            );
            if x != 0.0 {
                self.graph.set(shadow, "NSShadowHoriz", x);
            }
            if y != 0.0 {
                self.graph.set(shadow, "NSShadowVert", -y);
            }
            let key = self.graph.string("NSShadow");
            for run in &mut runs {
                run.attributes.push((key, shadow));
            }
        }
        let object = self.graph.object(if label.is_some() {
            "NSMutableAttributedString"
        } else {
            "NSAttributedString"
        });
        let string = if label.is_some() {
            let string = self.graph.object("NSMutableString");
            self.graph
                .set(string, "NS.bytes", Value::Data(text.as_bytes().to_vec()));
            string
        } else {
            self.graph.string(text)
        };
        self.graph.set(object, "NSString", string);
        let runs = runs
            .into_iter()
            .map(|run| {
                (
                    run.length,
                    self.graph.dictionary("NSDictionary", &run.attributes),
                )
            })
            .collect::<Vec<_>>();
        if runs.len() == 1 {
            self.graph.set(object, "NSAttributes", runs[0].1);
        } else if !runs.is_empty() {
            let mut dictionaries = Vec::new();
            let mut bytes = Vec::new();
            for (length, attributes) in runs {
                let index = dictionaries
                    .iter()
                    .position(|id| *id == attributes)
                    .unwrap_or_else(|| {
                        dictionaries.push(attributes);
                        dictionaries.len() - 1
                    });
                run_integer(length, &mut bytes);
                run_integer(index, &mut bytes);
            }
            let attributes = self.graph.array(
                if label.is_some() {
                    "NSMutableArray"
                } else {
                    "NSArray"
                },
                &dictionaries,
            );
            let info = self.graph.object(if label.is_some() {
                "NSMutableData"
            } else {
                "NSData"
            });
            self.graph.set(info, "NS.bytes", Value::Data(bytes));
            self.graph.set(object, "NSAttributes", attributes);
            self.graph.set(object, "NSAttributeInfo", info);
        }
        object
    }

    fn attributed_paragraph(
        &mut self,
        element: &Element,
        label: Option<&LabelStyle>,
    ) -> Result<(Id, i64), String> {
        check_attributes(
            element,
            &["key", "alignment", "lineBreakMode", "baseWritingDirection"],
        )?;
        if !element.children.is_empty() {
            return Err("unsupported paragraph style children".into());
        }
        let alignment = enumeration(
            element,
            "alignment",
            &[
                ("left", 0),
                ("center", 1),
                ("right", 2),
                ("justified", 3),
                ("natural", 4),
            ],
            4,
        )?;
        let line_break = enumeration(
            element,
            "lineBreakMode",
            &[
                ("wordWrapping", 0),
                ("charWrapping", 1),
                ("clipping", 2),
                ("truncatingHead", 3),
                ("truncatingTail", 4),
                ("truncatingMiddle", 5),
            ],
            0,
        )?;
        let direction = enumeration(
            element,
            "baseWritingDirection",
            &[("natural", -1), ("leftToRight", 0), ("rightToLeft", 1)],
            -1,
        )?;
        let mut values = vec![("NSTabStops", Value::Nil)];
        if let Some(label) = label {
            // NSParagraphStyle archives alignment in AppKit order. UIKit
            // applies the label's line-break mode to its attributed runs.
            let archived_alignment = match alignment {
                1 => 2,
                2 => 1,
                value => value,
            };
            if archived_alignment != 0 {
                values.push(("NSAlignment", Value::Int(archived_alignment)));
            }
            if label.line_break != 0 {
                values.push(("NSLineBreakMode", Value::Int(label.line_break)));
            }
            values.push(("NSLineBreakStrategy", Value::Int(65535)));
            if direction != -1 {
                values.push(("NSWritingDirection", Value::Float(1.0)));
            }
        } else {
            values.extend([
                ("NSAlignment", Value::Int(alignment)),
                ("NSLineBreakMode", Value::Int(line_break)),
                ("NSWritingDirection", Value::Int(direction)),
            ]);
        }
        Ok((
            self.graph.shared("NSMutableParagraphStyle", values),
            alignment,
        ))
    }
}
