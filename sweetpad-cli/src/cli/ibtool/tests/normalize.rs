//! Compare links by their destinations. Xcode assigns upstream proxy numbers
//! from hash tables; their names are local to each loading boundary.
use super::{Plist, Read, keyed_archive, read};
use std::collections::BTreeMap;
use std::fmt::Write as _;
mod layout_cache;
type Objects = Vec<read::Object>;
pub(super) type Names = BTreeMap<String, String>;
fn reference(objects: &[read::Object], object: usize, key: &str) -> Option<usize> {
    objects[object].values.iter().find_map(|(k, v)| {
        if k == key {
            if let Read::Object(id) = v {
                Some(*id)
            } else {
                None
            }
        } else {
            None
        }
    })
}
fn string(objects: &[read::Object], object: usize) -> Option<String> {
    objects[object].values.iter().find_map(|(k, v)| {
        if k == "NS.bytes" {
            if let Read::Data(bytes) = v {
                String::from_utf8(bytes.clone()).ok()
            } else {
                None
            }
        } else {
            None
        }
    })
}
fn pairs(objects: &[read::Object], object: usize) -> Vec<(usize, usize)> {
    let entries: Vec<_> = objects[object]
        .values
        .iter()
        .filter_map(|(k, v)| {
            if k == "UINibEncoderEmptyKey" {
                if let Read::Object(id) = v {
                    Some(*id)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    entries.chunks_exact(2).map(|p| (p[0], p[1])).collect()
}
fn destination(objects: &[read::Object], object: usize) -> String {
    if objects[object].class == "UIProxyObject" {
        return reference(objects, object, "UIProxiedObjectIdentifier")
            .and_then(|id| string(objects, id))
            .unwrap();
    }
    let mut name = objects[object].class.clone();
    for key in [
        "UINibName",
        "UIClassName",
        "UIIdentifier",
        "UIDestinationViewControllerIdentifier",
        "UIActionName",
        "NSClassName",
        "NSStoryboardIdentifier",
        "NSNibName",
        "NSDestinationControllerIdentifier",
        "NSIdentifier",
    ] {
        if let Some(value) = reference(objects, object, key).and_then(|id| string(objects, id)) {
            let _ = write!(name, " {key}={value}");
        }
    }
    if let Some(children) = reference(objects, object, "UIChildViewControllers") {
        for (_, value) in &objects[children].values {
            if let Read::Object(child) = value {
                let _ = write!(name, " child=[{}]", destination(objects, *child));
            }
        }
    }
    name
}
pub(super) fn upstreams(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Names> {
    let mut result = BTreeMap::new();
    for data in files.values().filter(|d| d.starts_with(b"NIBArchive")) {
        let objects = read::parse(data);
        for object in 0..objects.len() {
            for (name_key, table_key) in [
                ("UINibName", "UIExternalObjectsTableForViewLoading"),
                ("NSNibName", "NSExternalObjectsTableForViewLoading"),
            ] {
                if let (Some(nib), Some(table)) = (
                    reference(&objects, object, name_key),
                    reference(&objects, object, table_key),
                ) {
                    let names: Names = pairs(&objects, table)
                        .into_iter()
                        .map(|(k, v)| (string(&objects, k).unwrap(), destination(&objects, v)))
                        .collect();
                    result.insert(format!("{}.nib", string(&objects, nib).unwrap()), names);
                }
            }
        }
    }
    if let Some(info) = files.get("Info.plist") {
        let mut names = uuid_names(&keyed_archive::read::parse(info));
        embedded_uuids(files, &mut names);
        for file in files.keys().filter(|file| {
            std::path::Path::new(file)
                .extension()
                .is_some_and(|ext| ext == "nib")
        }) {
            result
                .entry(file.clone())
                .or_default()
                .extend(names.clone());
        }
    }
    result
}

fn embedded_uuids(files: &BTreeMap<String, Vec<u8>>, names: &mut Names) {
    for data in files
        .values()
        .filter(|data| data.starts_with(b"NIBArchive"))
    {
        let objects = read::parse(data);
        for object in 0..objects.len() {
            if let Some(uuid) =
                reference(&objects, object, "uniqueIdentifierForStoryboardCompilation")
                    .and_then(|id| string(&objects, id))
                && let Some(nib) =
                    reference(&objects, object, "NSNibName").and_then(|id| string(&objects, id))
            {
                names
                    .entry(uuid)
                    .or_insert_with(|| format!("embedded-controller:{nib}"));
            }
        }
    }
}

fn uuid_names(info: &Plist) -> Names {
    let Plist::Dict(info) = info else {
        return Names::new();
    };
    let Some(Plist::Dict(uuids)) = info
        .iter()
        .find_map(|(k, v)| (k == "NSViewControllerIdentifiersToUUIDs").then_some(v))
    else {
        return Names::new();
    };
    let mut result = Names::new();
    let mut identifiers = std::collections::BTreeSet::new();
    for (identifier, uuid) in uuids {
        if !identifiers.insert(identifier) {
            return Names::new();
        }
        let Plist::String(uuid) = uuid else {
            return Names::new();
        };
        if uuid.len() != 36
            || !uuid.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            })
            || result
                .insert(uuid.clone(), format!("storyboard-controller:{identifier}"))
                .is_some()
        {
            return Names::new();
        }
    }
    result
}

pub(super) fn metadata(data: &[u8]) -> Plist {
    let mut info = keyed_archive::read::parse(data);
    let names = uuid_names(&info);
    if let Plist::Dict(entries) = &mut info {
        for (key, value) in entries {
            if key == "NSViewControllerIdentifiersToUUIDs"
                && let Plist::Dict(uuids) = value
            {
                for (_, uuid) in uuids {
                    if let Plist::String(value) = uuid
                        && let Some(name) = names.get(value)
                    {
                        *value = name.clone();
                    }
                }
            }
        }
    }
    info
}

#[test]
fn storyboard_uuid_normalization_preserves_controller_links() {
    use crate::cli::ibtool::nib::Graph;
    fn files(first: &str, second: &str, wrong_link: bool) -> BTreeMap<String, Vec<u8>> {
        let info = keyed_archive::write(&Plist::Dict(vec![(
            "NSViewControllerIdentifiersToUUIDs".into(),
            Plist::Dict(vec![
                ("A".into(), Plist::String(first.into())),
                ("B".into(), Plist::String(second.into())),
            ]),
        )]));
        let mut graph = Graph::default();
        let controller = graph.object("NSViewController");
        let uuid = graph.string(if wrong_link { second } else { first });
        graph.set(controller, "uniqueIdentifierForStoryboardCompilation", uuid);
        BTreeMap::from([
            ("Info.plist".into(), info),
            ("A.nib".into(), graph.write(controller)),
        ])
    }
    let first = "11111111-1111-4111-8111-111111111111";
    let second = "22222222-2222-4222-8222-222222222222";
    let third = "33333333-3333-4333-8333-333333333333";
    let original = files(first, second, false);
    let renamed = files(third, first, false);
    let wrong = files(first, second, true);
    let duplicate = files(first, first, false);
    let canonical = |files: &BTreeMap<String, Vec<u8>>| {
        super::canonical_with(&files["A.nib"], &upstreams(files)["A.nib"])
    };
    assert_eq!(
        metadata(&original["Info.plist"]),
        metadata(&renamed["Info.plist"])
    );
    assert_eq!(canonical(&original), canonical(&renamed));
    assert_ne!(canonical(&original), canonical(&wrong));
    assert_ne!(
        metadata(&original["Info.plist"]),
        metadata(&duplicate["Info.plist"])
    );
}
fn new_string(objects: &mut Objects, text: String) -> usize {
    let id = objects.len();
    objects.push(read::Object {
        class: "NSString".to_string(),
        values: vec![("NS.bytes".to_string(), Read::Data(text.into_bytes()))],
    });
    id
}
pub(super) fn prepare(objects: &mut Objects, names: &Names) {
    rename_proxies(objects, names);
    for object in 0..objects.len() {
        for field in [
            "uniqueIdentifierForStoryboardCompilation",
            "NSUniqueControllerIdentifier",
        ] {
            if let Some(id) = reference(objects, object, field)
                && let Some(uuid) = string(objects, id)
                && let Some(name) = names.get(&uuid)
            {
                for (key, value) in &mut objects[id].values {
                    if key == "NS.bytes" {
                        *value = Read::Data(name.as_bytes().to_vec());
                    }
                }
            }
        }
    }
    controller_tables(objects);
    prototype_tables(objects);
    top_level_proxies(objects);
    appkit_layout_cache(objects);
    layout_cache::prepare(objects);
    appkit_tables(objects);
    intern_strings(objects);
}

fn appkit_layout_cache(objects: &mut Objects) {
    // Xcode changes these private table caches when window services are
    // unavailable in a sandbox. Keep table/scroller visibility, geometry,
    // and other flags intact; runtime checks compare the loaded controls.
    for table in 0..objects.len() {
        if !(matches!(
            objects[table].class.as_str(),
            "NSTableView" | "NSOutlineView"
        ) || (objects[table].class == "NSClassSwapper"
            && reference(objects, table, "NSOriginalClassName")
                .and_then(|id| string(objects, id))
                .as_deref()
                .is_some_and(|class| matches!(class, "NSTableView" | "NSOutlineView"))))
        {
            continue;
        }
        if let Some(corner) = reference(objects, table, "NSCornerView")
            && objects[corner].class == "_NSCornerView"
            && reference(objects, corner, "NSSuperview").is_none()
        {
            for (key, value) in &mut objects[corner].values {
                if key == "NSvFlags" && *value == Read::Int(-2_147_483_392) {
                    *value = Read::Int(256);
                }
            }
        }
        let Some(clip) = reference(objects, table, "NSSuperview") else {
            continue;
        };
        if objects[clip].class != "NSClipView" {
            continue;
        }
        let Some(scroll) = reference(objects, clip, "NSSuperview") else {
            continue;
        };
        if objects[scroll].class != "NSScrollView" {
            continue;
        }
        for key in ["NSHScroller", "NSVScroller"] {
            if let Some(scroller) = reference(objects, scroll, key)
                && objects[scroller].class == "NSScroller"
                && objects[scroller].values.iter().any(|(key, value)| {
                    key == "NSvFlags" && matches!(value, Read::Int(n) if n & 0x8000_0000 != 0)
                })
            {
                objects[scroller].values.retain(|(key, value)| {
                    key != "NSViewIsLayerTreeHost" || *value != Read::Bool(true)
                });
            }
        }
    }
}

#[test]
fn table_cache_normalization_preserves_public_visibility_and_flags() {
    use crate::cli::ibtool::nib::Graph;
    fn archive(cache_changed: bool, hidden: bool, corner_flags: i64) -> Vec<u8> {
        let mut graph = Graph::default();
        let table = graph.object("NSClassSwapper");
        let original = graph.string("NSTableView");
        graph.set(table, "NSOriginalClassName", original);
        graph.set(
            table,
            "NSvFlags",
            if hidden { -2_147_483_392_i64 } else { 256 },
        );
        let corner = graph.object("_NSCornerView");
        graph.set(table, "NSCornerView", corner);
        graph.set(corner, "NSvFlags", corner_flags);
        let clip = graph.object("NSClipView");
        let scroll = graph.object("NSScrollView");
        let scroller = graph.object("NSScroller");
        graph.set(table, "NSSuperview", clip);
        graph.set(clip, "NSSuperview", scroll);
        graph.set(scroll, "NSHScroller", scroller);
        graph.set(scroller, "NSvFlags", -2_147_483_392_i64);
        if cache_changed {
            graph.set(scroller, "NSViewIsLayerTreeHost", true);
        }
        graph.write(table)
    }
    let expected = super::canonical(&archive(false, false, -2_147_483_392));
    assert_eq!(expected, super::canonical(&archive(true, false, 256)));
    assert_ne!(expected, super::canonical(&archive(true, true, 256)));
    assert_ne!(expected, super::canonical(&archive(true, false, 257)));
}

fn array_references(objects: &Objects, array: usize) -> Option<Vec<usize>> {
    objects[array]
        .values
        .iter()
        .filter(|(key, _)| key == "UINibEncoderEmptyKey")
        .map(|(_, value)| match value {
            Read::Object(id) => Some(*id),
            _ => None,
        })
        .collect()
}

fn set_array_references(objects: &mut Objects, array: usize, entries: &[usize]) {
    let mut entries = entries.iter();
    for (key, value) in &mut objects[array].values {
        if key == "UINibEncoderEmptyKey" {
            *value = Read::Object(*entries.next().unwrap());
        }
    }
    assert!(entries.next().is_none());
}

fn appkit_signature(objects: &Objects, object: usize) -> String {
    let signature = super::signature(objects, object, 3);
    if objects[object].class != "NSNibExternalObjectPlaceholder" {
        return signature;
    }
    // Several controllers can have distinct placeholders for the same
    // storyboard. Their incoming outlets distinguish metadata sort ties.
    let mut incoming = (0..objects.len())
        .filter(|id| reference(objects, *id, "NSDestination") == Some(object))
        .map(|id| super::signature(objects, id, 3))
        .collect::<Vec<_>>();
    incoming.sort();
    format!("{signature} incoming={incoming:?}")
}

fn appkit_owners(objects: &mut Objects, table: usize) {
    // These are parallel dictionary arrays. Sorting them independently
    // would hide a wrong owner, so move each key with its value.
    if let (Some(keys), Some(values)) = (
        reference(objects, table, "NSObjectsKeys"),
        reference(objects, table, "NSObjectsValues"),
    ) && let (Some(key_ids), Some(value_ids)) = (
        array_references(objects, keys),
        array_references(objects, values),
    ) && key_ids.len() == value_ids.len()
    {
        let mut pairs = key_ids.into_iter().zip(value_ids).collect::<Vec<_>>();
        pairs.sort_by_cached_key(|(key, value)| {
            (
                appkit_signature(objects, *key),
                super::signature(objects, *value, 3),
            )
        });
        set_array_references(
            objects,
            keys,
            &pairs.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
        );
        set_array_references(
            objects,
            values,
            &pairs.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
        );
    }
}

fn object_integer(object: &read::Object) -> Option<i64> {
    if object.class != "NSNumber" || object.values.len() != 1 {
        return None;
    }
    match (&*object.values[0].0, &object.values[0].1) {
        ("NS.intval", Read::Int(n)) => Some(*n),
        ("NS.dblval", Read::Double(n)) if n.is_finite() && n.fract() == 0.0 => {
            format!("{n:.0}").parse().ok()
        }
        _ => None,
    }
}

fn appkit_tables(objects: &mut Objects) {
    let tables = (0..objects.len())
        .filter(|id| objects[*id].class == "NSIBObjectData")
        .collect::<Vec<_>>();
    for table in tables {
        appkit_owners(objects, table);
        // Rename both local number spaces together. Accessibility connector
        // targets remain ordinary object links and are compared in full.
        let mut groups = Vec::new();
        let mut all_numbers = std::collections::BTreeSet::new();
        let mut all_keys = std::collections::BTreeSet::new();
        let mut valid = true;
        for (key_name, value_name) in [
            ("NSOidsKeys", "NSOidsValues"),
            ("NSAccessibilityOidsKeys", "NSAccessibilityOidsValues"),
        ] {
            let (keys, values) = (
                reference(objects, table, key_name),
                reference(objects, table, value_name),
            );
            if keys.is_none() && values.is_none() && key_name == "NSAccessibilityOidsKeys" {
                continue;
            }
            let (Some(keys), Some(values)) = (keys, values) else {
                valid = false;
                break;
            };
            let (Some(mut key_ids), Some(value_ids)) = (
                array_references(objects, keys),
                array_references(objects, values),
            ) else {
                valid = false;
                break;
            };
            if key_ids.len() != value_ids.len()
                || !key_ids.iter().all(|key| all_keys.insert(*key))
                || !value_ids.iter().all(|id| {
                    object_integer(&objects[*id]).is_some_and(|n| n > 0 && all_numbers.insert(n))
                })
            {
                valid = false;
                break;
            }
            key_ids.sort_by_cached_key(|id| appkit_signature(objects, *id));
            groups.push((keys, values, key_ids));
        }
        if !valid {
            continue;
        }
        let mut next_number = 1_i64;
        for (keys, values, key_ids) in groups {
            let mut numbered = Vec::new();
            for _ in &key_ids {
                let number = objects
                    .iter()
                    .position(|object| object_integer(object) == Some(next_number))
                    .unwrap_or_else(|| {
                        let id = objects.len();
                        objects.push(read::Object {
                            class: "NSNumber".into(),
                            values: vec![("NS.intval".into(), Read::Int(next_number))],
                        });
                        id
                    });
                numbered.push(number);
                next_number += 1;
            }
            set_array_references(objects, keys, &key_ids);
            set_array_references(objects, values, &numbered);
        }
    }
}

#[test]
fn appkit_metadata_order_preserves_ownership_and_membership() {
    use crate::cli::ibtool::nib::{Graph, Value};
    #[derive(Clone, Copy, PartialEq)]
    enum Corruption {
        None,
        WrongOwner,
        DuplicateId,
        DuplicateKey,
    }
    fn archive(reordered: bool, corruption: Corruption) -> Vec<u8> {
        let mut graph = Graph::default();
        let root = graph.object("NSObject");
        let table = graph.object("NSIBObjectData");
        graph.set(root, "IB.objectdata", table);
        let owner = graph.object("NSCustomObject");
        graph.set(table, "NSRoot", owner);
        let first = graph.object("NSView");
        graph.set(first, "NSTag", 1_i64);
        let second = graph.object("NSView");
        graph.set(second, "NSTag", 2_i64);
        let keys = if reordered {
            [second, first]
        } else {
            [first, second]
        };
        let parents = keys.map(|key| {
            if corruption == Corruption::WrongOwner && key == second {
                first
            } else {
                owner
            }
        });
        for (key, ids) in [("NSObjectsKeys", keys), ("NSObjectsValues", parents)] {
            let array = graph.array("NSArray", &ids);
            graph.set(table, key, array);
        }
        let mut keys = if reordered {
            [second, owner, first]
        } else {
            [owner, first, second]
        };
        if corruption == Corruption::DuplicateKey {
            keys[2] = keys[1];
        }
        let numbers = if corruption == Corruption::DuplicateId {
            [1, 1, 3]
        } else if reordered {
            [19, 4, 52]
        } else {
            [1, 2, 3]
        };
        let values =
            numbers.map(|value| graph.shared("NSNumber", vec![("NS.intval", Value::Int(value))]));
        for (key, ids) in [("NSOidsKeys", keys), ("NSOidsValues", values)] {
            let array = graph.array("NSArray", &ids);
            graph.set(table, key, array);
        }
        graph.write(root)
    }
    let expected = super::canonical(&archive(false, Corruption::None));
    assert_eq!(expected, super::canonical(&archive(true, Corruption::None)));
    assert_ne!(
        expected,
        super::canonical(&archive(true, Corruption::DuplicateId))
    );
    assert_ne!(
        expected,
        super::canonical(&archive(true, Corruption::WrongOwner))
    );
    assert_ne!(
        expected,
        super::canonical(&archive(true, Corruption::DuplicateKey))
    );
}

fn rename_proxies(objects: &mut Objects, names: &Names) {
    let count = objects.len();
    for object in 0..count {
        let identifier = match objects[object].class.as_str() {
            "UIProxyObject" => "UIProxiedObjectIdentifier",
            "NSNibExternalObjectPlaceholder" => "NSExternalObjectPlaceholderIdentifier",
            _ => continue,
        };
        if let Some(id) = reference(objects, object, identifier)
            && let Some(name) = string(objects, id).and_then(|name| names.get(&name).cloned())
        {
            let new = new_string(objects, name);
            for (key, value) in &mut objects[object].values {
                if key == identifier {
                    *value = Read::Object(new);
                }
            }
        }
    }
    for object in 0..count {
        if objects[object].class == "UIRuntimeOutletConnection"
            && reference(objects, object, "UILabel")
                .and_then(|id| string(objects, id))
                .as_deref()
                == Some("storyboard")
            && let (Some(source), Some(proxy)) = (
                reference(objects, object, "UISource"),
                reference(objects, object, "UIDestination"),
            )
        {
            let label = format!("UIStoryboardPlaceholder {}", destination(objects, source));
            let id = new_string(objects, label);
            for (key, value) in &mut objects[proxy].values {
                if key == "UIProxiedObjectIdentifier" {
                    *value = Read::Object(id);
                }
            }
        }
    }
}

fn controller_tables(objects: &mut Objects) {
    let count = objects.len();
    // Each controller's table gives the names meaning independently.
    for object in 0..count {
        if let Some(table) = reference(objects, object, "UIExternalObjectsTableForViewLoading") {
            let labels: Vec<_> = pairs(objects, table)
                .into_iter()
                .map(|(_, v)| destination(objects, v))
                .collect();
            let mut labels = labels.into_iter();
            let mut key = true;
            let indices: Vec<_> = objects[table]
                .values
                .iter()
                .enumerate()
                .filter_map(|(i, (k, _))| (k == "UINibEncoderEmptyKey").then_some(i))
                .collect();
            for index in indices {
                if key {
                    let id = new_string(objects, labels.next().unwrap());
                    objects[table].values[index].1 = Read::Object(id);
                }
                key = !key;
            }
        }
    }
}

fn prototype_tables(objects: &mut Objects) {
    let count = objects.len();
    // Prototype nibs introduce a second boundary with their own registration
    // tables. Resolve those links before comparing the embedded archive.
    for object in 0..count {
        for (external, nibs) in [
            (
                "UITableViewCellPrototypeNibExternalObjects",
                "UITableViewCellPrototypeNibs",
            ),
            (
                "UICollectionViewCellPrototypeNibExternalObjects",
                "UICollectionViewCellNibDict",
            ),
            (
                "UICollectionViewSupplementaryViewPrototypeNibExternalObjects",
                "UICollectionViewSupplementaryViewNibDict",
            ),
        ] {
            let (Some(external), Some(nibs)) = (
                reference(objects, object, external),
                reference(objects, object, nibs),
            ) else {
                continue;
            };
            let registrations: BTreeMap<_, _> = pairs(objects, external)
                .into_iter()
                .map(|(k, v)| (string(objects, k).unwrap(), v))
                .collect();
            for (key, nib) in pairs(objects, nibs) {
                let table = registrations[&string(objects, key).unwrap()];
                let names: Names = pairs(objects, table)
                    .into_iter()
                    .map(|(k, v)| (string(objects, k).unwrap(), destination(objects, v)))
                    .collect();
                if let Some(data) = reference(objects, nib, "archiveData") {
                    for (_, value) in &mut objects[data].values {
                        if let Read::Data(bytes) = value
                            && bytes.starts_with(b"NIBArchive")
                        {
                            *bytes = format!(
                                "canonical nested nib:\n{}",
                                super::canonical_with(bytes, &names)
                            )
                            .into_bytes();
                        }
                    }
                }
                let labels: Vec<_> = pairs(objects, table)
                    .into_iter()
                    .map(|(_, v)| destination(objects, v))
                    .collect();
                let indices: Vec<_> = objects[table]
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, (k, _))| (k == "UINibEncoderEmptyKey").then_some(i))
                    .step_by(2)
                    .collect();
                for (index, label) in indices.into_iter().zip(labels) {
                    let id = new_string(objects, label);
                    objects[table].values[index].1 = Read::Object(id);
                }
            }
        }
    }
}

fn top_level_proxies(objects: &mut Objects) {
    // Proxy order in the top-level array has no loading order. Preserve the
    // order of application objects while sorting the proxy positions.
    if let Some(top) = reference(objects, 0, "UINibTopLevelObjectsKey") {
        let indices: Vec<_> = objects[top]
            .values
            .iter()
            .enumerate()
            .filter_map(|(index, (_, value))| match value {
                Read::Object(id) if objects[*id].class == "UIProxyObject" => Some(index),
                _ => None,
            })
            .collect();
        let mut proxies: Vec<_> = indices
            .iter()
            .map(|i| objects[top].values[*i].1.clone())
            .collect();
        proxies.sort_by_cached_key(|v| match v {
            Read::Object(id) => destination(objects, *id),
            _ => unreachable!(),
        });
        for (index, value) in indices.into_iter().zip(proxies) {
            objects[top].values[index].1 = value;
        }
    }
}

fn intern_strings(objects: &mut Objects) {
    // NSString value sharing is an encoder optimization, not object identity
    // on which UIKit relies. Intern equal values in both archives.
    let mut strings = BTreeMap::new();
    let mut aliases = BTreeMap::new();
    for (index, object) in objects
        .iter()
        .enumerate()
        .filter(|(_, o)| o.class == "NSString")
    {
        let key = format!("{:?}", object.values);
        let first = *strings.entry(key).or_insert(index);
        aliases.insert(index, first);
    }
    for object in objects {
        for (_, value) in &mut object.values {
            if let Read::Object(id) = value
                && let Some(first) = aliases.get(id)
            {
                *id = *first;
            }
        }
    }
}

#[test]
fn accessibility_numbering_preserves_connector_targets_and_unique_ids() {
    use crate::cli::ibtool::nib::{Graph, Value};
    fn archive(renumbered: bool, wrong_target: bool, duplicate: bool) -> Vec<u8> {
        let mut graph = Graph::default();
        let root = graph.object("NSObject");
        let table = graph.object("NSIBObjectData");
        graph.set(root, "IB.objectdata", table);
        let first = graph.object("NSView");
        graph.set(first, "NSTag", 1_i64);
        let second = graph.object("NSView");
        graph.set(second, "NSTag", 2_i64);
        let connector = graph.object("NSNibAXAttributeConnector");
        graph.set(
            connector,
            "AXDestinationArchiveKey",
            if wrong_target { second } else { first },
        );
        for (key, ids) in [
            ("NSOidsKeys", vec![first, second]),
            ("NSAccessibilityOidsKeys", vec![connector]),
            ("NSAccessibilityConnectors", vec![connector]),
        ] {
            let array = graph.array("NSArray", &ids);
            graph.set(table, key, array);
        }
        for (key, values) in [
            (
                "NSOidsValues",
                if renumbered { vec![17, 9] } else { vec![1, 2] },
            ),
            (
                "NSAccessibilityOidsValues",
                vec![if duplicate {
                    1
                } else if renumbered {
                    40
                } else {
                    3
                }],
            ),
        ] {
            let values = values
                .into_iter()
                .map(|v| graph.shared("NSNumber", vec![("NS.intval", Value::Int(v))]))
                .collect::<Vec<_>>();
            let array = graph.array("NSArray", &values);
            graph.set(table, key, array);
        }
        graph.write(root)
    }
    let expected = super::canonical(&archive(false, false, false));
    assert_eq!(expected, super::canonical(&archive(true, false, false)));
    assert_ne!(expected, super::canonical(&archive(false, true, false)));
    assert_ne!(expected, super::canonical(&archive(false, false, true)));
}
