use super::{Element, Encoder, Id, Value, no, number, yes};

pub(super) fn enumeration(
    element: &Element,
    key: &str,
    values: &[(&str, i64)],
    default: i64,
) -> Result<i64, String> {
    element.attr(key).map_or(Ok(default), |value| {
        values
            .iter()
            .find(|(name, _)| *name == value)
            .map(|(_, n)| *n)
            .ok_or_else(|| format!("unsupported {key}: {value}"))
    })
}

fn signed32(value: i64) -> i64 {
    if value >= 2_147_483_648 {
        value - 4_294_967_296
    } else {
        value
    }
}

fn button_type(cell: &Element) -> i64 {
    let Some(behavior) = cell.child("behavior") else {
        return 0;
    };
    let all = |keys: &[&str]| keys.iter().all(|key| yes(behavior, key));
    let none = |keys: &[&str]| keys.iter().all(|key| !yes(behavior, key));
    if all(&["pushIn", "lightByBackground", "lightByGray"])
        && none(&[
            "changeContents",
            "changeBackground",
            "changeGray",
            "lightByContents",
            "doesNotDimImage",
        ])
    {
        return 7;
    }
    if all(&["changeContents", "lightByContents", "doesNotDimImage"])
        && none(&[
            "pushIn",
            "lightByBackground",
            "lightByGray",
            "changeBackground",
            "changeGray",
        ])
    {
        return match cell.attr("type") {
            Some("check") => 3,
            Some("radio") => 4,
            _ => 0,
        };
    }
    if all(&[
        "pushIn",
        "lightByBackground",
        "lightByGray",
        "changeBackground",
        "changeGray",
    ]) {
        return 1;
    }
    0
}

pub(super) fn cell_flags(
    element: &Element,
    cell: &Element,
    alignment: i64,
    line_break: i64,
    size: i64,
) -> (i64, i64) {
    let mut flags = 1_i64 << 26;
    if yes(cell, "editable") {
        flags |= (1 << 28) | (1 << 21);
        if matches!(cell.name.as_str(), "textFieldCell" | "secureTextFieldCell") {
            flags |= 1;
        }
    }
    if yes(cell, "selectable") {
        flags |= (1 << 21) | 1;
    }
    if yes(cell, "scrollable") && line_break < 3 {
        flags |= 1 << 20;
    }
    if no(cell, "enabled") {
        flags |= 1 << 29;
    }
    if matches!(cell.attr("state"), Some("on" | "mixed")) {
        flags |= 1 << 31;
    }
    flags |= match cell.attr("borderStyle") {
        Some("bezel" | "borderAndBezel") => 1 << 22,
        Some("border") if !matches!(element.name.as_str(), "button" | "popUpButton") => 1 << 23,
        _ => 0,
    };
    if line_break != 0 {
        flags |= 1 << 6;
    }
    let mut flags2 = [0_i64, 2, 1, 3, 4][usize::try_from(alignment).unwrap()] << 26;
    flags2 |= i64::from(yes(cell, "refusesFirstResponder")) << 25;
    flags2 |= i64::from(yes(cell, "usesSingleLineMode")) << 6;
    flags2 |= line_break << 9;
    flags2 |= i64::from(yes(cell, "sendsActionOnEndEditing")) << 22;
    flags2 |= i64::from(no(cell, "allowsUndo")) << 12;
    flags2 |= i64::from(yes(cell, "allowsMixedState")) << 24;
    flags2 |= i64::from(yes(cell, "allowsMixedState") && cell.attr("state") == Some("mixed")) << 23;
    flags2 |= (if size > 2 { 0 } else { size }) << 17;
    (signed32(flags), signed32(flags2))
}

