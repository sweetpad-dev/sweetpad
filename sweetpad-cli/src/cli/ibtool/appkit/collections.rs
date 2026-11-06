//! Collection views and flow-layout state shared with their UIKit core.
use super::{Element, Encoder, Id, Value, no, number, yes};

impl Encoder<'_> {
    pub(super) fn collection_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(layout) = element.child("collectionViewFlowLayout") {
            let layout_object = self.object(layout)?;
            self.owners.push((layout_object, object));
            self.oids.push(layout_object);
            self.collection_flow(layout, layout_object)?;
            let core = self.graph.object("_NSCollectionViewCore");
            self.graph.set(core, "UICollectionLayout", layout_object);
            if !yes(element, "selectable") {
                self.graph.set(core, "UIAllowsSelection", false);
            }
            if yes(element, "allowsMultipleSelection") {
                self.graph.set(core, "UIAllowsMultipleSelection", true);
            }
            self.graph.set(object, "NSCollectionViewCore", core);
            if layout.attr("scrollDirection") == Some("horizontal")
                && let Some(Value::Int(flags)) = self.graph.value(object, "NSvFlags")
            {
                self.graph.replace(
                    object,
                    "NSvFlags",
                    (flags & !0x12) | ((flags & 0x02) << 3) | ((flags & 0x10) >> 3),
                );
            }
        }
        for key in ["NSMinGridSize", "NSMaxGridSize"] {
            self.string(object, key, "{0, 0}");
        }
        for key in ["NSMaxNumberOfGridRows", "NSMaxNumberOfGridColumns"] {
            self.graph.set(object, key, 0_i64);
        }
        let color = element
            .children_named("color")
            .find(|c| c.attr("key") == Some("primaryBackgroundColor"))
            .ok_or("a collection view has no primary background color")?;
        let color = self.color(color)?;
        let colors = self.graph.array("NSArray", &[color]);
        self.graph.set(object, "NSBackgroundColors", colors);
        for (key, value) in [
            ("NSSelectable", yes(element, "selectable")),
            (
                "NSAllowsMultipleSelection",
                yes(element, "allowsMultipleSelection"),
            ),
            (
                "NSAvoidsEmptySelection",
                no(element, "allowsEmptySelection"),
            ),
        ] {
            if value {
                self.graph.set(object, key, true);
            }
        }
        self.graph
            .set(object, "NSDraggingSourceMaskForLocal", -1_i64);
        self.graph
            .set(object, "NSDraggingSourceMaskForNonLocal", 0_i64);
        Ok(())
    }

    #[allow(clippy::cast_possible_truncation)]
    fn collection_flow(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for (attribute, key) in [
            ("minimumInteritemSpacing", "NSInteritemSpacing"),
            ("minimumLineSpacing", "NSLineSpacing"),
        ] {
            let value = number(element, attribute, 10.0)? as f32;
            if value.to_bits() != 10.0_f32.to_bits() {
                self.graph.set(object, key, value);
            }
        }
        for (attribute, key, default) in [
            ("itemSize", "NSItemSize", 50.0),
            ("headerReferenceSize", "NSHeaderReferenceSize", 0.0),
            ("footerReferenceSize", "NSFooterReferenceSize", 0.0),
        ] {
            if let Some(size) = element
                .children_named("size")
                .find(|s| s.attr("key") == Some(attribute))
            {
                let w = number(size, "width", default)?;
                let h = number(size, "height", default)?;
                if w.to_bits() != default.to_bits() || h.to_bits() != default.to_bits() {
                    let size = self.graph.string(&format!("{{{w}, {h}}}"));
                    let value = self.graph.object_with_values(
                        "NSValue",
                        vec![("NS.special", Value::Int(2)), ("NS.sizeval", size.into())],
                    );
                    self.graph.set(object, key, value);
                }
            }
        }
        if let Some(insets) = element.child("edgeInsets") {
            let mut fields = vec![("NS.special", Value::Int(12))];
            for (attribute, key) in [
                ("top", "NS.edgeval.top"),
                ("left", "NS.edgeval.left"),
                ("bottom", "NS.edgeval.bottom"),
                ("right", "NS.edgeval.right"),
            ] {
                fields.push((key, Value::Double(number(insets, attribute, 0.0)?)));
            }
            if fields
                .iter()
                .skip(1)
                .any(|(_, value)| matches!(value, Value::Double(n) if *n != 0.0))
            {
                let value = self.graph.object_with_values("NSValue", fields);
                self.graph.set(object, "NSSectionInset", value);
            }
        }
        match element.attr("scrollDirection") {
            None | Some("vertical") => {}
            Some("horizontal") => self.graph.set(object, "NSScrollDirection", 1_i64),
            Some(other) => return Err(format!("unsupported collection scroll direction {other}")),
        }
        Ok(())
    }
}
