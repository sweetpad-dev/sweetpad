use super::{Element, Encoder, Id, Value, custom_class, no, number, yes};

fn rect(element: &Element) -> Result<String, String> {
    let [x, y, width, height] = ["x", "y", "width", "height"]
        .map(|key| number(element, key, 0.0))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .unwrap();
    Ok(format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}"))
}

impl Encoder<'_> {
    pub(super) fn window(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let mut style = 0_i64;
        if let Some(mask) = element.child("windowStyleMask") {
            for (attribute, bit) in [
                ("titled", 1),
                ("closable", 2),
                ("miniaturizable", 4),
                ("resizable", 8),
                ("utility", 16),
                ("docModal", 64),
                ("nonactivatingPanel", 128),
                ("HUD", 8192),
                ("fullSizeContentView", 32768),
            ] {
                if yes(mask, attribute) {
                    style |= bit;
                }
            }
        }
        self.graph.set(object, "NSWindowStyleMask", style);
        self.graph.set(object, "NSWindowBacking", 2_i64);
        let content = element
            .children_named("rect")
            .find(|e| e.attr("key") == Some("contentRect"))
            .ok_or("a window has no content rect")?;
        if element.child("windowPositionMask").is_none()
            && let Some(screen) = element
                .children_named("rect")
                .find(|e| e.attr("key") == Some("screenRect"))
        {
            let width = number(content, "width", 0.0)?;
            let height = number(content, "height", 0.0)?;
            let x = number(screen, "x", 0.0)? + (number(screen, "width", 0.0)? - width) / 2.0;
            let y = number(screen, "y", 0.0)? + (number(screen, "height", 0.0)? - height) / 2.0;
            self.string(
                object,
                "NSWindowRect",
                &format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}"),
            );
        } else {
            self.string(object, "NSWindowRect", &rect(content)?);
        }
        self.window_flags(element, object);
        let title = self.text(element, "title", element.attr("title").unwrap_or(""));
        self.graph.set(object, "NSWindowTitle", title);
        self.string(
            object,
            "NSWindowSubtitle",
            element.attr("subtitle").unwrap_or(""),
        );
        self.string(
            object,
            "NSWindowClass",
            &custom_class(element, self.options).unwrap_or_else(|| "NSWindow".into()),
        );
        if element.attr("customClass").is_some() {
            self.class_reference(element, object);
        }
        self.graph.set(object, "NSViewClass", Value::Nil);
        self.graph
            .set(object, "NSUserInterfaceItemIdentifier", Value::Nil);
        if let Some(identifier) = element.attr("identifier") {
            self.graph.remove(object, "NSUserInterfaceItemIdentifier");
            self.string(object, "NSUserInterfaceItemIdentifier", identifier);
        }
        if let Some(view) = element
            .children
            .iter()
            .find(|e| e.attr("key") == Some("contentView"))
        {
            self.root_views.insert(self.object(view)?);
            let view = self.encode(view, object)?;
            self.graph.set(object, "NSWindowView", view);
        }
        if let Some(screen) = element
            .children_named("rect")
            .find(|e| e.attr("key") == Some("screenRect"))
        {
            self.string(object, "NSScreenRect", &rect(screen)?);
        }
        self.window_sizes(element, object, style)?;
        self.window_behavior(element, object)?;
        if !no(element, "visibleAtLaunch") {
            self.visible_windows.push(object);
        }
        Ok(())
    }
    fn window_behavior(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.graph
            .set(object, "NSWindowIsRestorable", !no(element, "restorable"));
        if !yes(element, "ibtoolStoryboardWindow") {
            for key in ["NSMinFullScreenContentSize", "NSMaxFullScreenContentSize"] {
                self.string(object, key, "{0, 0}");
            }
        }
        if let Some(mode) = element.attr("tabbingMode") {
            let mode = match mode {
                "automatic" => 0_i64,
                "preferred" => 1,
                "disallowed" => 2,
                other => return Err(format!("unsupported window tabbing mode {other}")),
            };
            if mode != 0 {
                self.graph.set(object, "NSWindowTabbingMode", mode);
            }
        }
        if let Some(name) = element.attr("frameAutosaveName") {
            self.string(object, "NSFrameAutosaveName", name);
        }
        if yes(element, "titlebarAppearsTransparent") {
            self.graph.set(object, "NSTitlebarAppearsTransparent", true);
        }
        for (attribute, key, values) in [
            (
                "titleVisibility",
                "NSWindowTitleVisibility",
                &["visible", "hidden"][..],
            ),
            (
                "separatorStyle",
                "NSTitlebarSeparatorStyle",
                &["automatic", "none", "line", "shadow"][..],
            ),
            (
                "toolbarStyle",
                "NSWindowToolbarStyle",
                &[
                    "automatic",
                    "expanded",
                    "preference",
                    "unified",
                    "unifiedCompact",
                ][..],
            ),
        ] {
            if let Some(value) = element.attr(attribute) {
                let value = values
                    .iter()
                    .position(|v| *v == value)
                    .ok_or_else(|| format!("unsupported window {attribute}: {value}"))?;
                if value != 0 {
                    self.graph.set(object, key, i64::try_from(value).unwrap());
                }
            }
        }
        if let Some(behavior) = element.child("windowCollectionBehavior") {
            let flags = [
                "canJoinAllSpaces",
                "moveToActiveSpace",
                "managed",
                "transient",
                "stationary",
                "participatesInCycle",
                "ignoresCycle",
                "fullScreenPrimary",
                "fullScreenAuxiliary",
                "fullScreenNone",
                "fullScreenAllowsTiling",
                "fullScreenDisallowsTiling",
            ]
            .iter()
            .enumerate()
            .fold(0_i64, |mask, (i, key)| {
                mask | (i64::from(yes(behavior, key)) << i)
            });
            if flags != 0 {
                self.graph.set(object, "NSWindowCollectionBehavior", flags);
            }
        }
        self.window_animation(element, object)?;
        Ok(())
    }

    fn window_sizes(&mut self, element: &Element, object: Id, style: i64) -> Result<(), String> {
        for (key, archive) in [("minSize", "NSMinSize"), ("maxSize", "NSMaxSize")] {
            if let Some(size) = element.children.iter().find(|e| e.attr("key") == Some(key)) {
                let width = number(size, "width", 0.0)?;
                let height = number(size, "height", 0.0)?;
                self.string(
                    object,
                    if key == "minSize" {
                        "NSWindowContentMinSize"
                    } else {
                        "NSWindowContentMaxSize"
                    },
                    &format!("{{{width}, {height}}}"),
                );
                // Xcode 27's AppKit frame conversion adds its 32-point title bar.
                let height = height
                    + if style & 1 != 0 {
                        if style & 16 != 0 && element.attr("customClass") == Some("NSPanel") {
                            24.0
                        } else {
                            32.0
                        }
                    } else {
                        0.0
                    };
                self.string(object, archive, &format!("{{{width}, {height}}}"));
            } else if key == "maxSize" {
                self.string(object, archive, "{10000000000000, 10000000000000}");
            }
        }
        Ok(())
    }

    fn window_flags(&mut self, element: &Element, object: Id) {
        let mut flags = 0x2000_0000_i64;
        if no(element, "releasedWhenClosed") {
            flags |= 1 << 30;
        }
        if yes(element, "hidesOnDeactivate") {
            flags |= 1 << 31;
        }
        if !no(element, "autorecalculatesKeyViewLoop") {
            flags |= 1 << 11;
        }
        if !no(element, "allowsToolTipsWhenApplicationIsInactive") {
            flags |= 1 << 13;
        }
        for (index, key) in ["leftStrut", "rightStrut", "topStrut", "bottomStrut"]
            .iter()
            .enumerate()
        {
            if element
                .child("windowPositionMask")
                .is_none_or(|mask| yes(mask, key))
            {
                flags |= 1 << (index + 19);
            }
        }
        if flags >= 2_147_483_648 {
            flags -= 4_294_967_296;
        }
        self.graph.set(object, "NSWTFlags", flags);
    }
    fn window_animation(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(behavior) = element.attr("animationBehavior") {
            let value = match behavior {
                "default" => 0_i64,
                "none" => 2,
                "documentWindow" => 3,
                "utilityWindow" => 4,
                other => return Err(format!("unsupported window animation {other}")),
            };
            if value != 0 {
                self.graph.set(object, "NSWindowAnimationBehavior", value);
            }
        }
        Ok(())
    }
}
