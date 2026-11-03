use std::collections::BTreeMap;

use super::{Element, Encoder, Id, Value};

type AttributeVariations<'a> = BTreeMap<&'a str, (Value, Vec<(&'a str, Value)>)>;

impl Encoder<'_> {
    fn trait_collection(&mut self, key: &str) -> Result<Id, String> {
        let mut values = Vec::new();
        if key != "default" {
            for part in key.split('-') {
                let (name, value) = part
                    .split_once('=')
                    .ok_or_else(|| format!("invalid variation key {key}"))?;
                let (attribute, value) = match (name, value) {
                    ("widthClass", "compact") => (
                        "UITraitCollectionBuiltinTrait-_UITraitNameHorizontalSizeClass",
                        1,
                    ),
                    ("widthClass", "regular") => (
                        "UITraitCollectionBuiltinTrait-_UITraitNameHorizontalSizeClass",
                        2,
                    ),
                    ("heightClass", "compact") => (
                        "UITraitCollectionBuiltinTrait-_UITraitNameVerticalSizeClass",
                        1,
                    ),
                    ("heightClass", "regular") => (
                        "UITraitCollectionBuiltinTrait-_UITraitNameVerticalSizeClass",
                        2,
                    ),
                    // Xcode ignores this legacy condition key; it does not encode an idiom.
                    ("device", _) => continue,
                    other => return Err(format!("unsupported variation condition {other:?}")),
                };
                values.push((attribute, Value::Int(value)));
            }
        }
        values.sort_by_key(|(key, _)| *key);
        Ok(if values.is_empty() {
            self.graph.shared("UITraitCollection", values)
        } else {
            self.graph.object_with_values("UITraitCollection", values)
        })
    }

    fn boxed_boolean(&mut self, value: bool) -> Value {
        Value::Object(self.graph.shared(
            "NSNumber",
            vec![("NS.intval", Value::Int(i64::from(value)))],
        ))
    }

    pub(super) fn constraint_variations(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        let mut variations: Vec<_> = element
            .children_named("variation")
            .filter(|v| v.attr("constant").is_some())
            .collect();
        if variations.is_empty() {
            return Ok(());
        }
        variations.sort_by(|a, b| {
            let a = a.attr("key").unwrap_or("");
            let b = b.attr("key").unwrap_or("");
            (b.split('-').count(), b).cmp(&(a.split('-').count(), a))
        });
        // Xcode records both the legacy and V2 constant setters. Each has its
        // own storage and constant wrappers; priority-only variations add none.
        for _ in 0..2 {
            let storage = self.graph.object("_UIAttributeTraitStorage");
            self.graph.set(storage, "UIObject", object);
            let path = self.graph.string("encodedConstant");
            self.graph.set(storage, "UIKeyPath", path);
            let mut records = Vec::new();
            for variation in variations.iter().copied().chain(std::iter::once(element)) {
                let condition = if std::ptr::eq(variation, element) {
                    "default"
                } else {
                    variation
                        .attr("key")
                        .ok_or("a variation has no condition")?
                };
                if variation.attr("symbolic") == Some("YES") {
                    return Err("symbolic constraint variations are not supported".into());
                }
                let value = self.graph.object("_NSLayoutConstraintConstant");
                self.graph.set(value, "UISymbolicConstant", Value::Nil);
                self.graph.set(
                    value,
                    "UINumericConstant",
                    super::number(variation, "constant")?.unwrap_or(0.0),
                );
                let traits = self.trait_collection(condition)?;
                let record = self.graph.object("_UIAttributeTraitStorageRecord");
                self.graph.set(record, "UITraitCollection", traits);
                self.graph.set(record, "UIValue", value);
                records.push(record);
            }
            let records = self.graph.array("NSMutableArray", &records);
            self.graph.set(storage, "UIRecords", records);
            self.trait_storages
                .push((self.current_trait_root.unwrap_or(object), object, storage));
        }
        Ok(())
    }

    fn set_objects(&mut self, class: &'static str, objects: &[Id]) -> Id {
        let set = self.graph.object(class);
        for (index, object) in objects.iter().enumerate() {
            self.graph.set(set, &format!("NS.object.{index}"), *object);
        }
        set
    }

    pub(super) fn trait_variations(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let mut attributes: AttributeVariations<'_> = BTreeMap::new();
        let variations: Vec<_> = element.children_named("variation").collect();
        for variation in &variations {
            let condition = variation
                .attr("key")
                .ok_or("a variation has no condition")?;
            for (attribute, value) in &variation.attributes {
                match attribute.as_str() {
                    "key" | "text" => {}
                    "hidden" => {
                        let base = self.boxed_boolean(element.attr("hidden") == Some("YES"));
                        let value = self.boxed_boolean(value == "YES");
                        attributes
                            .entry("hidden")
                            .or_insert_with(|| (base, Vec::new()))
                            .1
                            .push((condition, value));
                    }
                    other => return Err(format!("unsupported variation attribute {other}")),
                }
            }
            for child in &variation.children {
                let (path, base_key, value) = match child.name.as_str() {
                    "fontDescription" => ("font", "UIFont", Value::Object(self.font(Some(child))?)),
                    "color" => {
                        let (path, base) = match child.attr("key") {
                            Some("textColor") => ("textColor", "UITextColor"),
                            Some("backgroundColor") => ("backgroundColor", "UIBackgroundColor"),
                            Some("tintColor") => ("tintColor", "UITintColor"),
                            other => return Err(format!("unsupported variation color {other:?}")),
                        };
                        (path, base, Value::Object(self.color(child)?))
                    }
                    "mask" => continue,
                    other => return Err(format!("unsupported variation element <{other}>")),
                };
                let base = self.graph.value(object, base_key).unwrap_or(Value::Nil);
                attributes
                    .entry(path)
                    .or_insert_with(|| (base, Vec::new()))
                    .1
                    .push((condition, value));
            }
        }
        for (path, (base, mut entries)) in attributes {
            let storage = self.graph.object("_UIAttributeTraitStorage");
            self.graph.set(storage, "UIObject", object);
            let path = self.graph.string(path);
            self.graph.set(storage, "UIKeyPath", path);
            entries.sort_by(|(a, _), (b, _)| {
                (b.split('-').count(), b).cmp(&(a.split('-').count(), a))
            });
            entries.push(("default", base));
            let mut records = Vec::new();
            for (condition, value) in entries {
                let traits = self.trait_collection(condition)?;
                let record = self.graph.object("_UIAttributeTraitStorageRecord");
                self.graph.set(record, "UITraitCollection", traits);
                self.graph.set(record, "UIValue", value);
                records.push(record);
            }
            let records = self.graph.array("NSMutableArray", &records);
            self.graph.set(storage, "UIRecords", records);
            self.trait_storages
                .push((self.current_trait_root.unwrap_or(object), object, storage));
        }
        self.relationship_variations(object, &variations)
    }

    fn relationship_variations(
        &mut self,
        object: Id,
        variations: &[&Element],
    ) -> Result<(), String> {
        let mut masks: BTreeMap<&str, Vec<(&str, &Element)>> = BTreeMap::new();
        for variation in variations {
            for mask in variation.children_named("mask") {
                let key = mask.attr("key").ok_or("a variation mask has no key")?;
                masks
                    .entry(key)
                    .or_default()
                    .push((variation.attr("key").unwrap(), mask));
            }
        }
        for (key, mut entries) in masks {
            let (path, archive_key) = match key {
                "subviews" => ("traitStorageSubviews", "UISubviews"),
                "constraints" => ("traitStorageConstraints", "UIViewAutolayoutConstraints"),
                other => return Err(format!("unsupported variation relationship {other}")),
            };
            let original = self
                .graph
                .value(object, archive_key)
                .and_then(|value| match value {
                    Value::Object(id) => Some(self.graph.references(id)),
                    _ => None,
                })
                .unwrap_or_default();
            if key == "constraints" {
                for constraint in &original {
                    for key in ["NSFirstItem", "NSSecondItem"] {
                        if let Some(Value::Object(view)) = self.graph.value(*constraint, key) {
                            self.trait_descendants
                                .push((self.current_trait_root.unwrap_or(object), view));
                        }
                    }
                }
            }
            let storage = self.graph.object("_UIRelationshipTraitStorage");
            self.graph.set(storage, "UIObject", object);
            let path = self.graph.string(path);
            self.graph.set(storage, "UIKeyPath", path);
            entries.sort_by_key(|(condition, _)| *condition != "default");
            let mut records = Vec::new();
            let mut candidates = original.clone();
            for (condition, mask) in entries {
                let mut added = Vec::new();
                let mut removed = Vec::new();
                for change in &mask.children {
                    let reference = change
                        .attr("reference")
                        .ok_or("a variation mask entry has no reference")?;
                    let target = self
                        .lookup(reference)
                        .ok_or_else(|| format!("unknown variation reference {reference}"))?;
                    if !candidates.contains(&target) {
                        candidates.push(target);
                    }
                    match change.name.as_str() {
                        "include" => added.push(target),
                        "exclude" => removed.push(target),
                        other => return Err(format!("unsupported variation mask change {other}")),
                    }
                }
                if condition == "default" {
                    let defaults: Vec<_> = original
                        .iter()
                        .copied()
                        .filter(|id| !removed.contains(id))
                        .collect();
                    if defaults.is_empty() {
                        self.graph.remove(object, archive_key);
                    } else {
                        let defaults = self.graph.array("NSMutableArray", &defaults);
                        self.graph.replace(object, archive_key, defaults);
                    }
                }
                let record = self.graph.object("_UIRelationshipTraitStorageRecord");
                let traits = self.trait_collection(condition)?;
                self.graph.set(record, "UITraitCollection", traits);
                let added = self.set_objects("NSSet", &added);
                self.graph.set(record, "UIAddedObjects", added);
                let removed = self.set_objects("NSSet", &removed);
                self.graph.set(record, "UIRemovedObjects", removed);
                records.push(record);
            }
            let records = self.graph.array("NSMutableArray", &records);
            self.graph.set(storage, "UIRecords", records);
            let candidates = self.set_objects("NSOrderedSet", &candidates);
            self.graph
                .set(storage, "UIRelationshipCandidates", candidates);
            self.trait_storages
                .push((self.current_trait_root.unwrap_or(object), object, storage));
        }
        Ok(())
    }

    pub(super) fn install_traits(&mut self, root: Id) {
        let mut groups: Vec<(Id, Vec<Id>, Vec<Id>)> = Vec::new();
        for &(top, target, storage) in &self.trait_storages {
            let group = if let Some(index) = groups.iter().position(|(id, _, _)| *id == top) {
                index
            } else {
                groups.push((top, Vec::new(), Vec::new()));
                groups.len() - 1
            };
            groups[group].1.push(storage);
            if target != top && !groups[group].2.contains(&target) {
                groups[group].2.push(target);
            }
        }
        if groups.is_empty() {
            return;
        }
        let mut lists = Vec::new();
        for (top, storages, mut descendants) in groups {
            for &(root, view) in &self.trait_descendants {
                if root == top && view != top && !descendants.contains(&view) {
                    descendants.push(view);
                }
            }
            let list = self.graph.object("_UITraitStorageList");
            self.graph.set(list, "UITopLevelObject", top);
            let storages = self.graph.array("NSArray", &storages);
            self.graph.set(list, "UITraitStorages", storages);
            let descendants = self.set_objects("NSSet", &descendants);
            self.graph.set(list, "UIDescendants", descendants);
            lists.push(list);
        }
        let lists = self.graph.array("NSArray", &lists);
        self.graph.replace(root, "UINibTraitStorageListsKey", lists);
    }
}
