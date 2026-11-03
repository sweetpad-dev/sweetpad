use super::{Element, Encoder, Id, Value, custom_class, no, number, yes};

fn child_views(element: &Element) -> Vec<&Element> {
    if element.name == "scrollView" {
        ["contentView", "horizontalScroller", "verticalScroller"]
            .iter()
            .filter_map(|key| {
                element
                    .children
                    .iter()
                    .find(|child| child.attr("key") == Some(key))
            })
            .collect()
    } else if element.name == "gridView" {
        element.child("gridCells").map_or(Vec::new(), |cells| {
            cells
                .children
                .iter()
                .filter(|cell| super::grids::visible(cell, element))
                .filter_map(super::grids::content)
                .collect()
        })
    } else {
        element
            .child("subviews")
            .map_or(Vec::new(), |s| s.children.iter().collect::<Vec<_>>())
    }
}

impl Encoder<'_> {
    pub(super) fn view(&mut self, element: &Element, object: Id, parent: Id) -> Result<(), String> {
        self.custom_view(element, object)?;
        let top = self.oids.first() == Some(&parent) || self.root_views.contains(&object);
        self.graph.set(
            object,
            "NSNextResponder",
            if top { Value::Nil } else { parent.into() },
        );
        self.graph.set(object, "NSNibTouchBar", Value::Nil);
        let flags = element.child("autoresizingMask").map_or(12, |mask| {
            [
                "flexibleMinX",
                "widthSizable",
                "flexibleMaxX",
                "flexibleMinY",
                "heightSizable",
                "flexibleMaxY",
            ]
            .iter()
            .enumerate()
            .fold(0, |flags, (index, key)| {
                flags | (i64::from(yes(mask, key)) << index)
            })
        });
        self.graph.set(
            object,
            "NSvFlags",
            if yes(element, "hidden") {
                (256_i64 | flags) - 2_147_483_648
            } else {
                256_i64 | flags
            },
        );
        let children = child_views(element);
        let child_ids = children
            .iter()
            .map(|c| self.object(c))
            .collect::<Result<Vec<_>, _>>()?;
        if !(matches!(
            element.name.as_str(),
            "textField"
                | "textView"
                | "tableView"
                | "outlineView"
                | "collectionView"
                | "secureTextField"
                | "progressIndicator"
        ) || matches!(
            element.name.as_str(),
            "view" | "customView" | "containerView" | "stackView" | "splitView" | "tableCellView"
        ) && children.is_empty())
        {
            let subviews = self.graph.array("NSMutableArray", &child_ids);
            self.graph.set(object, "NSSubviews", subviews);
        }
        self.drag_types(element, object);
        self.frame(element, object)?;
        self.view_metadata(element, object)?;
        self.accessibility_properties(element, object);

        if !top {
            self.graph.set(object, "NSSuperview", parent);
        }
        if element.name == "progressIndicator" || yes(element, "wantsLayer") {
            self.graph.set(object, "NSViewIsLayerTreeHost", true);
        }
        self.graph
            .set(object, "NSViewWantsBestResolutionOpenGLSurface", true);
        if no(element, "translatesAutoresizingMaskIntoConstraints")
            && !yes(element, "fixedFrame")
            && element
                .attr("id")
                .is_some_and(|id| self.constrained.contains(id))
        {
            self.graph
                .set(object, "NSDoNotTranslateAutoresizingMask", true);
        }
        self.priorities(element, object)?;
        if yes(element, "allowsExpansionToolTips")
            && matches!(element.name.as_str(), "textField" | "secureTextField")
        {
            self.graph
                .set(object, "NSControlAllowsExpansionToolTips", true);
        }
        if element.name != "gridView" {
            for child in &children {
                self.encode(child, object)?;
            }
        }
        self.view_contents(element, object, parent)?;
        self.view_constraints(element, object)?;
        self.graph
            .set(object, "IBNSSafeAreaLayoutGuide", Value::Nil);
        self.graph.set(object, "IBNSLayoutMarginsGuide", Value::Nil);
        self.graph.set(object, "IBNSClipsToBounds", 0_i64);
        Ok(())
    }

    fn view_constraints(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(constraints) = element.child("constraints") {
            let constraints = constraints
                .children
                .iter()
                .filter(|c| !yes(c, "placeholder"))
                .rev()
                .map(|c| self.constraint(c, object))
                .collect::<Result<Vec<_>, _>>()?;
            if !constraints.is_empty() {
                let constraints = self.graph.array("NSArray", &constraints);
                self.graph.set(object, "NSViewConstraints", constraints);
            }
        }

        Ok(())
    }
    fn accessibility_properties(&mut self, element: &Element, object: Id) {
        if let Some(accessibility) = element.child("accessibility")
            && let Some(description) = accessibility.attr("description")
        {
            let connector = self.graph.object("NSNibAXAttributeConnector");
            let attribute = self.graph.object_with_values(
                "NSMutableString",
                vec![("NS.bytes", Value::Data(b"AXDescription".to_vec()))],
            );
            let value = self.text(element, "ibExternalAccessibilityDescription", description);
            self.graph
                .set(connector, "AXAttributeTypeArchiveKey", attribute);
            self.graph
                .set(connector, "AXAttributeValueArchiveKey", value);
            self.graph.set(connector, "AXDestinationArchiveKey", object);
            self.accessibility.push(connector);
        }
    }

    fn view_contents(&mut self, element: &Element, object: Id, parent: Id) -> Result<(), String> {
        match element.name.as_str() {
            "imageView" => self.image_view(element, object)?,
            "button" | "textField" | "secureTextField" | "popUpButton" => {
                self.control(element, object)?;
            }
            "progressIndicator" => self.progress(element, object)?,
            "stackView" => self.stack(element, object)?,
            "gridView" => self.grid(element, object)?,
            "wkWebView" => self.web_view(element, object)?,
            "textView" => self.text_view(element, object)?,
            "outlineView" => {
                self.outline_view(element, object, self.graph.class(parent) == "NSClipView")?;
            }
            "tableView" => {
                self.table_view(element, object, self.graph.class(parent) == "NSClipView")?;
            }
            "collectionView" => self.collection_view(element, object)?,
            "box" => self.box_view(element, object)?,
            "splitView" => self.split_view(element, object)?,
            "scrollView" => self.scroll_view(element, object)?,
            "clipView" => self.clip_view(element, object)?,
            "scroller" => {
                self.graph.remove(object, "NSSubviews");
                self.graph.replace(
                    object,
                    "NSvFlags",
                    if yes(element, "hidden") {
                        -2_147_483_392_i64
                    } else {
                        256
                    },
                );
            }
            _ => {}
        }
        Ok(())
    }

    #[allow(clippy::float_cmp)]
    fn view_metadata(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let alpha = number(element, "alphaValue", 1.0)?;
        if alpha != 1.0 {
            self.graph.set(object, "NSViewAlphaValue", alpha);
        }
        if let Some(identifier) = element.attr("identifier") {
            self.string(object, "NSReuseIdentifierKey", identifier);
        }
        Ok(())
    }

    fn custom_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if element.name == "customView" || element.attr("customClass").is_some() {
            let base = match element.name.as_str() {
                "customView" | "view" => "NSView",
                "imageView" => "NSImageView",
                "textField" => "NSTextField",
                "textView" => "NSTextView",
                "tableView" => "NSTableView",
                "outlineView" => "NSOutlineView",
                "tableCellView" => "NSTableCellView",
                "collectionView" => "NSCollectionView",
                "secureTextField" => "NSSecureTextField",
                "button" => "NSButton",
                "popUpButton" => "NSPopUpButton",
                "progressIndicator" => "NSProgressIndicator",
                other => return Err(format!("unsupported custom view <{other}>")),
            };
            self.string(
                object,
                "NSClassName",
                &custom_class(element, self.options).unwrap_or_else(|| base.into()),
            );
            if self.graph.class(object) != "NSCustomView" {
                self.string(object, "NSOriginalClassName", base);
            }
        }
        Ok(())
    }

    fn drag_types(&mut self, element: &Element, object: Id) {
        if element.name == "imageView" {
            let types = [
                "Apple PDF pasteboard type",
                "Apple PICT pasteboard type",
                "Apple PNG pasteboard type",
                "NSFilenamesPboardType",
                "NeXT TIFF v4.0 pasteboard type",
                "com.apple.NSFilePromiseItemMetaData",
                "com.apple.pasteboard.promised-file-content-type",
                "dyn.ah62d4rv4gu8yc6durvwwa3xmrvw1gkdusm1044pxqyuha2pxsvw0e55bsmwca7d3sbwu",
            ]
            .map(|s| self.graph.string(s));
            let types = self.graph.array("NSMutableSet", &types);
            self.graph.set(object, "NSDragTypes", types);
        }
    }

    #[allow(clippy::float_cmp)] // These exact defaults decide field omission.
    fn priorities(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if element.name == "scroller" {
            let default_h = 250.0;
            let default_v = 750.0;
            let h = number(element, "horizontalHuggingPriority", default_h)?;
            let v = number(element, "verticalHuggingPriority", default_v)?;
            if h != default_h || v != default_v {
                self.string(object, "NSHuggingPriority", &format!("{{{h}, {v}}}"));
            }
            return Ok(());
        }
        if element.attr("horizontalHuggingPriority").is_some()
            || element.attr("verticalHuggingPriority").is_some()
            || matches!(
                element.name.as_str(),
                "textField" | "tableView" | "outlineView" | "secureTextField" | "progressIndicator"
            )
            || (element.name == "button"
                && element.child("buttonCell").is_some_and(|c| {
                    c.attr("borderStyle") == Some("border")
                        && (!matches!(c.attr("bezelStyle"), Some("regularSquare" | "smallSquare"))
                            || matches!(c.attr("type"), Some("check" | "radio")))
                }))
        {
            let h = number(element, "horizontalHuggingPriority", 250.0)?;
            let v = number(
                element,
                "verticalHuggingPriority",
                if element.name == "popUpButton" {
                    750.0
                } else {
                    250.0
                },
            )?;
            let square_button = element.name == "button"
                && element.child("buttonCell").is_some_and(|cell| {
                    matches!(
                        cell.attr("bezelStyle"),
                        Some("regularSquare" | "smallSquare" | "roundedRect")
                    ) || cell.attr("borderStyle").is_none()
                });
            let default_v = if !square_button
                && matches!(
                    element.name.as_str(),
                    "textField"
                        | "secureTextField"
                        | "button"
                        | "popUpButton"
                        | "progressIndicator"
                        | "imageView"
                        | "tableView"
                        | "outlineView"
                ) {
                750.0
            } else {
                250.0
            };
            if h != 250.0 || v != default_v {
                self.string(object, "NSHuggingPriority", &format!("{{{h}, {v}}}"));
            }
        }
        let h = number(element, "horizontalCompressionResistancePriority", 750.0)?;
        let v = number(element, "verticalCompressionResistancePriority", 750.0)?;
        if h != 750.0 || v != 750.0 {
            self.string(
                object,
                "NSAntiCompressionPriority",
                &format!("{{{h}, {v}}}"),
            );
        }
        Ok(())
    }

    fn frame(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let frame = element.child("rect").ok_or("an AppKit view has no frame")?;
        let [x, y, width, height] = ["x", "y", "width", "height"]
            .map(|k| number(frame, k, 0.0))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .unwrap();
        if x == 0.0 && y == 0.0 {
            self.string(object, "NSFrameSize", &format!("{{{width}, {height}}}"));
        } else {
            self.string(
                object,
                "NSFrame",
                &format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}"),
            );
        }
        Ok(())
    }

    fn image_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let cell = element
            .child("imageCell")
            .ok_or("an image view has no cell")?;
        let cell_object = self.object(cell)?;
        if let Some(identifier) = element.attr("identifier") {
            self.string(cell_object, "NSCellIdentifier", identifier);
        }
        self.owners.push((cell_object, object));
        self.oids.push(cell_object);
        self.graph.set(cell_object, "NSCellFlags", 0_i64);
        self.graph.set(cell_object, "NSCellFlags2", 33_554_432_i64);
        if let Some(name) = cell.attr("image") {
            let image = self.image(name);
            self.graph.set(cell_object, "NSContents", image);
        }
        self.graph.set(cell_object, "NSControlView", object);
        self.graph.set(
            cell_object,
            "NSAlign",
            match cell.attr("alignment") {
                None | Some("center" | "left") => 0_i64,
                Some(other) => return Err(format!("unsupported image cell alignment {other}")),
            },
        );
        self.graph.set(
            cell_object,
            "NSScale",
            match cell.attr("imageScaling") {
                Some("proportionallyUpOrDown") => 3_i64,
                Some("axesIndependently") => 1,
                Some("none") => 2,
                None | Some("proportionallyDown") => 0,
                Some(other) => return Err(format!("unsupported image scaling {other}")),
            },
        );
        self.graph.set(cell_object, "NSStyle", 0_i64);
        self.graph.set(cell_object, "NSAnimates", self.storyboard);
        self.graph.set(cell_object, "NSImageAnimation", -1_i64);
        self.graph.set(object, "NSEnabled", !no(element, "enabled"));
        self.graph.set(object, "NSCell", cell_object);
        for (key, value) in [
            ("NSAllowsLogicalLayoutDirection", self.options.localized),
            ("NSControlContinuous", false),
            (
                "NSControlRefusesFirstResponder",
                !no(cell, "refusesFirstResponder"),
            ),
            ("NSControlUsesSingleLineMode", false),
            ("NSEditable", true),
        ] {
            self.graph.set(object, key, value);
        }
        for (key, value) in [
            ("NSControlSize", 0_i64),
            ("NSControlTextAlignment", 0),
            ("NSControlLineBreakMode", 0),
            ("NSControlWritingDirection", -1),
            ("NSControlSendActionMask", 4),
            ("NSImageViewPlaceholderPrecedence", 0),
        ] {
            self.graph.set(object, key, value);
        }
        self.graph
            .set(object, "IBNSShadowedSymbolConfiguration", Value::Nil);
        Ok(())
    }
}
