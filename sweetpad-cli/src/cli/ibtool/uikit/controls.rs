//! Common UIKit controls and layout containers, archived against ibtool probes.

use super::{Element, Encoder, Fallback, Id, Value, doubles, keyed, no, number, single, yes};

pub(super) fn enumeration(
    element: &Element,
    attribute: &str,
    values: &[(&str, i64)],
    default: i64,
) -> Result<i64, String> {
    element.attr(attribute).map_or(Ok(default), |value| {
        values
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, value)| *value)
            .ok_or_else(|| format!("unknown <{}> {attribute}={value}", element.name))
    })
}

impl Encoder<'_> {
    pub(super) fn cell_separator(
        &mut self,
        element: &Element,
        width: f64,
        height: f64,
    ) -> Result<Id, String> {
        let inset = keyed(element, "inset", "separatorInset");
        let left = 8.0
            + inset
                .map(|e| number(e, "minX"))
                .transpose()?
                .flatten()
                .unwrap_or(0.0);
        let right = inset
            .map(|e| number(e, "maxX"))
            .transpose()?
            .flatten()
            .unwrap_or(0.0);
        let object = self.graph.object("_UITableViewCellSeparatorView");
        self.graph.set(
            object,
            "UIBounds",
            doubles(&[0.0, 0.0, width - left - right, 1.0]),
        );
        self.graph.set(
            object,
            "UICenter",
            doubles(&[(width + left - right) / 2.0, height - 0.5]),
        );
        self.graph.set(object, "UIDeepDrawRect", true);
        self.graph.set(object, "UIOpaque", true);
        self.graph.set(object, "UIAutoresizeSubviews", true);
        self.graph
            .set(object, "UIViewSemanticContentAttribute", 0_i64);
        self.large_content_metadata(object);
        Ok(object)
    }

    pub(super) fn table_cell(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(identifier) = element.attr("reuseIdentifier") {
            let identifier = self.graph.string(identifier);
            self.graph.set(object, "UIReuseIdentifier", identifier);
        }
        let style = enumeration(
            element,
            "style",
            &[
                ("IBUITableViewCellStyleDefault", 0),
                ("IBUITableViewCellStyleValue1", 1),
                ("IBUITableViewCellStyleValue2", 2),
                ("IBUITableViewCellStyleSubtitle", 3),
            ],
            0,
        )?;
        if style != 0 {
            self.graph.set(object, "UITableViewCellStyle", style);
        }
        if let Some(level) = super::integer(element, "indentationLevel")?
            && level != 0
        {
            self.graph.set(object, "UIIndentationLevel", level);
        }
        if keyed(element, "color", "backgroundColor").is_some() {
            self.graph
                .set(object, "UITableCellBackgroundColorSet", true);
        }
        let accessory = enumeration(
            element,
            "accessoryType",
            &[
                ("none", 0),
                ("disclosureIndicator", 1),
                ("detailDisclosureButton", 2),
                ("checkmark", 3),
                ("detailButton", 4),
            ],
            0,
        )?;
        if accessory != 0 {
            self.graph.set(object, "UIAccessoryType", accessory);
        }
        let selection = enumeration(
            element,
            "selectionStyle",
            &[("none", 0), ("blue", 1), ("gray", 2), ("default", 3)],
            3,
        )?;
        if selection != 3 {
            self.graph.set(object, "UISelectionStyle", selection);
        }
        self.graph.set(
            object,
            "UIIndentationWidth",
            single(number(element, "indentationWidth")?.unwrap_or(10.0)),
        );
        if let Some(inset) = keyed(element, "inset", "separatorInset") {
            let mut values = Vec::new();
            for key in ["minY", "minX", "maxY", "maxX"] {
                values.push(number(inset, key)?.unwrap_or(0.0));
            }
            self.graph.set(object, "UISeparatorInset", doubles(&values));
        }
        self.table_background(element, object, selection)?;
        self.graph
            .set(object, "UIContentConfigurationView", Value::Nil);
        let content = element
            .child("tableViewCellContentView")
            .ok_or("a table cell has no content view")?;
        self.graph
            .set(object, "UIContentView", self.object_for(content)?);
        for (attribute, key) in [
            ("textLabel", "UITextLabel"),
            ("detailTextLabel", "UIDetailTextLabel"),
            ("imageView", "UIImageView"),
        ] {
            let value = element
                .attr(attribute)
                .and_then(|id| self.ids.get(id).copied());
            self.graph.set(object, key, value);
        }
        Ok(())
    }

    fn table_background(
        &mut self,
        element: &Element,
        object: Id,
        selection: i64,
    ) -> Result<(), String> {
        if selection == 0
            && keyed(element, "color", "backgroundColor").is_none()
            && !element
                .children_named("nil")
                .any(|e| e.attr("key") == Some("backgroundColor"))
        {
            let color =
                self.system_color("tableCellPlainBackgroundColor", Fallback::White(1.0, 1.0));
            self.graph.set(object, "UIBackgroundColor", color);
            self.graph
                .set(object, "UIDidEncodeCompatibilityBackgroundColor", true);
            let frame = keyed(element, "rect", "frame").ok_or("a table cell has no frame")?;
            let width = number(frame, "width")?.unwrap_or(0.0);
            let height = number(frame, "height")?.unwrap_or(0.0).ceil();
            let view = self.graph.object("_UISystemBackgroundView");
            self.graph
                .set(view, "UIBounds", doubles(&[0.0, 0.0, width, height]));
            self.graph
                .set(view, "UICenter", doubles(&[width / 2.0, height / 2.0]));
            for key in ["UIDeepDrawRect", "UIOpaque", "UIAutoresizeSubviews"] {
                self.graph.set(view, key, true);
            }
            self.graph
                .set(view, "UIViewSemanticContentAttribute", 0_i64);
            self.large_content_metadata(view);
            let configuration = self.background_configuration(color, 0.0, false);
            self.graph.set(view, "configurationAPI", configuration);
            self.graph.set(object, "UISystemBackgroundView", view);
        } else {
            self.graph.set(object, "UISystemBackgroundView", Value::Nil);
        }
        Ok(())
    }

    pub(super) fn control(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for (attribute, key, values) in [
            (
                "contentHorizontalAlignment",
                "UIContentHorizontalAlignment",
                &[
                    ("center", 0),
                    ("left", 1),
                    ("right", 2),
                    ("fill", 3),
                    ("leading", 4),
                    ("trailing", 5),
                ][..],
            ),
            (
                "contentVerticalAlignment",
                "UIContentVerticalAlignment",
                &[("center", 0), ("top", 1), ("bottom", 2), ("fill", 3)][..],
            ),
        ] {
            let value = enumeration(element, attribute, values, 0)?;
            if value != 0 {
                self.graph.set(object, key, value);
                if key == "UIContentVerticalAlignment" {
                    self.graph.set(object, "UIContentVerticalAlignment2", value);
                }
            }
        }
        if no(element, "enabled") {
            self.graph.set(object, "UIDisabled", true);
        }
        if yes(element, "selected") {
            self.graph.set(object, "UISelected", true);
        }
        if yes(element, "highlighted") {
            self.graph.set(object, "UIHighlighted", true);
        }
        Ok(())
    }

    pub(super) fn button(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.control(element, object)?;
        self.button_states(element, object)?;
        if !no(element, "adjustsImageWhenHighlighted") {
            self.graph
                .set(object, "UIAdjustsImageWhenHighlighted", true);
        }
        if !no(element, "adjustsImageWhenDisabled") {
            self.graph.set(object, "UIAdjustsImageWhenDisabled", true);
        }
        let kind = enumeration(
            element,
            "buttonType",
            &[
                ("custom", 0),
                ("system", 1),
                ("roundedRect", 1),
                ("detailDisclosure", 2),
                ("infoLight", 3),
                ("infoDark", 4),
                ("contactAdd", 5),
            ],
            0,
        )?;
        if kind != 0 {
            self.graph.set(object, "UIButtonType", kind);
        }
        for (key, archive) in [
            ("imageEdgeInsets", "UIImageEdgeInsets"),
            ("titleEdgeInsets", "UITitleEdgeInsets"),
            ("contentEdgeInsets", "UIContentEdgeInsets"),
        ] {
            if let Some(inset) = keyed(element, "inset", key) {
                let values = ["minY", "minX", "maxY", "maxX"]
                    .map(|key| number(inset, key).map(|v| v.unwrap_or(0.0)));
                let values = values.into_iter().collect::<Result<Vec<_>, _>>()?;
                self.graph.set(object, archive, doubles(&values));
            }
        }
        if let Some(configuration) = element.child("buttonConfiguration") {
            let configuration = self.button_configuration(configuration, element)?;
            self.graph
                .set(object, "UIButtonConfiguration", configuration);
        } else {
            let font = self.font_with_default(
                keyed(element, "fontDescription", "fontDescription"),
                // Xcode uses 15 points in storyboards, 17 in XIBs.
                if self.document.child("scenes").is_some() {
                    15.0
                } else {
                    17.0
                },
            )?;
            self.graph.set(object, "UIFont", font);
            self.graph.set(object, "UIButtonConfiguration", Value::Nil);
        }
        Ok(())
    }

    fn button_states(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let mut states = Vec::new();
        for state in element
            .children_named("state")
            .filter(|_| element.child("buttonConfiguration").is_none())
        {
            let value = enumeration(
                state,
                "key",
                &[
                    ("normal", 0),
                    ("highlighted", 1),
                    ("disabled", 2),
                    ("selected", 4),
                ],
                0,
            )?;
            let key = self
                .graph
                .shared("NSNumber", vec![("NS.intval", Value::Int(value))]);
            let content = self.graph.object("UIButtonContent");
            if let Some(title) = state.attr("title") {
                let title = self.text(element, &format!("{value}.title"), title);
                self.graph.set(content, "UITitle", title);
            }
            for (key, archive) in [
                ("titleColor", "UITitleColor"),
                ("titleShadowColor", "UIShadowColor"),
            ] {
                if state
                    .children_named("nil")
                    .any(|e| e.attr("key") == Some(key))
                {
                    continue;
                }
                let color = if let Some(color) = keyed(state, "color", key) {
                    self.color(color)?
                } else if self.document.child("dependencies").is_none() {
                    self.system_color("blackColor", Fallback::White(0.0, 1.0))
                } else {
                    continue;
                };
                self.graph.set(content, archive, color);
            }
            for (key, archive) in [
                ("image", "UIImage"),
                ("backgroundImage", "UIBackgroundImage"),
            ] {
                if let Some(name) = state.attr(key) {
                    let image = self.image(
                        name,
                        state.attr("catalog"),
                        state.child("preferredSymbolConfiguration"),
                    )?;
                    self.graph.set(content, archive, image);
                }
            }
            states.push((key, content));
        }
        if element.child("buttonConfiguration").is_none() {
            let states = self.graph.dictionary("NSMutableDictionary", &states);
            self.graph.set(object, "UIButtonStatefulContent", states);
        }
        Ok(())
    }

    pub(super) fn text_field(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.control(element, object)?;
        let font = self.font(keyed(element, "fontDescription", "fontDescription"))?;
        self.graph.set(object, "UIFont", font);
        for (key, archive) in [("placeholder", "UIPlaceholder"), ("text", "UIText")] {
            if let Some(text) = element.attr(key) {
                let text = self.text(element, key, text);
                self.graph.set(object, archive, text);
            }
        }
        let alignment = enumeration(
            element,
            "textAlignment",
            &[
                ("left", 0),
                ("center", 1),
                ("right", 2),
                ("justified", 3),
                ("natural", 4),
            ],
            0,
        )?;
        self.graph.set(object, "UITextAlignment", alignment);
        let color = if let Some(color) = keyed(element, "color", "textColor") {
            self.color(color)?
        } else {
            self.system_color("labelColor", Fallback::White(0.0, 1.0))
        };
        self.graph.set(object, "UITextColor", color);
        let border = enumeration(
            element,
            "borderStyle",
            &[("none", 0), ("line", 1), ("bezel", 2), ("roundedRect", 3)],
            0,
        )?;
        if border != 0 {
            self.graph.set(object, "UIBorderStyle", border);
        }
        self.graph.set(
            object,
            "UIAdjustsFontSizeToFit",
            !no(element, "adjustsFontSizeToFit"),
        );
        if let Some(size) = number(element, "minimumFontSize")? {
            self.graph.set(object, "UIMinimumFontSize", single(size));
        }
        if yes(element, "adjustsFontForContentSizeCategory") {
            self.graph
                .set(object, "UIAdjustsFontForContentSizeCategory", true);
        }
        self.graph
            .set(object, "UIClearButtonOffset", doubles(&[3.0, 1.0]));
        if let Some(traits) = keyed(element, "textInputTraits", "textInputTraits") {
            self.text_traits(traits, object)?;
        }
        self.graph.set(
            object,
            "UIDisableTextColorUpdateOnTraitCollectionChange",
            !element
                .children_named("nil")
                .any(|e| e.attr("key") == Some("textColor")),
        );
        if border == 3 {
            self.graph
                .set(object, "UIRoundedRectBackgroundCornerRadius", 0.0);
        }
        Ok(())
    }

    pub(super) fn text_traits(&mut self, traits: &Element, object: Id) -> Result<(), String> {
        if let Some(content) = traits.attr("textContentType") {
            let content = self.graph.string(content);
            self.graph.set(object, "UITextContentType", content);
        }
        self.keyboard_traits(traits, object)?;
        let capitalization = enumeration(
            traits,
            "autocapitalizationType",
            &[
                ("none", 0),
                ("words", 1),
                ("sentences", 2),
                ("allCharacters", 3),
            ],
            0,
        )?;
        if capitalization != 0 {
            self.graph
                .set(object, "UIAutocapitalizationType", capitalization);
        }
        for (attribute, key) in [
            ("autocorrectionType", "UIAutocorrectionType"),
            ("spellCheckingType", "UISpellCheckingType"),
        ] {
            let value = enumeration(
                traits,
                attribute,
                &[("default", 0), ("no", 1), ("yes", 2)],
                0,
            )?;
            if value != 0 || (attribute == "autocorrectionType" && yes(traits, "secureTextEntry")) {
                self.graph
                    .set(object, key, if value == 0 { 1 } else { value });
            }
        }
        for (attribute, key) in [
            ("smartQuotesType", "UITextSmartQuotesType"),
            ("smartDashesType", "UITextSmartDashesType"),
            ("smartInsertDeleteType", "UITextSmartInsertDeleteType"),
        ] {
            let value = enumeration(
                traits,
                attribute,
                &[("default", 0), ("no", 1), ("yes", 2)],
                0,
            )?;
            if value != 0 {
                self.graph.set(object, key, value);
            }
        }
        let return_key = enumeration(
            traits,
            "returnKeyType",
            &[
                ("default", 0),
                ("go", 1),
                ("google", 2),
                ("join", 3),
                ("next", 4),
                ("route", 5),
                ("search", 6),
                ("send", 7),
                ("yahoo", 8),
                ("done", 9),
                ("emergencyCall", 10),
                ("continue", 11),
            ],
            0,
        )?;
        if return_key != 0 {
            self.graph.set(object, "UIReturnKeyType", return_key);
        }
        if yes(traits, "enablesReturnKeyAutomatically") {
            self.graph
                .set(object, "UIEnablesReturnKeyAutomatically", true);
        }
        if yes(traits, "secureTextEntry") {
            self.graph.set(object, "UISecureTextEntry", true);
            self.graph.set(object, "UITextInlinePredictionType", 1_i64);
        }
        Ok(())
    }

    fn keyboard_traits(&mut self, traits: &Element, object: Id) -> Result<(), String> {
        let keyboard = enumeration(
            traits,
            "keyboardType",
            &[
                ("default", 0),
                ("ASCIICapable", 1),
                ("numbersAndPunctuation", 2),
                ("URL", 3),
                ("numberPad", 4),
                ("phonePad", 5),
                ("namePhonePad", 6),
                ("emailAddress", 7),
                ("decimalPad", 8),
                ("twitter", 9),
                ("webSearch", 10),
                ("asciiCapableNumberPad", 11),
            ],
            0,
        )?;
        if keyboard != 0 {
            self.graph.set(object, "UIKeyboardType", keyboard);
        }
        Ok(())
    }

    pub(super) fn switch(
        &mut self,
        element: &Element,
        object: Id,
        frame: [f64; 4],
    ) -> Result<(), String> {
        self.control(element, object)?;
        if !no(element, "enabled") {
            self.graph.set(object, "UISwitchEnabled", true);
        }
        if yes(element, "on") {
            self.graph.set(object, "UISwitchOn", true);
        }
        for (key, archive) in [
            ("onTintColor", "UISwitchOnTintColor"),
            ("thumbTintColor", "UISwitchThumbTintColor"),
        ] {
            if let Some(color) = keyed(element, "color", key) {
                let color = self.color(color)?;
                self.graph.set(object, archive, color);
            }
        }
        let value = self.graph.object("NSValue");
        self.graph.set(value, "NS.special", 3_i64);
        let rect = self.graph.string(&format!(
            "{{{{{}, {}}}, {{{}, {}}}}}",
            frame[0], frame[1], frame[2], frame[3]
        ));
        self.graph.set(value, "NS.rectval", rect);
        let pair = self.graph.object("UINibKeyValuePair");
        self.graph.set(pair, "UIObject", object);
        self.graph.set(pair, "UIValue", value);
        let key = self.graph.string("frame");
        self.graph.set(pair, "UIKeyPath", key);
        self.key_value_pairs.push(pair);
        Ok(())
    }

    pub(super) fn scroll_view(&mut self, element: &Element, object: Id) {
        self.graph.set(
            object,
            "UIScrollViewIndicatorInsetAdjustmentBehavior",
            1_i64,
        );
        if !no(element, "bouncesZoom") {
            self.graph.set(object, "UIBouncesZoom", true);
        }
        self.graph
            .set(object, "UIScrollViewContentInsetAdjustmentBehavior", 0_i64);
        for (attribute, key, default) in [
            ("minimumZoomScale", "UIMinimumZoomScale", 1.0_f64),
            ("maximumZoomScale", "UIMaximumZoomScale", 1.0_f64),
        ] {
            if let Ok(Some(value)) = number(element, attribute)
                && value.to_bits() != default.to_bits()
            {
                self.graph.set(object, key, single(value));
            }
        }
        for (attribute, key, default) in [
            ("bounces", "UIBounceEnabled", true),
            ("scrollEnabled", "UIScrollDisabled", true),
            ("pagingEnabled", "UIPagingEnabled", false),
            ("alwaysBounceHorizontal", "UIAlwaysBounceHorizontal", false),
            ("alwaysBounceVertical", "UIAlwaysBounceVertical", false),
            (
                "showsHorizontalScrollIndicator",
                "UIShowsHorizontalScrollIndicator",
                true,
            ),
            (
                "showsVerticalScrollIndicator",
                "UIShowsVerticalScrollIndicator",
                true,
            ),
        ] {
            let value = element.attr(attribute).map_or(default, |v| v == "YES");
            if value != default {
                self.graph.set(
                    object,
                    key,
                    if key == "UIScrollDisabled" {
                        !value
                    } else {
                        value
                    },
                );
            }
        }
    }

    pub(super) fn stack_view(
        &mut self,
        element: &Element,
        object: Id,
        children: &[&Element],
    ) -> Result<(), String> {
        if !children.is_empty() {
            let views: Vec<Id> = children
                .iter()
                .map(|e| self.object_for(e))
                .collect::<Result<_, _>>()?;
            let views = self.graph.array("NSArray", &views);
            self.graph.set(object, "UIStackViewArrangedSubviews", views);
        }
        let axis = enumeration(element, "axis", &[("horizontal", 0), ("vertical", 1)], 0)?;
        self.graph.set(object, "UIStackViewAxis", axis);
        let distribution = enumeration(
            element,
            "distribution",
            &[
                ("fill", 0),
                ("fillEqually", 1),
                ("fillProportionally", 2),
                ("equalSpacing", 3),
                ("equalCentering", 4),
            ],
            0,
        )?;
        self.graph
            .set(object, "UIStackViewDistribution", distribution);
        let alignment = enumeration(
            element,
            "alignment",
            &[
                ("fill", 0),
                ("leading", 1),
                ("top", 1),
                ("firstBaseline", 2),
                ("center", 3),
                ("trailing", 4),
                ("bottom", 4),
                ("lastBaseline", 5),
            ],
            0,
        )?;
        self.graph.set(object, "UIStackViewAlignment", alignment);
        self.graph.set(
            object,
            "UIStackViewSpacing",
            number(element, "spacing")?.unwrap_or(0.0),
        );
        self.graph.set(
            object,
            "UIStackViewBaselineRelative",
            yes(element, "baselineRelativeArrangement"),
        );
        self.graph.set(
            object,
            "UIStackViewLayoutMarginsRelative",
            (yes(element, "layoutMarginsRelativeArrangement")
                || yes(element, "layoutMarginsRelative"))
                && keyed(element, "edgeInsets", "layoutMargins").is_some(),
        );
        Ok(())
    }
}
