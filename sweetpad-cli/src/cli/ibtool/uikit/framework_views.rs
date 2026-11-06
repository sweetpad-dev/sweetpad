use super::controls::enumeration;
use super::{Element, Encoder, Id, Value, no, number};

impl Encoder<'_> {
    pub(super) fn framework_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        match element.name.as_str() {
            "wkWebView" => self.web_view(element, object),
            "mapView" => self.map_view(element, object),
            _ => unreachable!(),
        }
    }

    fn web_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let empty = Element::default();
        let source = element.child("wkWebViewConfiguration").unwrap_or(&empty);
        let configuration = self.graph.object("WKWebViewConfiguration");
        self.graph.set(object, "configuration", configuration);
        for (attribute, default) in [
            ("allowsBackForwardNavigationGestures", false),
            ("allowsLinkPreview", true),
            ("findInteractionEnabled", false),
        ] {
            self.graph.set(
                object,
                attribute,
                element
                    .attr(attribute)
                    .map_or(default, |value| value == "YES"),
            );
        }
        let agent = self
            .graph
            .string(element.attr("customUserAgent").unwrap_or(""));
        self.graph.set(object, "customUserAgent", agent);
        for class in ["WKProcessPool", "WKUserContentController"] {
            let value = self.graph.object(class);
            self.graph.set(
                configuration,
                if class == "WKProcessPool" {
                    "processPool"
                } else {
                    "userContentController"
                },
                value,
            );
        }
        let preferences = self.graph.object("WKPreferences");
        self.graph.set(configuration, "preferences", preferences);
        let prefs = source.child("wkPreferences").unwrap_or(&empty);
        self.graph.set(
            preferences,
            "minimumFontSize",
            number(prefs, "minimumFontSize")?.unwrap_or(0.0),
        );
        for (attribute, default) in [
            ("javaScriptCanOpenWindowsAutomatically", false),
            ("javaScriptEnabled", true),
            ("textInteractionEnabled", true),
            ("shouldPrintBackgrounds", false),
        ] {
            self.graph.set(
                preferences,
                attribute,
                prefs
                    .attr(attribute)
                    .map_or(default, |value| value == "YES"),
            );
        }
        let store = self.graph.object("WKWebsiteDataStore");
        self.graph.set(store, "isDefaultDataStore", true);
        self.graph.set(configuration, "websiteDataStore", store);
        for (attribute, default) in [
            ("suppressesIncrementalRendering", false),
            ("allowsAirPlayForMediaPlayback", true),
            ("drawsBackground", true),
            ("allowsInlineMediaPlayback", false),
            ("allowsInlineMediaPlaybackAfterFullscreen", false),
            ("mediaTypesRequiringUserActionForPlayback", true),
            ("allowsPictureInPictureMediaPlayback", true),
            ("ignoresViewportScaleLimits", false),
            ("textInteractionGesturesEnabled", true),
            ("longPressActionsEnabled", true),
            ("systemPreviewEnabled", false),
            ("shouldDecidePolicyBeforeLoadingQuickLookPreview", true),
            ("scrollToTextFragmentMarkingEnabled", true),
            ("multiRepresentationHEICInsertionEnabled", false),
            ("scrollToTextFragmentIndicatorEnabled", true),
        ] {
            self.graph.set(
                configuration,
                attribute,
                source
                    .attr(attribute)
                    .map_or(default, |value| value == "YES"),
            );
        }
        let agent = self
            .graph
            .string(source.attr("applicationNameForUserAgent").unwrap_or(""));
        self.graph
            .set(configuration, "applicationNameForUserAgent", agent);
        self.graph.set(configuration, "dataDetectorTypes", 127_i64);
        self.graph.set(configuration, "selectionGranularity", 0_i64);
        self.graph.set(configuration, "dragLiftDelay", 0_i64);
        Ok(())
    }

    fn map_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.graph.set(object, "MKDelegate", Value::Nil);
        let kind = enumeration(
            element,
            "mapType",
            &[
                ("standard", 0),
                ("satellite", 1),
                ("hybrid", 2),
                ("satelliteFlyover", 3),
                ("hybridFlyover", 4),
                ("mutedStandard", 5),
            ],
            0,
        )?;
        self.graph.set(object, "MKMapType", kind);
        for (attribute, key, default) in [
            ("zoomEnabled", "MKZoomEnabled", true),
            ("scrollEnabled", "MKScrollEnabled", true),
            ("showsUserLocation", "MKShowsUserLocation", false),
            ("pitchEnabled", "MKPitchEnabled", true),
            ("rotateEnabled", "MKRotateEnabled", true),
            ("showsBuildings", "MKShowsBuildings", true),
            ("showsTraffic", "MKShowsTraffic", false),
            ("showsCompass", "MKShowsCompass", true),
            ("showsScale", "MKShowsScaleKey", false),
            ("showsPointsOfInterest", "MKShowsPointsOfInterest", true),
            (
                "showsUserTrackingButton",
                "MKShowsUserTrackingButton",
                false,
            ),
        ] {
            self.graph.set(
                object,
                key,
                element
                    .attr(attribute)
                    .map_or(default, |value| value == "YES"),
            );
        }
        self.graph
            .set(object, "MKPointOfInterestFilter", Value::Nil);
        self.graph.set(object, "MKPitchButtonVisibility", 1_i64);
        let configuration = self.graph.object(match kind {
            1 | 3 => "MKImageryMapConfiguration",
            2 | 4 => "MKHybridMapConfiguration",
            _ => "MKStandardMapConfiguration",
        });
        self.graph
            .set(object, "MKPreferredConfiguration", configuration);
        self.graph
            .set(configuration, "MKMapConfigurationElevationStyle", 0_i64);
        for (key, value) in [
            (
                "MKMapConfigurationShowsTraffic",
                !no(element, "showsTraffic") && element.attr("showsTraffic") == Some("YES"),
            ),
            ("MKMapConfigurationShowsHiking", false),
            ("MKMapConfigurationShowsTopographicFeatures", false),
        ] {
            self.graph.set(configuration, key, value);
        }
        self.graph.set(
            configuration,
            "MKMapConfigurationPointOfInterestFilter",
            Value::Nil,
        );
        if matches!(kind, 0 | 5) {
            self.graph.set(
                configuration,
                "MKStandardMapConfigurationEmphasisStyle",
                i64::from(kind == 5),
            );
        }
        Ok(())
    }
}
