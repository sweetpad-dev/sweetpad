use super::{Element, Encoder, Id, Value, no, number, yes};

impl Encoder<'_> {
    pub(super) fn web_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let frame = element.child("rect").ok_or("a web view has no frame")?;
        let width = number(frame, "width", 0.0)?;
        let height = number(frame, "height", 0.0)?;
        let size = self.graph.string(&format!("{{{width}, {height}}}"));
        let flipped = self.web_subview("WKFlippedView", object, size, 274);
        self.graph.set(flipped, "NSViewClipsToBoundsKeyName", true);
        let zero = self.graph.string("{0, 0}");
        let background = self.web_subview("WKColorExtensionView", object, zero, -2_147_483_392);
        let foreground = self.web_subview("WKColorExtensionView", object, zero, -2_147_483_392);
        let views = self
            .graph
            .array("NSMutableArray", &[flipped, background, foreground]);
        self.graph.replace(object, "NSSubviews", views);
        let types = [
            "Apple HTML pasteboard type",
            "Apple PDF pasteboard type",
            "Apple URL pasteboard type",
            "Apple Web Archive pasteboard type",
            "Apple WebKit dummy pasteboard type",
            "Apple files promise pasteboard type",
            "NSColor pasteboard type",
            "NSFilenamesPboardType",
            "NSStringPboardType",
            "NeXT RTFD pasteboard type",
            "NeXT Rich Text Format v1.0 pasteboard type",
            "NeXT TIFF v4.0 pasteboard type",
            "WebURLsWithTitlesPboardType",
            "com.apple.webarchive",
            "public.png",
            "public.url",
            "public.url-name",
        ]
        .map(|s| self.graph.string(s));
        let types = self.graph.array("NSMutableSet", &types);
        self.graph.set(object, "NSDragTypes", types);
        self.graph.replace(object, "NSViewIsLayerTreeHost", true);
        let configuration = self.web_configuration(element)?;
        self.graph.set(object, "configuration", configuration);
        for (key, default) in [
            ("allowsBackForwardNavigationGestures", false),
            ("allowsLinkPreview", true),
            ("allowsMagnification", false),
        ] {
            self.graph.set(
                object,
                key,
                element.attr(key).map_or(default, |s| s == "YES"),
            );
        }
        self.string(
            object,
            "customUserAgent",
            element.attr("customUserAgent").unwrap_or(""),
        );
        self.graph.set(
            object,
            "magnification",
            number(element, "magnification", 1.0)?,
        );
        Ok(())
    }

    fn web_subview(&mut self, class: &'static str, parent: Id, size: Id, flags: i64) -> Id {
        let view = self.graph.object(class);
        self.graph.set(view, "NSNextResponder", parent);
        self.graph.set(view, "NSSuperview", parent);
        self.graph.set(view, "NSNibTouchBar", Value::Nil);
        self.graph.set(view, "NSvFlags", flags);
        self.graph.set(view, "NSFrameSize", size);
        self.graph.set(view, "NSViewIsLayerTreeHost", true);
        self.graph
            .set(view, "NSViewWantsBestResolutionOpenGLSurface", true);
        self.graph.set(view, "IBNSSafeAreaLayoutGuide", Value::Nil);
        self.graph.set(view, "IBNSLayoutMarginsGuide", Value::Nil);
        self.graph.set(view, "IBNSClipsToBounds", 0_i64);
        view
    }

    fn web_configuration(&mut self, element: &Element) -> Result<Id, String> {
        let empty = Element::default();
        let source = element.child("wkWebViewConfiguration").unwrap_or(&empty);
        let configuration = self.graph.object("WKWebViewConfiguration");
        for (key, class) in [
            ("processPool", "WKProcessPool"),
            ("userContentController", "WKUserContentController"),
        ] {
            let value = self.graph.object(class);
            self.graph.set(configuration, key, value);
        }
        let prefs = source.child("wkPreferences").unwrap_or(&empty);
        let preferences = self.graph.object("WKPreferences");
        self.graph.set(
            preferences,
            "minimumFontSize",
            number(prefs, "minimumFontSize", 0.0)?,
        );
        for (key, default) in [
            ("javaScriptCanOpenWindowsAutomatically", true),
            ("javaScriptEnabled", true),
            ("shouldPrintBackgrounds", false),
            ("textInteractionEnabled", true),
        ] {
            self.graph.set(
                preferences,
                key,
                prefs.attr(key).map_or(default, |s| s == "YES"),
            );
        }
        // Xcode's XML importer ignores tabFocusesLinks and drawsBackground.
        self.graph.set(preferences, "tabFocusesLinks", false);
        self.graph.set(configuration, "preferences", preferences);
        let store = self.graph.object("WKWebsiteDataStore");
        self.graph.set(store, "isDefaultDataStore", true);
        self.graph.set(configuration, "websiteDataStore", store);
        self.graph.set(
            configuration,
            "suppressesIncrementalRendering",
            yes(source, "suppressesIncrementalRendering"),
        );
        self.graph.set(
            configuration,
            "allowsAirPlayForMediaPlayback",
            !no(source, "allowsAirPlayForMediaPlayback"),
        );
        self.graph.set(configuration, "drawsBackground", true);
        self.string(
            configuration,
            "applicationNameForUserAgent",
            source.attr("applicationNameForUserAgent").unwrap_or(""),
        );
        self.graph
            .set(configuration, "userInterfaceDirectionPolicy", 0_i64);
        for (key, default) in [
            ("scrollToTextFragmentIndicatorEnabled", true),
            ("scrollToTextFragmentMarkingEnabled", true),
            ("multiRepresentationHEICInsertionEnabled", false),
        ] {
            self.graph.set(
                configuration,
                key,
                source.attr(key).map_or(default, |s| s == "YES"),
            );
        }
        Ok(configuration)
    }
}