impl Encoder<'_> {
    pub(super) fn control(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let cell = element
            .children
            .iter()
            .find(|e| e.attr("key") == Some("cell"))
            .ok_or("an AppKit control has no cell")?;
        let cell_object = self.object(cell)?;
        if let Some(identifier) = element.attr("identifier") {
            self.string(cell_object, "NSCellIdentifier", identifier);
        }
        self.owners.push((cell_object, object));
        self.oids.push(cell_object);
        let alignment = enumeration(
            cell,
            "alignment",
            &[
                ("left", 0),
                ("center", 1),
                ("right", 2),
                ("justified", 3),
                ("natural", 4),
            ],
            if element.name == "popUpButton" { 0 } else { 4 },
        )?;
        let line_break = enumeration(
            cell,
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
        let size = enumeration(
            cell,
            "controlSize",
            &[("regular", 0), ("small", 1), ("mini", 2), ("large", 3)],
            0,
        )?;
        self.graph.set(object, "NSEnabled", !no(element, "enabled"));
        self.graph.set(object, "NSCell", cell_object);
        for (key, value) in [
            ("NSAllowsLogicalLayoutDirection", self.options.localized),
            ("NSControlContinuous", yes(cell, "continuous")),
            (
                "NSControlRefusesFirstResponder",
                yes(cell, "refusesFirstResponder"),
            ),
            (
                "NSControlUsesSingleLineMode",
                yes(cell, "usesSingleLineMode"),
            ),
        ] {
            self.graph.set(object, key, value);
        }
        self.graph
            .set(object, "NSControlSize", if size > 2 { 0_i64 } else { size });
        if size > 2 {
            for target in [object, cell_object] {
                self.graph.set(target, "NSControlSize2", size);
                self.graph.set(target, "NSControlSizeExtraLarge", false);
            }
        }
        self.graph.set(object, "NSControlTextAlignment", alignment);
        self.graph.set(object, "NSControlLineBreakMode", line_break);
        self.graph.set(object, "NSControlWritingDirection", -1_i64);
        self.graph.set(object, "NSControlSendActionMask", 4_i64);
        let (flags, flags2) = cell_flags(element, cell, alignment, line_break, size);
        self.graph.set(cell_object, "NSCellFlags", flags);
        self.graph.set(cell_object, "NSCellFlags2", flags2);
        let title = super::string_value(cell, "title")?.unwrap_or_default();
        let title = if element.name == "popUpButton" {
            self.graph.string(&title)
        } else {
            self.text(cell, "title", &title)
        };
        self.graph.set(cell_object, "NSContents", title);
        let font = self.cell_font(cell)?;
        self.graph.set(cell_object, "NSSupport", font);
        self.graph.set(cell_object, "NSControlView", object);
        self.content_tint(element, cell_object)?;

        match element.name.as_str() {
            "textField" | "secureTextField" => {
                self.text_field(element, cell, cell_object, object)?;
            }
            "button" | "popUpButton" => {
                self.button(cell, cell_object, object)?;
                if element.name == "popUpButton" {
                    self.popup(cell, cell_object)?;
                }
            }
            other => return Err(format!("unsupported AppKit control {other}")),
        }
        Ok(())
    }

    fn content_tint(&mut self, element: &Element, cell_object: Id) -> Result<(), String> {
        if let Some(color) = element
            .children_named("color")
            .find(|c| c.attr("key") == Some("contentTintColor"))
        {
            let color = self.color(color)?;
            self.graph.set(cell_object, "NSContentTintColor", color);
        }
        Ok(())
    }

    pub(super) fn cell_font(&mut self, cell: &Element) -> Result<Id, String> {
        let font = cell.child("font").ok_or("an AppKit cell has no font")?;
        let mut font = font.clone();
        if cell.name == "popUpButtonCell" && font.attr("metaFont") == Some("menu") {
            font.attributes
                .iter_mut()
                .find(|(key, _)| key == "metaFont")
                .unwrap()
                .1 = "message".into();
        }
        if cell.attr("bezelStyle") == Some("roundedRect") && yes(&font, "usesAppearanceFont") {
            let name = self.graph.string(".AppleSystemUIFont");
            return Ok(self.graph.object_with_values(
                "NSFont",
                vec![
                    ("NSName", name.into()),
                    ("NSSize", Value::Double(13.0)),
                    ("NSfFlags", Value::Int(1044)),
                ],
            ));
        }
        self.font(&font)
    }

    pub(super) fn text_cell_properties(
        &mut self,
        cell: &Element,
        cell_object: Id,
    ) -> Result<(), String> {
        for (xml, key) in [
            ("backgroundColor", "NSBackgroundColor"),
            ("textColor", "NSTextColor"),
        ] {
            if let Some(color) = cell
                .children_named("color")
                .find(|c| c.attr("key") == Some(xml))
            {
                let color = if xml == "textColor"
                    && color.attr("name") == Some("textColor")
                    && yes(cell, "editable")
                {
                    self.named_color("controlTextColor")?
                } else {
                    self.color(color)?
                };
                self.graph.set(cell_object, key, color);
            }
        }
        self.graph.set(cell_object, "NSControlBorderShape", 2_i64);
        if yes(cell, "drawsBackground") {
            self.graph.set(cell_object, "NSDrawsBackground", true);
        }
        if let Some(text) = cell.attr("placeholderString") {
            let text = self.text(cell, "placeholderString", text);
            self.graph.set(cell_object, "NSPlaceholderString", text);
        }
        if cell.name == "secureTextFieldCell" {
            let locales = if let Some(list) = cell.child("allowedInputSourceLocales") {
                list.children
                    .iter()
                    .map(|s| self.graph.string(&s.text))
                    .collect::<Vec<_>>()
            } else {
                vec![self.graph.string("NSAllRomanInputSourcesLocaleIdentifier")]
            };
            let locales = self.graph.array("NSArray", &locales);
            self.graph
                .set(cell_object, "NSAllowedInputLocales", locales);
        }
        Ok(())
    }

    fn text_field(
        &mut self,
        element: &Element,
        cell: &Element,
        cell_object: Id,
        object: Id,
    ) -> Result<(), String> {
        let constrained = element
            .attr("id")
            .is_some_and(|id| self.constrained.contains(id));
        if yes(element, "ibtoolTablePrototype")
            || (self.storyboard && constrained && !yes(cell, "editable"))
        {
            self.graph
                .set(object, "NSTextFieldLineBreakStrategyVersion", 2_i64);
            self.graph.set(object, "NSLineBreakStrategy", 65_535_i64);
        }
        if no(element, "textCompletion") {
            self.graph
                .set(cell_object, "NSAutomaticTextCompletionDisabled", true);
        }
        if yes(cell, "editable")
            && element
                .attr("id")
                .is_some_and(|id| self.baseline_measured.contains(id))
        {
            let children = self.graph.array("NSMutableArray", &[]);
            self.graph.set(object, "NSSubviews", children);
        }
        if let Some(kind) = element.attr("contentType") {
            self.string(object, "NSTextContentType", kind);
        }
        self.text_cell_properties(cell, cell_object)?;
        self.graph
            .set(object, "NSTextFieldAlignmentRectInsetsVersion", 2_i64);
        self.graph.set(
            object,
            "NSAllowsWritingTools",
            cell.name != "secureTextFieldCell",
        );
        self.graph
            .set(object, "NSTextFieldAllowsWritingToolsAffordance", false);
        self.graph.set(
            object,
            "NS.resolvesNaturalAlignmentWithBaseWritingDirection",
            false,
        );
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation)]
    fn button(&mut self, cell: &Element, cell_object: Id, object: Id) -> Result<(), String> {
        let bezel = enumeration(
            cell,
            "bezelStyle",
            &[
                ("rounded", 1),
                ("regularSquare", 2),
                ("smallSquare", 10),
                ("roundedRect", 12),
                ("recessed", 13),
            ],
            1,
        )?;
        let mut flags = 0_i64;
        if let Some(behavior) = cell.child("behavior") {
            flags |= i64::from(yes(behavior, "pushIn")) << 31;
            flags |= i64::from(yes(behavior, "lightByBackground")) << 26;
            flags |= i64::from(yes(behavior, "lightByGray")) << 25;
            flags |= i64::from(yes(behavior, "lightByContents")) << 27;
            flags |= i64::from(yes(behavior, "doesNotDimImage")) << 12;
            flags |= i64::from(yes(behavior, "changeContents")) << 30;
            flags |= i64::from(yes(behavior, "changeBackground")) << 29;
            flags |= i64::from(yes(behavior, "changeGray")) << 28;
        }
        flags |= if matches!(cell.attr("borderStyle"), Some("border" | "borderAndBezel")) {
            1 << 23
        } else {
            0
        };
        flags |= enumeration(
            cell,
            "imagePosition",
            &[
                ("overlaps", 0x0048_0000),
                ("left", 0x0038_0000),
                ("only", 0x0040_0000),
            ],
            0,
        )?;
        flags |= (number(cell, "inset", 0.0)? as i64) << 13;
        if matches!(cell.attr("type"), Some("check" | "radio")) {
            flags |= 256;
        }
        self.graph
            .set(cell_object, "NSButtonFlags", signed32(flags));
        self.graph.set(
            cell_object,
            "NSButtonFlags2",
            (if bezel >= 10 { bezel + 24 } else { bezel })
                | if bezel == 13 { 8 } else { 0 }
                | (i64::from(yes(cell, "transparent")) << 28)
                | cell
                    .children_named("modifierMask")
                    .find(|m| m.attr("key") == Some("keyEquivalentModifierMask"))
                    .map_or(0, |mask| {
                        [
                            "capsLock",
                            "shift",
                            "control",
                            "option",
                            "command",
                            "numericPad",
                            "help",
                            "function",
                        ]
                        .iter()
                        .enumerate()
                        .fold(0_i64, |bits, (i, key)| {
                            bits | (i64::from(yes(mask, key)) << (i + 24))
                        })
                    })
                | if cell.attr("imageScaling") == Some("proportionallyDown") {
                    128
                } else {
                    0
                },
        );
        self.graph.set(cell_object, "NSBezelStyle", bezel);
        self.button_content(cell, cell_object)?;
        self.graph.set(cell_object, "NSPeriodicDelay", 400_i64);
        self.graph.set(cell_object, "NSPeriodicInterval", 75_i64);
        self.graph
            .set(cell_object, "NSAuxButtonType", button_type(cell));
        if matches!(cell.attr("type"), Some("check" | "radio")) {
            let name = self.graph.string(if cell.attr("type") == Some("check") {
                "NSSwitch"
            } else {
                "NSRadioButton"
            });
            let image = self
                .graph
                .shared("NSButtonImageSource", vec![("NSImageName", name.into())]);
            self.graph.set(cell_object, "NSAlternateImage", image);
        }
        self.graph
            .set(object, "IBNSShadowedSymbolConfiguration", Value::Nil);
        Ok(())
    }

    fn button_content(&mut self, cell: &Element, cell_object: Id) -> Result<(), String> {
        for (attribute, key) in [
            ("image", "NSNormalImage"),
            ("alternateImage", "NSAlternateImage"),
        ] {
            if let Some(name) = cell.attr(attribute) {
                let image = self.image(name);
                self.graph.set(cell_object, key, image);
            }
        }

        for (property, key) in [
            ("alternateTitle", "NSAlternateContents"),
            ("keyEquivalent", "NSKeyEquivalent"),
        ] {
            let value = super::string_value(cell, property)?.unwrap_or_default();
            self.string(cell_object, key, &value);
        }

        Ok(())
    }

    pub(super) fn progress(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let size = enumeration(
            element,
            "controlSize",
            &[("regular", 0), ("small", 1), ("mini", 2), ("large", 3)],
            0,
        )?;
        let mut flags = 16_384_i64 | 4 | (size << 8);
        if !no(element, "bezeled") {
            flags |= 1;
        }
        if yes(element, "indeterminate") {
            flags |= 2;
        }
        if element.attr("style") == Some("spinning") {
            flags |= 4096;
        }
        if no(element, "displayedWhenStopped") {
            flags |= 8192;
        }
        self.graph.set(object, "NSpiFlags", flags);
        let min = number(element, "minValue", 0.0)?;
        if min != 0.0 {
            self.graph.set(object, "NSMinValue", min);
            self.graph
                .set(object, "NSProgressIndicatorDoubleValue", min);
        }
        let max = number(element, "maxValue", 0.0)?;
        if max != 0.0 {
            self.graph.set(object, "NSMaxValue", max);
        }
        Ok(())
    }
}
