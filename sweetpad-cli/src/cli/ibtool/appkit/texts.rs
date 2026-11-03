//! NSTextView's text container, layout manager, storage, and shared editing state.
use super::{Element, Encoder, Id, Value, no, number, yes};

fn size(element: &Element, key: &str, default: [f64; 2]) -> Result<[f64; 2], String> {
    element
        .children_named("size")
        .find(|s| s.attr("key") == Some(key))
        .map_or(Ok(default), |s| {
            Ok([
                number(s, "width", default[0])?,
                number(s, "height", default[1])?,
            ])
        })
}

pub(super) fn prepare(element: &mut Element) -> Result<(), String> {
    let frame = element.child("rect").ok_or("a text view has no frame")?;
    let width = number(frame, "width", 0.0)?;
    let height = number(frame, "height", 0.0)?;
    let min = size(element, "minSize", [0.0; 2])?;
    let max = size(element, "maxSize", [width, 0.0])?;
    if max[1] < min[1] {
        return Err("a text view's maximum height is below its minimum".into());
    }
    let height = height.max(min[1]).min(max[1]);
    let frame = element
        .children
        .iter_mut()
        .find(|e| e.name == "rect")
        .unwrap();
    frame.attributes.retain(|(key, _)| key != "height");
    frame.attributes.push(("height".into(), height.to_string()));
    Ok(())
}

fn shared_flags(element: &Element) -> i64 {
    let selectable = !no(element, "selectable");
    let rich = !no(element, "richText");
    let mut flags = 2048_i64
        | i64::from(selectable)
        | (i64::from(selectable && !no(element, "editable")) << 1)
        | (i64::from(rich) << 2)
        | (i64::from(rich && !no(element, "importsGraphics")) << 3)
        | (i64::from(!no(element, "drawsBackground")) << 8);
    for (attribute, bit) in [
        ("fieldEditor", 4),
        ("usesRuler", 6),
        ("continuousSpellChecking", 7),
        ("smartInsertDelete", 9),
        ("allowsUndo", 10),
        ("allowsDocumentBackgroundColorChange", 14),
        ("grammarChecking", 15),
        ("quoteSubstitution", 17),
        ("linkDetection", 18),
        ("dashSubstitution", 24),
        ("textReplacement", 25),
        ("spellingCorrection", 26),
    ] {
        flags |= i64::from(yes(element, attribute)) << bit;
    }
    if yes(element, "dataDetection") {
        flags |= (1 << 22) | (1 << 23);
    }
    flags
}

