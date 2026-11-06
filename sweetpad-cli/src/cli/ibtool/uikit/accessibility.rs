use super::{Element, Encoder, Id, Value, yes};

impl Encoder<'_> {
    pub(super) fn accessibility(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let Some(configuration) = element.child("accessibility") else {
            return Ok(());
        };
        let entry = self.graph.object("UIRuntimeAccessibilityConfiguration");
        self.graph.set(entry, "UIAccessibileObject", object);
        for (attribute, key) in [
            ("hint", "UIAccessibilityHint"),
            ("identifier", "UIAccessibilityIdentifier"),
            ("label", "UIAccessibilityLabel"),
            ("value", "UIAccessibilityValue"),
            ("language", "UIAccessibilityLanguage"),
        ] {
            if let Some(text) = configuration.attr(attribute) {
                let text = match attribute {
                    "hint" => self.text(element, "accessibilityHint", text),
                    "label" => self.text(element, "accessibilityLabel", text),
                    "value" => self.text(element, "accessibilityValue", text),
                    _ => self.graph.string(text),
                };
                self.graph.set(entry, key, text);
            }
        }
        if let Some(traits) = configuration.child("accessibilityTraits") {
            let mut mask = 0;
            for (name, value) in &traits.attributes {
                if name == "key" || value == "NO" {
                    continue;
                }
                let bit = match name.as_str() {
                    "button" => 1,
                    "link" => 2,
                    "image" => 4,
                    "selected" => 8,
                    "playsSound" => 16,
                    "keyboardKey" => 32,
                    "staticText" => 64,
                    "summaryElement" => 128,
                    "notEnabled" => 256,
                    "updatesFrequently" => 512,
                    "searchField" => 1024,
                    "startsMediaSession" => 2048,
                    "adjustable" => 4096,
                    "allowsDirectInteraction" => 8192,
                    "causesPageTurn" => 16384,
                    "header" => 65536,
                    other => return Err(format!("unsupported accessibility trait {other}")),
                };
                mask |= bit;
            }
            let mask = self
                .graph
                .shared("NSNumber", vec![("NS.intval", Value::Int(mask))]);
            self.graph.set(entry, "UIAccessibilityTraits", mask);
        }
        for boolean in configuration.children_named("bool") {
            if boolean.attr("key") != Some("isElement") {
                return Err(format!(
                    "unsupported accessibility boolean {:?}",
                    boolean.attr("key")
                ));
            }
            let value = self.graph.shared(
                "NSNumber",
                vec![("NS.intval", Value::Int(i64::from(yes(boolean, "value"))))],
            );
            self.graph.set(entry, "UIIsAccessibilityElement", value);
        }
        self.accessibility_configurations.push(entry);
        Ok(())
    }

    pub fn install_accessibility(&mut self, root: Id) {
        self.install_traits(root);
        if !self.accessibility_configurations.is_empty() {
            let entries = self
                .graph
                .array("NSArray", &self.accessibility_configurations);
            self.graph
                .replace(root, "UINibAccessibilityConfigurationsKey", entries);
        }
    }

    pub(super) fn outlet_collection(
        &mut self,
        connections: &mut Vec<Id>,
        source: Id,
        destination: Id,
        property: &str,
        class: &str,
    ) {
        let key = (source, property.to_string());
        if let Some(&array) = self.outlet_collections.get(&key) {
            self.graph
                .set(array, super::super::nib::ELEMENT, destination);
            return;
        }
        let connection = self.graph.object("UIRuntimeOutletCollectionConnection");
        let label = self.graph.string(property);
        let array = self.graph.array("NSMutableArray", &[destination]);
        let class = self.graph.string(class);
        self.graph.set(connection, "UILabel", label);
        self.graph.set(connection, "UISource", source);
        self.graph.set(connection, "UIDestination", array);
        self.graph
            .set(connection, "runtimeCollectionClassName", class);
        self.graph
            .set(connection, "addsContentToExistingCollection", false);
        self.outlet_collections.insert(key, array);
        connections.push(connection);
    }
}
