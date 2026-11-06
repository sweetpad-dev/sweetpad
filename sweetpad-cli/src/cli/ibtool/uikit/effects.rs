//! Remaining UIKit controls, visual effects, and runtime key/value pairs.

use super::{Element, Encoder, Fallback, Id, Value, keyed, no, number, single, yes};

impl Encoder<'_> {
    pub(super) fn additional_control(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        match element.name.as_str() {
            "pickerView" => self.graph.set(object, "UIAllowSelectingCells", true),
            "slider" => {
                for (attribute, key, default) in [
                    ("value", "UIValue", 0.0),
                    ("minValue", "UIMinValue", 0.0),
                    ("maxValue", "UIMaxValue", 1.0),
                ] {
                    self.graph.set(
                        object,
                        key,
                        single(number(element, attribute)?.unwrap_or(default)),
                    );
                }
                if no(element, "continuous") {
                    self.graph.set(object, "UIIsContinuous", false);
                }
                if let Some(color) = keyed(element, "color", "thumbTintColor") {
                    let color = self.color(color)?;
                    self.graph.set(object, "UIThumbTintColor", color);
                }
                self.graph.set(object, "UIBehavioralStyle", 0_i64);
            }
            "activityIndicatorView" => self.activity_indicator(element, object)?,
            "refreshControl" => {
                if no(element, "enabled") {
                    self.graph.set(object, "UIDisabled", true);
                }
                self.graph.set(object, "UIRefreshControlStyle", 2_i64);
            }
            "visualEffectView" => {
                let blur = element
                    .child("blurEffect")
                    .ok_or("a visual effect view has no blur effect")?;
                let effect = self.graph.object("UIBlurEffect");
                match blur.attr("style") {
                    Some("systemUltraThinMaterial") => {
                        self.graph.set(effect, "UIBlurEffectMaterialStyle", 6_i64);
                        self.graph.set(effect, "UIBlurEffectAutomaticStyle", 4_i64);
                        self.graph.set(effect, "UIBlurEffectStyle", 1_i64);
                    }
                    Some("light" | "dark" | "extraLight") => {
                        let style = match blur.attr("style") {
                            Some("extraLight") => 0_i64,
                            Some("dark") => 2,
                            _ => 1,
                        };
                        self.graph.set(effect, "UIBlurEffectStyle", style);
                    }
                    other => return Err(format!("unsupported blur style {other:?}")),
                }
                self.graph.set(object, "UIVisualEffectViewEffect", effect);
                self.graph
                    .set(object, "UIVisualEffectViewGroupName", Value::Nil);
                let content = element
                    .children
                    .iter()
                    .find(|e| e.attr("key") == Some("contentView"))
                    .ok_or("a visual effect view has no content view")?;
                let content = self.object_for(content)?;
                self.graph
                    .set(object, "UIVisualEffectViewContentView", content);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn activity_indicator(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let (modern, legacy) = match element.attr("style") {
            Some("large") => (101_i64, 0_i64),
            None | Some("medium") => (100, 1),
            Some("whiteLarge") => (0, 0),
            Some("white") => (1, 1),
            Some("gray") => (2, 2),
            Some(other) => {
                return Err(format!("unsupported activity indicator style {other}"));
            }
        };
        self.graph
            .set(object, "UIActivityIndicatorViewStyle-Modern", modern);
        self.graph
            .set(object, "UIActivityIndicatorViewStyle", legacy);
        self.graph.set(
            object,
            "UIHidesWhenStopped",
            yes(element, "hidesWhenStopped"),
        );
        let color = keyed(element, "color", "color")
            .map(|e| self.color(e))
            .transpose()?;
        self.graph.set(object, "UIColor", color);
        if keyed(element, "color", "tintColor").is_none() {
            let tint = self.system_color(
                "secondaryLabelColor",
                Fallback::Rgb(
                    0.235_294_117_647_058_82,
                    0.235_294_117_647_058_82,
                    0.262_745_098_039_215_7,
                    0.6,
                ),
            );
            self.graph.set(object, "UITintColor", tint);
        }
        Ok(())
    }

    pub fn runtime_attributes(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for attribute in element
            .children_named("userDefinedRuntimeAttributes")
            .flat_map(|attrs| attrs.children.iter())
        {
            let value = match attribute.attr("type") {
                Some("boolean") => self.graph.shared(
                    "NSNumber",
                    vec![(
                        "NS.intval",
                        Value::Int(i64::from(attribute.attr("value").is_some_and(|value| {
                            let value = value.trim_start();
                            value.starts_with(['Y', 'y', 'T', 't'])
                                || value.parse::<f64>().is_ok_and(|n| n != 0.0)
                        }))),
                    )],
                ),
                Some("string") => self.graph.string(attribute.attr("value").unwrap_or("")),
                Some("number") => self.graph.number(
                    number(attribute, "value")?
                        .or_else(|| {
                            attribute
                                .child("real")
                                .and_then(|r| r.attr("value"))
                                .and_then(|r| r.parse().ok())
                        })
                        .ok_or("a runtime number has no value")?,
                ),
                Some("image") => self.image(
                    attribute
                        .attr("value")
                        .ok_or("a runtime image has no value")?,
                    attribute.attr("catalog"),
                    None,
                )?,
                Some("color") => self.color(
                    attribute
                        .child("color")
                        .ok_or("a runtime color has no value")?,
                )?,
                other => return Err(format!("unsupported runtime attribute type {other:?}")),
            };
            let key = self.graph.string(
                attribute
                    .attr("keyPath")
                    .ok_or("a runtime attribute has no key path")?,
            );
            let pair = self.graph.object("UINibKeyValuePair");
            self.graph.set(pair, "UIObject", object);
            self.graph.set(pair, "UIValue", value);
            self.graph.set(pair, "UIKeyPath", key);
            self.key_value_pairs.push(pair);
        }
        Ok(())
    }

    pub fn document_tint(&mut self, object: Id, element: &Element) -> Result<(), String> {
        if keyed(element, "color", "tintColor").is_none()
            && let Some(tint) = keyed(self.document, "color", "tintColor")
        {
            let tint = self.color(tint)?;
            self.graph.set(object, "UITintColor", tint);
        }
        Ok(())
    }
}
