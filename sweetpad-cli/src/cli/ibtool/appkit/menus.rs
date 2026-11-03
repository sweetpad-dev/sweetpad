use super::{Element, Encoder, Id, Value, no, yes};

impl Encoder<'_> {
    pub(super) fn main_menu_items(&mut self, menu: Id) {
        if let Some(Value::Object(items)) = self.graph.value(menu, "NSMenuItems") {
            for item in self.graph.references(items) {
                if let Some(Value::Object(submenu)) = self.graph.value(item, "NSSubmenu") {
                    self.graph.set(item, "NSAllowedForLimitedAppMode", true);
                    self.main_menu_items(submenu);
                }
            }
        }
    }

    pub(super) fn popup(&mut self, cell: &Element, object: Id) -> Result<(), String> {
        let menu = cell.child("menu").ok_or("a popup cell has no menu")?;
        let items = menu.child("items").map_or(&[][..], |list| &list.children);
        let selected_index = cell
            .attr("selectedItem")
            .and_then(|selected| {
                items
                    .iter()
                    .position(|item| item.attr("id") == Some(selected))
            })
            .or_else(|| (!items.is_empty()).then_some(0));
        let menu = self.menu(menu, object, Some(object), yes(cell, "pullsDown"))?;
        if let Some(index) = selected_index {
            let selected = self.object(&items[index])?;
            self.graph.set(object, "NSMenuItem", selected);
            if !yes(cell, "pullsDown") && !no(cell, "altersStateOfSelectedItem") {
                for (item_index, item) in items.iter().enumerate() {
                    let item = self.object(item)?;
                    self.graph.remove(item, "NSState");
                    if item_index == index {
                        self.graph.set(item, "NSState", 1_i64);
                    }
                }
                if let Some(Value::Int(flags)) = self.graph.value(object, "NSCellFlags") {
                    let flags = flags | 0x8000_0000;
                    self.graph.replace(
                        object,
                        "NSCellFlags",
                        if flags >= 0x8000_0000 {
                            flags - 0x1_0000_0000
                        } else {
                            flags
                        },
                    );
                }
            }
            let archived_index = if yes(cell, "pullsDown") && cell.attr("selectedItem").is_none() {
                if self.storyboard {
                    i64::try_from(index + 1).unwrap()
                } else {
                    -1
                }
            } else {
                i64::try_from(index).unwrap()
            };
            if archived_index != 0 {
                self.graph.set(object, "NSSelectedIndex", archived_index);
            }
        }
        self.graph.set(object, "NSMenuItemRespectAlignment", true);
        self.graph.set(object, "NSMenu", menu);
        if yes(cell, "pullsDown") {
            self.graph.set(object, "NSPullDown", true);
        }
        let edge = match cell.attr("preferredEdge") {
            None | Some("minY") => 1_i64,
            Some("minX") => 0,
            Some("maxX") => 2,
            Some("maxY") => 3,
            Some(other) => return Err(format!("unsupported popup edge {other}")),
        };
        self.graph.set(object, "NSPreferredEdge", edge);
        self.graph
            .set(object, "NSUsesItemFromMenu", !no(cell, "usesItemFromMenu"));
        if !no(cell, "altersStateOfSelectedItem") {
            self.graph.set(object, "NSAltersState", true);
        }
        self.graph.set(object, "NSArrowPosition", 2_i64);
        Ok(())
    }

    pub(super) fn menu(
        &mut self,
        element: &Element,
        parent: Id,
        popup: Option<Id>,
        pulls_down: bool,
    ) -> Result<Id, String> {
        let object = self.object(element)?;
        self.owners.push((object, parent));
        self.oids.push(object);
        let title = super::string_value(element, "title")?.unwrap_or_default();
        let title = self.text(element, "title", &title);
        self.graph.set(object, "NSTitle", title);
        let mut items = Vec::new();
        if let Some(list) = element.child("items") {
            for (index, item) in list.children.iter().enumerate() {
                if item.name != "menuItem" {
                    return Err(format!("unsupported menu child <{}>", item.name));
                }
                items.push(self.menu_item(item, object, popup, pulls_down && index == 0)?);
            }
        }
        let items = self.graph.array("NSMutableArray", &items);
        self.graph.set(object, "NSMenuItems", items);
        if no(element, "autoenablesItems") {
            self.graph.set(object, "NSNoAutoenable", true);
        }
        if let Some(name) = element.attr("systemMenu").and_then(|name| match name {
            "main" => Some("_NSMainMenu"),
            "apple" => Some("_NSAppleMenu"),
            "services" => Some("_NSServicesMenu"),
            "window" => Some("_NSWindowsMenu"),
            "help" => Some("_NSHelpMenu"),
            "font" => Some("_NSFontMenu"),
            "recentDocuments" => Some("_NSRecentDocumentsMenu"),
            _ => None,
        }) {
            self.string(object, "NSName", name);
        }
        Ok(object)
    }

    fn menu_indentation(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(indent) = element.attr("indentationLevel") {
            let indent = indent
                .parse::<i64>()
                .map_err(|_| "invalid menu indentation")?;
            if indent != 0 {
                self.graph.set(object, "NSIndent", indent);
            }
        }

        Ok(())
    }
    fn menu_attributed_title(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(attributed) = element.child("attributedString") {
            if !attributed.children.is_empty() || !attributed.text.is_empty() {
                return Err("nonempty attributed menu titles need text attributes".into());
            }
            let string = self.graph.string("");
            let title = self
                .graph
                .shared("NSAttributedString", vec![("NSString", string.into())]);
            self.graph.set(object, "NSAttributedTitle", title);
        }

        Ok(())
    }

    fn menu_item(
        &mut self,
        element: &Element,
        menu: Id,
        popup: Option<Id>,
        title_item: bool,
    ) -> Result<Id, String> {
        let object = self.object(element)?;
        self.owners.push((object, menu));
        self.oids.push(object);
        self.graph.set(object, "NSMenu", menu);
        let separator = yes(element, "isSeparatorItem");
        if no(element, "enabled") || separator {
            self.graph.set(object, "NSIsDisabled", true);
        }
        if separator {
            self.graph.set(object, "NSIsSeparator", true);
        }
        if yes(element, "hidden") || title_item {
            self.graph.set(object, "NSIsHidden", true);
        }
        for (attribute, key) in [
            ("alternate", "NSIsAlternate"),
            ("allowsKeyEquivalentWhenHidden", "NSKEAllowedWhenHidden"),
        ] {
            if yes(element, attribute) {
                self.graph.set(object, key, true);
            }
        }
        self.menu_indentation(element, object)?;
        if let Some(identifier) = element.attr("identifier") {
            self.string(object, "NSMenuItemIdentifier", identifier);
        }
        self.graph
            .set(object, "NSAllowsKeyEquivalentLocalization", true);
        self.graph
            .set(object, "NSAllowsKeyEquivalentMirroring", true);
        let title = super::string_value(element, "title")?.unwrap_or_default();
        let title = self.text(element, "title", &title);
        self.graph.set(object, "NSTitle", title);
        self.string(
            object,
            "NSKeyEquiv",
            element.attr("keyEquivalent").unwrap_or(""),
        );
        if !separator {
            let mask = element.child("modifierMask").map_or(1_i64 << 20, |mask| {
                [
                    ("capsLock", 16),
                    ("shift", 17),
                    ("control", 18),
                    ("option", 19),
                    ("command", 20),
                    ("numericPad", 21),
                    ("help", 22),
                    ("function", 23),
                ]
                .iter()
                .fold(0_i64, |bits, (key, bit)| {
                    bits | (i64::from(yes(mask, key)) << bit)
                })
            });
            if mask != 0 {
                self.graph.set(object, "NSKeyEquivModMask", mask);
            }
        }
        self.graph.set(object, "NSMnemonicLoc", 2_147_483_647_i64);
        if element.attr("state") == Some("on") {
            self.graph.set(object, "NSState", 1_i64);
        } else if element.attr("state") == Some("mixed") {
            self.graph.set(object, "NSState", -1_i64);
        }
        for (key, name) in [
            ("NSOnImage", "NSMenuCheckmark"),
            ("NSMixedImage", "NSMenuMixedState"),
        ] {
            let image = self.image(name);
            self.graph.set(object, key, image);
        }
        if let Some(image) = element.attr("image") {
            let image = self.image(image);
            self.graph.set(object, "NSImage", image);
        }
        if let Some(submenu) = element.child("menu") {
            let submenu = self.menu(submenu, object, None, false)?;
            self.string(object, "NSAction", "submenuAction:");
            self.graph.set(object, "NSTarget", submenu);
            self.graph.set(object, "NSSubmenu", submenu);
        } else if let Some(popup) = popup {
            self.string(object, "NSAction", "_popUpItemAction:");
            self.graph.set(object, "NSTarget", popup);
        }
        if let Some(tag) = element.attr("tag") {
            self.graph.set(
                object,
                "NSTag",
                tag.parse::<i64>().map_err(|_| "invalid menu item tag")?,
            );
        }
        self.menu_attributed_title(element, object)?;
        self.graph.set(object, "NSHiddenInRepresentation", false);
        Ok(object)
    }
}
