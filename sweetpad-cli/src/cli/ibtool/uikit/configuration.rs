//! Modern button configuration archives, including their background defaults.

use super::{Element, Encoder, Fallback, Id, Value, color_values_precision, components, keyed};

impl Encoder<'_> {
    pub(super) fn button_configuration(
        &mut self,
        element: &Element,
        button: &Element,
    ) -> Result<Id, String> {
        let style = match element.attr("style") {
            None | Some("plain") => 0_i64,
            Some("gray") => 1,
            Some("tinted") => 2,
            Some("filled") => 3,
            other => return Err(format!("unsupported button configuration style {other:?}")),
        };
        let object = self.graph.object("UIButtonConfiguration");
        self.graph
            .set(object, "UIButtonConfigurationButtonStyle", style);
        let background = self.button_background(style, element, button)?;
        self.graph
            .set(object, "UIButtonConfigurationBackground", background);
        let corner = match element.attr("cornerStyle") {
            None | Some("dynamic") => 0_i64,
            Some("small") => 1,
            Some("medium") => 2,
            Some("large") => 3,
            Some("capsule") => 4,
            other => return Err(format!("unsupported corner style {other:?}")),
        };
        self.graph
            .set(object, "UIButtonConfigurationCornerStyle", corner);
        self.graph
            .set(object, "UIButtonConfigurationButtonSize", 0_i64);
        self.graph
            .set(object, "UIButtonConfigurationMacIdiomStyle", 0_i64);
        for (key, archive) in [
            (
                "baseBackgroundColor",
                "UIButtonConfigurationBaseBackgroundColor",
            ),
            (
                "baseForegroundColor",
                "UIButtonConfigurationBaseForegroundColor",
            ),
        ] {
            let color = keyed(element, "color", key)
                .map(|c| self.color(c))
                .transpose()?;
            self.graph.set(object, archive, color);
        }
        let reference = keyed(element, "imageReference", "image");
        let source = reference.unwrap_or(element);
        let image = source
            .attr("image")
            .map(|name| self.image(name, source.attr("catalog"), reference))
            .transpose()?;
        self.graph.set(object, "UIButtonConfigurationImage", image);
        let symbol = element
            .child("preferredSymbolConfiguration")
            .map(|c| self.symbol_configuration(c))
            .transpose()?;
        self.graph.set(
            object,
            "UIButtonConfigurationPreferredSymbolConfigurationForImage",
            symbol,
        );
        self.graph.set(
            object,
            "UIButtonConfigurationSymbolContentTransition",
            Value::Nil,
        );
        self.graph
            .set(object, "UIButtonConfigurationShowsActivityIndicator", false);
        for (key, archive) in [
            ("title", "UIButtonConfigurationTitle"),
            ("subtitle", "UIButtonConfigurationSubtitle"),
        ] {
            let text = element.attr(key).unwrap_or("");
            let text = self.text(button, &format!("configuration.{key}"), text);
            let attributed =
                self.attributed_string(text, element.attr(key).is_some_and(|t| !t.is_empty()));
            self.graph.set(object, archive, attributed);
        }
        self.graph
            .set(object, "UIButtonConfigurationImagePlacement", 2_i64);
        self.graph
            .set(object, "UIButtonConfigurationImagePadding", 0_f32);
        self.graph
            .set(object, "UIButtonConfigurationTitlePadding", 1_f32);
        self.graph
            .set(object, "UIButtonConfigurationTitleAlignment", 0_i64);
        Ok(object)
    }

    fn attributed_string(&mut self, text: Id, nonempty: bool) -> Id {
        let styled = self.graph.object("NSAttributedString");
        self.graph.set(styled, "NSString", text);
        if nonempty {
            let attributes = self.graph.dictionary("NSDictionary", &[]);
            self.graph.set(styled, "NSAttributes", attributes);
        }
        styled
    }

    fn button_background(
        &mut self,
        style: i64,
        configuration: &Element,
        button: &Element,
    ) -> Result<Id, String> {
        let clear = self.system_color("clearColor", Fallback::White(0.0, 0.0));
        let background = match style {
            0 => clear,
            1 => self.graph.object_with_values(
                "UIColor",
                color_values_precision(
                    Fallback::Rgb(120.0 / 255.0, 120.0 / 255.0, 128.0 / 255.0, 0.16),
                    false,
                ),
            ),
            _ => {
                let tint = keyed(button, "color", "tintColor");
                let tint = tint
                    .and_then(|c| c.attr("name"))
                    .and_then(|name| self.named_colors.get(name).copied())
                    .or(tint);
                let tint = tint.map(components).transpose()?.unwrap_or(Fallback::Rgb(
                    0.0,
                    0.533_333_333_333_333_3,
                    1.0,
                    1.0,
                ));
                let tint = match tint {
                    Fallback::Rgb(r, g, b, _) => {
                        Fallback::Rgb(r, g, b, if style == 2 { 0.18 } else { 1.0 })
                    }
                    Fallback::White(w, _) => {
                        Fallback::White(w, if style == 2 { 0.18 } else { 1.0 })
                    }
                };
                // This is a resolved configuration color, rather than an IB color literal.
                self.graph
                    .object_with_values("UIColor", color_values_precision(tint, false))
            }
        };
        Ok(self.background_configuration(
            background,
            if matches!(configuration.attr("cornerStyle"), None | Some("dynamic")) {
                17.0
            } else {
                5.949_999_999_999_999
            },
            true,
        ))
    }

    pub(super) fn background_configuration(
        &mut self,
        background: Id,
        corner: f64,
        button: bool,
    ) -> Id {
        let clear = self.system_color("clearColor", Fallback::White(0.0, 0.0));
        let object = self.graph.object("UIBackgroundConfiguration");
        for key in [
            "customView",
            "visualEffect",
            "visualEffectGroupName",
            "image",
        ] {
            self.graph.set(object, key, Value::Nil);
        }
        self.graph.set(object, "cornerRadius", corner);
        self.graph.set(object, "maskedCorners", 15_i64);
        self.graph.set(object, "backgroundColor", background);
        self.graph.set(object, "strokeColor", clear);
        for key in [
            "backgroundColorTransformerIdentifier",
            "visualEffectblurClippingMode",
            "imageContentMode",
            "edgesAddingLayoutMarginsToBackgroundInsets",
            "strokeColorTransformerIdentifier",
            "strokeLocation",
            "defaultStyle",
            "defaultStyle_v2",
        ] {
            self.graph.set(object, key, 0_i64);
        }
        let insets = self.graph.string("{0, 0, 0, 0}");
        self.graph.set(object, "backgroundInsets", insets);
        self.graph
            .set(object, "strokeWidth", if button { 1.0 } else { 0.0 });
        self.graph.set(object, "strokeOutset", 0.0);
        for key in [
            "hasCustomized-customView",
            "hasCustomized-cornerRadius",
            "hasCustomized-maskedCorners",
            "hasCustomized-backgroundColor",
            "hasCustomized-backgroundColorTransformerIdentifier",
            "hasCustomized-visualEffect",
            "hasCustomized-visualEffectGroupName",
            "hasCustomized-image",
            "hasCustomized-imageContentMode",
            "hasCustomized-backgroundInsets",
            "hasCustomized-edgesAddingLayoutMarginsToBackgroundInsets",
            "hasCustomized-strokeColorTransformerIdentifier",
            "hasCustomized-strokeOutset",
        ] {
            self.graph.set(object, key, false);
        }
        self.graph.set(object, "hasCustomized-strokeColor", button);
        self.graph.set(object, "hasCustomized-strokeWidth", button);
        if !button {
            self.graph.replace(object, "defaultStyle", 1_i64);
            self.graph.replace(object, "defaultStyle_v2", 11_i64);
        }
        object
    }
}
