//! Separator boxes retain a title cell even when no title is displayed.
use super::{Element, Encoder, Id, Value};

impl Encoder<'_> {
    pub(super) fn box_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if element.attr("boxType") != Some("separator") {
            return Err("this AppKit box type isn't compiled yet".into());
        }
        self.string(object, "NSOffsets", "{5, 5}");
        let cell = self.graph.object("NSTextFieldCell");
        self.graph.set(cell, "NSCellFlags", 67_108_864_i64);
        self.graph.set(cell, "NSCellFlags2", 134_217_728_i64);
        let title = if let Some(title) = element.attr("title") {
            self.text(element, "title", title)
        } else {
            self.graph.string("Title")
        };
        self.graph.set(cell, "NSContents", title);
        let font = Element {
            name: "font".into(),
            attributes: vec![("metaFont".into(), "smallSystem".into())],
            children: vec![],
            text: String::new(),
        };
        let font = self.font(&font)?;
        self.graph.set(cell, "NSSupport", font);
        if !self.storyboard {
            self.graph.set(cell, "NSControlView", Value::Nil);
        }
        for (key, name) in [
            ("NSBackgroundColor", "textBackgroundColor"),
            ("NSTextColor", "labelColor"),
        ] {
            let color = self.named_color(name)?;
            self.graph.set(cell, key, color);
        }
        self.graph.set(object, "NSTitleCell", cell);
        self.graph.set(object, "NSBorderType", 3_i64);
        self.graph.set(object, "NSBoxType", 2_i64);
        self.graph.set(object, "NSTitlePosition", 2_i64);
        self.graph
            .set(object, "NSTransparent", super::yes(element, "transparent"));
        Ok(())
    }
}