impl Encoder<'_> {
    pub(super) fn text_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if element
            .children
            .iter()
            .any(|e| e.name == "attributedString")
        {
            return Err("AppKit attributed text storage isn't compiled yet".into());
        }
        let frame = element.child("rect").ok_or("a text view has no frame")?;
        let dimensions = [number(frame, "width", 0.0)?, number(frame, "height", 0.0)?];
        let container = self.text_container(object, dimensions[0]);
        self.graph.set(object, "NSTextContainer", container);
        let shared = self.text_shared_data(element)?;
        self.graph.set(object, "NSSharedData", shared);
        self.graph.set(
            object,
            "NSTVFlags",
            0x84_i64
                | i64::from(yes(element, "horizontallyResizable"))
                | (i64::from(yes(element, "verticallyResizable")) << 1),
        );
        for (attribute, key, default) in [
            ("minSize", "NSMinize", [0.0; 2]),
            ("maxSize", "NSMaxSize", [dimensions[0], 0.0]),
        ] {
            let value = size(element, attribute, default)?;
            if value
                .iter()
                .zip(dimensions)
                .any(|(a, b)| a.to_bits() != b.to_bits())
            {
                self.string(object, key, &format!("{{{}, {}}}", value[0], value[1]));
            }
        }
        self.graph.set(object, "NSDelegate", Value::Nil);
        let color = self.text_color(element, "textColor", Some("textColor"))?;
        self.graph.set(object, "NSTextViewTextColor", color);
        Ok(())
    }

    fn text_container(&mut self, view: Id, width: f64) -> Id {
        let container = self.graph.object("NSTextContainer");
        let manager = self.graph.object("NSLayoutManager");
        let storage = self.graph.object("NSTextStorage");
        let string = self.graph.object_with_values(
            "NSMutableString",
            vec![("NS.bytes", Value::Data(Vec::new()))],
        );
        self.graph.set(storage, "NSString", string);
        self.graph.set(storage, "NSDelegate", Value::Nil);
        self.graph.set(manager, "NSTextStorage", storage);
        let containers = self.graph.array("NSMutableArray", &[container]);
        self.graph.set(manager, "NSTextContainers", containers);
        self.graph.set(manager, "NSLMFlags", 102_i64);
        self.graph.set(manager, "NSDelegate", Value::Nil);
        self.graph.set(container, "NSLayoutManager", manager);
        self.graph.set(container, "NSTextLayoutManager", Value::Nil);
        self.graph.set(container, "NSTextView", view);
        self.graph.set(container, "NSWidth", width);
        self.graph.set(container, "NSMinWidth", 15.0);
        self.graph.set(container, "NSTCFlags", 1_i64);
        container
    }

    fn text_color(
        &mut self,
        element: &Element,
        key: &str,
        default: Option<&str>,
    ) -> Result<Option<Id>, String> {
        if let Some(color) = element
            .children_named("color")
            .find(|c| c.attr("key") == Some(key))
        {
            return self.color(color).map(Some);
        }
        default.map(|name| self.named_color(name)).transpose()
    }

    fn text_shared_data(&mut self, element: &Element) -> Result<Id, String> {
        let shared = self.graph.object("NSTextViewSharedData");
        self.graph.set(
            shared,
            "NSAutomaticTextCompletionDisabled",
            no(element, "textCompletion"),
        );
        self.graph.set(shared, "NSFlags", shared_flags(element));
        self.graph.set(
            shared,
            "NSMoreFlags",
            1_i64 | (i64::from(no(element, "allowsCharacterPickerTouchBarItem")) << 1),
        );
        self.graph.set(shared, "NSTextCheckingTypes", 0_i64);
        self.graph.set(shared, "NSMarkedAttributes", Value::Nil);
        let background = self.text_color(element, "backgroundColor", None)?;
        self.graph.set(shared, "NSBackgroundColor", background);
        let background = self.named_color("selectedTextBackgroundColor")?;
        let foreground = self.named_color("selectedTextColor")?;
        let selected = [
            (self.graph.string("NSBackgroundColor"), background),
            (self.graph.string("NSColor"), foreground),
        ];
        let selected = self.graph.fresh_dictionary("NSDictionary", &selected);
        self.graph.set(shared, "NSSelectedAttributes", selected);
        let insertion = self.text_color(
            element,
            "insertionPointColor",
            Some("textInsertionPointColor"),
        )?;
        self.graph.set(shared, "NSInsertionColor", insertion);
        let link = self.named_color("linkColor")?;
        let hotspot = self.graph.string("{8, -8}");
        let cursor = self.graph.shared(
            "NSCursor",
            vec![
                ("NSHotSpot", hotspot.into()),
                ("NSCursorType", Value::Int(13)),
            ],
        );
        let underline = self
            .graph
            .shared("NSNumber", vec![("NS.intval", Value::Int(1))]);
        let attributes = [
            (self.graph.string("NSColor"), link),
            (self.graph.string("NSCursor"), cursor),
            (self.graph.string("NSUnderline"), underline),
        ];
        let attributes = self.graph.fresh_dictionary("NSDictionary", &attributes);
        self.graph.set(shared, "NSLinkAttributes", attributes);
        for key in [
            "NSDefaultParagraphStyle",
            "NSTextFinder",
            "NSTextHighlightAttributes",
        ] {
            self.graph.set(shared, key, Value::Nil);
        }
        let finder = match element.attr("findStyle") {
            None | Some("none") => 0_i64,
            Some("panel") => 1,
            Some("bar") => 2,
            Some(other) => return Err(format!("unsupported text finder style {other}")),
        };
        self.graph.set(shared, "NSPreferredTextFinderStyle", finder);
        self.graph.set(shared, "NSWritingToolsFlags", 256_i64);
        Ok(shared)
    }
}
