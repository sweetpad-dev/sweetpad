//! Navigation items and bars shared by controller archives.

use super::{Element, Encoder, Id, id_of, keyed, no, number};

impl Encoder<'_> {
    pub(super) fn navigation_bar(&mut self, _element: &Element, object: Id) {
        self.graph.set(object, "UIIsLocked", true);
        self.graph.set(object, "UIBarPosition", 2_i64);
        self.graph.set(object, "UIBarTranslucence", 1_i64);
    }

    pub fn navigation_item(&mut self, element: &Element, bar: Option<Id>) -> Result<Id, String> {
        let object = self.graph.object("UINavigationItem");
        self.bind(id_of(element)?, object);
        self.objects.push(object);
        if let Some(title) = element.attr("title") {
            let title = self.text(element, "title", title);
            self.graph.set(object, "UITitle", title);
            let attributes = self.graph.dictionary("NSDictionary", &[]);
            let styled = self.graph.object("NSAttributedString");
            self.graph.set(styled, "NSString", title);
            self.graph.set(styled, "NSAttributes", attributes);
            self.graph.set(object, "UIAttributedTitle", styled);
        }
        for (key, single, multiple) in [
            (
                "leftBarButtonItem",
                "UILeftBarButtonItem",
                "UILeftBarButtonItems",
            ),
            (
                "rightBarButtonItem",
                "UIRightBarButtonItem",
                "UIRightBarButtonItems",
            ),
        ] {
            let list_key = if key == "leftBarButtonItem" {
                "leftBarButtonItems"
            } else {
                "rightBarButtonItems"
            };
            let mut buttons = Vec::new();
            if let Some(button) = keyed(element, "barButtonItem", key) {
                buttons.push(self.bar_button(button)?);
            }
            if let Some(list) = element
                .children
                .iter()
                .find(|c| c.attr("key") == Some(list_key) || c.name == list_key)
            {
                for button in list.children_named("barButtonItem") {
                    buttons.push(self.bar_button(button)?);
                }
            }
            if let Some(first) = buttons.first() {
                self.graph.set(object, single, *first);
                let array = self.graph.array("NSArray", &buttons);
                self.graph.set(object, multiple, array);
            }
        }
        let empty = self.graph.array("NSArray", &[]);
        self.graph.set(object, "UICenterItemGroups", empty);
        let large = match element.attr("largeTitleDisplayMode") {
            None | Some("automatic") => 0_i64,
            Some("always") => 1,
            Some("never") => 2,
            other => return Err(format!("unsupported large title mode {other:?}")),
        };
        self.graph.set(object, "UILargeTitleDisplayMode", large);
        if let Some(bar) = bar {
            self.graph.set(object, "UINavigationBar", bar);
        }
        Ok(object)
    }

    pub fn tab_item(&mut self, element: &Element) -> Result<Id, String> {
        let object = self.graph.object("UITabBarItem");
        self.bind(id_of(element)?, object);
        self.objects.push(object);
        if !no(element, "enabled") {
            self.graph.set(object, "UIEnabled", true);
        }
        if let Some(tag) = super::integer(element, "tag")? {
            self.graph.set(object, "UITag", tag);
        }
        if element.attr("systemItem").is_some() {
            let value = super::controls::enumeration(
                element,
                "systemItem",
                &[
                    ("more", 0),
                    ("favorites", 1),
                    ("featured", 2),
                    ("topRated", 3),
                    ("recents", 4),
                    ("contacts", 5),
                    ("history", 6),
                    ("bookmarks", 7),
                    ("search", 8),
                    ("downloads", 9),
                    ("mostRecent", 10),
                    ("mostViewed", 11),
                ],
                0,
            )?;
            self.graph.set(object, "UISystemItem", value);
        }
        for (attribute, key) in [("title", "UITitle"), ("badgeValue", "UIBadgeValue")] {
            if let Some(text) = element.attr(attribute) {
                let text = self.text(element, attribute, text);
                self.graph.set(object, key, text);
            }
        }
        for (attribute, key) in [("image", "UIImage"), ("selectedImage", "UISelectedImage")] {
            if let Some(name) = element.attr(attribute) {
                let image = self.image(name, element.attr("catalog"), None)?;
                self.graph.set(object, key, image);
            }
        }
        self.runtime_attributes(element, object)?;
        Ok(object)
    }

    pub fn bar_button(&mut self, element: &Element) -> Result<Id, String> {
        let object = self.graph.object("UIBarButtonItem");
        self.bind(id_of(element)?, object);
        self.objects.push(object);
        if let Some(view) = element
            .children
            .iter()
            .find(|v| v.attr("key") == Some("customView"))
        {
            let view = self.root_view(view)?;
            self.graph.set(object, "UICustomView", view);
        }
        if !no(element, "enabled") {
            self.graph.set(object, "UIEnabled", true);
        }
        if let Some(item) = element.attr("systemItem") {
            let value = match item {
                "done" => 0_i64,
                "cancel" => 1,
                "edit" => 2,
                "save" => 3,
                "add" => 4,
                "flexibleSpace" => 5,
                "fixedSpace" => 6,
                "compose" => 7,
                "reply" => 8,
                "action" => 9,
                "organize" => 10,
                "bookmarks" => 11,
                "search" => 12,
                "refresh" => 13,
                "stop" => 14,
                "camera" => 15,
                "trash" => 16,
                "play" => 17,
                "pause" => 18,
                "rewind" => 19,
                "fastForward" => 20,
                "undo" => 21,
                "redo" => 22,
                other => return Err(format!("unsupported bar button system item {other}")),
            };
            self.graph.set(object, "UIIsSystemItem", true);
            self.graph.set(object, "UISystemItem", value);
        }
        let style = match element.attr("style") {
            Some("plain") => 0_i64,
            Some("done") => 2,
            None => 1,
            other => return Err(format!("unsupported bar button style {other:?}")),
        };
        if style != 0 {
            self.graph.set(object, "UIStyle", style);
        }
        if element.attr("systemItem") == Some("flexibleSpace") {
            self.graph.set(object, "UIHidesSharedBG", true);
        }
        if let Some(title) = element.attr("title") {
            let title = self.text(element, "title", title);
            self.graph.set(object, "UITitle", title);
        }
        if let Some(name) = element.attr("image") {
            let image = self.image(
                name,
                element.attr("catalog"),
                element.child("preferredSymbolConfiguration"),
            )?;
            self.graph.set(object, "UIImage", image);
        }
        if let Some(width) = number(element, "width")? {
            self.graph.set(object, "UIWidth", super::single(width));
        }
        self.runtime_attributes(element, object)?;
        Ok(object)
    }
}
