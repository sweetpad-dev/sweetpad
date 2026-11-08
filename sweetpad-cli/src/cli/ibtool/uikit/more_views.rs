use super::controls::enumeration;
use super::{
    Element, Encoder, Fallback, Id, Value, doubles, integer, keyed, no, number, single, yes,
};

impl Encoder<'_> {
    pub(super) fn more_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        match element.name.as_str() {
            "progressView" => {
                if let Some(value) = number(element, "progress")?
                    && value != 0.0
                {
                    self.graph.set(object, "UIProgress", single(value));
                }
                let style = enumeration(
                    element,
                    "progressViewStyle",
                    &[("default", 0), ("bar", 1)],
                    0,
                )?;
                if style != 0 {
                    self.graph.set(object, "UIProgressViewStyle", style);
                }
                self.control_colors(
                    element,
                    object,
                    &[
                        ("progressTintColor", "UIProgressTintColor"),
                        ("trackTintColor", "UITrackTintColor"),
                    ],
                )?;
            }
            "stepper" => self.stepper(element, object)?,
            "pageControl" => {
                self.control(element, object)?;
                if let Some(pages) = integer(element, "numberOfPages")?
                    && pages != 0
                {
                    self.graph.set(object, "UINumberOfPages", pages);
                }
                self.graph.set(
                    object,
                    "UICurrentPage",
                    integer(element, "currentPage")?.unwrap_or(0),
                );
                for (attribute, key) in [
                    ("hidesForSinglePage", "UIHideForSinglePage"),
                    ("defersCurrentPageDisplay", "UIDefersCurrentPageDisplay"),
                ] {
                    if yes(element, attribute) {
                        self.graph.set(object, key, true);
                    }
                }
                self.control_colors(
                    element,
                    object,
                    &[
                        ("pageIndicatorTintColor", "UIPageIndicatorTintColor"),
                        (
                            "currentPageIndicatorTintColor",
                            "UICurrentPageIndicatorTintColor",
                        ),
                    ],
                )?;
            }
            "segmentedControl" => self.segmented_control(element, object)?,
            "textView" => self.text_view(element, object)?,
            "searchBar" => self.search_bar(element, object)?,
            "toolbar" | "tabBar" => {
                let mut items = Vec::new();
                if let Some(list) = element.child("items") {
                    for item in &list.children {
                        items.push(match item.name.as_str() {
                            "barButtonItem" => self.bar_button(item)?,
                            "tabBarItem" => self.tab_item(item)?,
                            other => {
                                return Err(format!("unsupported {} item <{other}>", element.name));
                            }
                        });
                    }
                }
                let items = self.graph.array("NSArray", &items);
                self.graph.set(object, "UIItems", items);
                if element.name == "toolbar" {
                    self.graph.set(object, "UIBarPosition", 1_i64);
                }
                self.graph.set(
                    object,
                    "UIBarTranslucence",
                    i64::from(!no(element, "translucent")),
                );
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    // Exact defaults determine whether a field is present in an archive.
    #[allow(clippy::float_cmp)]
    fn stepper(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.control(element, object)?;
        for (attribute, key, default, always) in [
            ("value", "UIValue", 0.0, false),
            ("minimumValue", "UIMinimumValue", 0.0, false),
            ("maximumValue", "UIMaximumValue", 0.0, true),
            ("stepValue", "UIStepValue", 1.0, false),
        ] {
            let value = number(element, attribute)?.unwrap_or(default);
            if always || value != default {
                self.graph.set(object, key, value);
            }
        }
        for (attribute, key, default) in [
            ("continuous", "UIContinuous", true),
            ("autorepeat", "UIAutorepeat", true),
            ("wraps", "UIWraps", false),
        ] {
            let value = element.attr(attribute).map_or(default, |v| v == "YES");
            if value != default {
                self.graph.set(object, key, value);
            }
        }
        Ok(())
    }

    fn search_bar(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.graph.set(
            object,
            "UISearchBarTranslucence",
            i64::from(!no(element, "translucent")),
        );
        let text = self.text(element, "text", element.attr("text").unwrap_or(""));
        self.graph.set(object, "UIText", text);
        if let Some(placeholder) = element.attr("placeholder") {
            let text = self.text(element, "placeholder", placeholder);
            self.graph.set(object, "UIPlaceholder", text);
        }
        if let Some(traits) = element.child("textInputTraits") {
            self.text_traits(traits, object)?;
        }
        for key in ["UIEnablesReturnKeyAutomatically", "UISecureTextEntry"] {
            if self.graph.value(object, key).is_none() {
                self.graph.set(object, key, false);
            }
        }
        Ok(())
    }

    fn control_colors(
        &mut self,
        element: &Element,
        object: Id,
        colors: &[(&str, &str)],
    ) -> Result<(), String> {
        for &(attribute, key) in colors {
            if let Some(color) = keyed(element, "color", attribute) {
                let color = self.color(color)?;
                self.graph.set(object, key, color);
            }
        }
        Ok(())
    }

    fn segmented_control(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.control(element, object)?;
        self.graph.replace(
            object,
            "UIPointerInteractionEnabled",
            !no(element, "pointerInteractionEnabled"),
        );
        if yes(element, "momentary") {
            self.graph.set(object, "UIMomentary", true);
        }
        let mut segments = Vec::new();
        if let Some(list) = element.child("segments") {
            for (index, segment) in list.children_named("segment").enumerate() {
                let item = self.graph.object("UISegment");
                self.graph
                    .set(item, "UIBounds", doubles(&[0.0, 0.0, 0.0, 32.0]));
                self.graph.set(item, "UICenter", doubles(&[0.0, 16.0]));
                self.graph.set(item, "UIDeepDrawRect", true);
                self.graph.set(item, "UIAutoresizeSubviews", true);
                self.graph
                    .set(item, "UIViewSemanticContentAttribute", 0_i64);
                self.large_content_metadata(item);
                if let Some(title) = segment.attr("title") {
                    let title = self.text(element, &format!("segmentTitles[{index}]"), title);
                    self.graph.set(item, "UISegmentInfo", title);
                } else if let Some(image) = segment.attr("image") {
                    let image = self.image(image, segment.attr("catalog"), None)?;
                    self.graph.set(item, "UISegmentInfo", image);
                }
                if let Some(width) = number(segment, "width")?
                    && width != 0.0
                {
                    self.graph.set(item, "UISegmentWidth", single(width));
                }
                self.graph.set(item, "UISegmentStyle", 5_i64);
                if index > 0 {
                    self.graph.set(
                        item,
                        "UISegmentPosition",
                        if index + 1 == list.children.len() {
                            3_i64
                        } else {
                            2_i64
                        },
                    );
                }
                segments.push(item);
            }
        }
        let views = self.graph.array("NSMutableArray", &segments);
        self.graph.set(object, "UISubviews", views);
        let segments = self.graph.array("NSMutableArray", &segments);
        self.graph.set(object, "UISegments", segments);
        self.graph.set(
            object,
            "UISelectedSegmentIndex",
            integer(element, "selectedSegmentIndex")?.unwrap_or(-1),
        );
        Ok(())
    }

    fn text_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.scroll_view(element, object);
        if let Some(detectors) = element.child("dataDetectorType") {
            let flags = [
                ("phoneNumber", 1_i64),
                ("link", 2),
                ("address", 4),
                ("calendarEvent", 8),
                ("shipmentTrackingNumber", 16),
                ("flightNumber", 32),
                ("lookupSuggestion", 64),
                ("money", 128),
                ("physicalValue", 256),
            ];
            let value = if yes(detectors, "all") {
                -1
            } else {
                flags
                    .into_iter()
                    .filter(|(name, _)| yes(detectors, name))
                    .map(|(_, flag)| flag)
                    .sum()
            };
            self.graph.set(object, "UIDataDetectorTypes", value);
        }
        let text = element.attr("text").unwrap_or("");
        let string = self.text(element, "text", text);
        self.graph.set(object, "UIText", string);
        let font = self.font(element.child("fontDescription"))?;
        let paragraph = self.graph.object("NSMutableParagraphStyle");
        self.graph.set(paragraph, "NSTabStops", Value::Nil);
        let font_key = self.graph.string("NSFont");
        let paragraph_key = self.graph.string("NSParagraphStyle");
        let attributes = self.graph.dictionary(
            "NSDictionary",
            &[(font_key, font), (paragraph_key, paragraph)],
        );
        let styled = self.graph.object("NSAttributedString");
        let plain = self.graph.string(text);
        self.graph.set(styled, "NSString", plain);
        self.graph.set(styled, "NSAttributes", attributes);
        self.graph.set(object, "UIAttributedText", styled);
        self.graph.set(object, "UIFont", font);
        self.graph.set(
            object,
            "UITextAlignment",
            enumeration(
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
            )?,
        );
        if let Some(traits) = element.child("textInputTraits") {
            self.text_traits(traits, object)?;
        }
        if self
            .graph
            .value(object, "UIAutocapitalizationType")
            .is_none()
        {
            self.graph.set(object, "UIAutocapitalizationType", 0_i64);
        }
        let color = self.system_color(
            "systemBlueColor",
            Fallback::Rgb(0.0, 0.533_333_333_3, 1.0, 1.0),
        );
        let outline = self.system_color(
            "secondarySystemFillColor",
            Fallback::Rgb(0.470_588_235_3, 0.470_588_235_3, 0.501_960_784_3, 0.16),
        );
        let color_key = self.graph.string("NSColor");
        let outline_key = self.graph.string("NSTextHighlightOutlineColor");
        let highlights = self.graph.dictionary(
            "NSDictionary",
            &[(color_key, color), (outline_key, outline)],
        );
        self.graph
            .set(object, "UITextHighlightAttributes", highlights);
        self.graph.set(object, "UITextAllowsNumberPadPopover", true);
        for (attribute, key) in [("editable", "UIEditable"), ("selectable", "UISelectable")] {
            if no(element, attribute) {
                self.graph.set(object, key, false);
            }
        }
        Ok(())
    }
}
