//! AppKit uses the same NIBArchive container as UIKit, with NSIBObjectData
//! describing object ownership and connections instead of the UINib root.
use std::collections::{HashMap, HashSet};

use base64::Engine as _;
use sweetpad_lib::xcscheme::Element;

use super::{
    Options, custom_class,
    nib::{Graph, Id, Value},
};

mod bindings;
mod boxes;
mod collections;
mod colors;
mod constraints;
mod controls;
mod fonts;
mod grids;
mod layout;
mod menus;
mod outlines;
mod prototypes;
mod scrolls;
mod stacks;
mod storyboards;
mod tables;
mod texts;
mod views;
mod web;
mod windows;

struct Encoder<'a> {
    graph: Graph,
    options: &'a Options<'a>,
    storyboard: bool,
    ids: HashMap<String, Id>,
    owners: Vec<(Id, Id)>,
    oids: Vec<Id>,
    visible_windows: Vec<Id>,
    images: HashMap<String, [f64; 2]>,
    image_ids: HashMap<String, Id>,
    named_colors: HashMap<String, Element>,
    constrained: HashSet<String>,
    baseline_measured: HashSet<String>,
    accessibility: Vec<Id>,
    early_connections: Vec<Id>,
    late_connections: Vec<Id>,
    root_views: HashSet<Id>,
}

fn id(element: &Element) -> Result<&str, String> {
    element
        .attr("id")
        .ok_or_else(|| format!("<{}> has no id", element.name))
}

fn number(element: &Element, key: &str, fallback: f64) -> Result<f64, String> {
    element.attr(key).map_or(Ok(fallback), |s| {
        s.parse().map_err(|_| format!("invalid {key}: {s}"))
    })
}

fn yes(element: &Element, key: &str) -> bool {
    element.attr(key) == Some("YES")
}
fn no(element: &Element, key: &str) -> bool {
    element.attr(key) == Some("NO")
}

fn string_value(element: &Element, key: &str) -> Result<Option<String>, String> {
    if let Some(value) = element.attr(key) {
        return Ok(Some(value.to_string()));
    }
    let Some(value) = element
        .children_named("string")
        .find(|e| e.attr("key") == Some(key))
    else {
        return Ok(None);
    };
    if !yes(value, "base64-UTF8") {
        return Ok(Some(value.text.clone()));
    }
    let text = value
        .text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect::<Vec<_>>();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&text)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&text))
        .map_err(|error| format!("invalid Base64 {key}: {error}"))?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| format!("invalid UTF-8 {key}: {error}"))
}

fn collect_constraints(
    element: &Element,
    owner: Option<&str>,
    constrained: &mut HashSet<String>,
    baseline_measured: &mut HashSet<String>,
) {
    if element.name == "constraint" && yes(element, "placeholder") {
        return;
    }
    if element.name == "gridView" {
        if let Some(cells) = element.child("gridCells") {
            for cell in &cells.children {
                if let Some(view) = grids::content(cell)
                    && let Some(id) = view.attr("id")
                {
                    baseline_measured.insert(id.into());
                }
            }
        }

        if let Some(id) = element.attr("id") {
            constrained.insert(id.to_string());
        }
        if let Some(cells) = element.child("gridCells") {
            constrained.extend(
                cells
                    .children
                    .iter()
                    .filter(|c| grids::visible(c, element))
                    .filter_map(|c| {
                        grids::content(c)
                            .and_then(|v| v.attr("id"))
                            .map(str::to_string)
                    }),
            );
        }
    }
    if element.name == "stackView"
        && let Some(views) = element.child("subviews")
        && !views.children.is_empty()
    {
        if let Some(id) = element.attr("id") {
            constrained.insert(id.to_string());
        }
        constrained.extend(
            views
                .children
                .iter()
                .filter_map(|v| v.attr("id").map(str::to_string)),
        );
    }
    if element.name == "constraint" {
        for (item, attribute) in [
            (
                element.attr("firstItem").or(owner),
                element.attr("firstAttribute"),
            ),
            (element.attr("secondItem"), element.attr("secondAttribute")),
        ] {
            if let Some(item) = item
                && matches!(
                    attribute,
                    Some("baseline" | "firstBaseline" | "lastBaseline")
                )
            {
                baseline_measured.insert(item.into());
            }
        }
        if let Some(first) = element.attr("firstItem").or(owner) {
            constrained.insert(first.to_string());
        }
        if let Some(second) = element.attr("secondItem") {
            constrained.insert(second.to_string());
        }
    }
    let owner = element.attr("id").or(owner);
    for child in &element.children {
        collect_constraints(child, owner, constrained, baseline_measured);
    }
}

fn placeholder_view(element: &Element) -> bool {
    element.attr("customClass").is_none()
        && element
            .child("subviews")
            .is_none_or(|children| children.children.is_empty())
}

pub(super) fn compile_storyboard(
    document: &Element,
    options: &Options<'_>,
) -> Result<super::storyboard::Compiled, String> {
    storyboards::compile(document, options)
}

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Vec<u8>, String> {
    let mut document = document.clone();
    layout::prepare(&mut document);
    let mut document = scrolls::prepare(&document)?;
    prototypes::prepare(&mut document)?;
    let storyboard = document
        .attr("type")
        .is_some_and(|kind| kind.contains("Storyboard"));
    if !storyboard
        && let Some(objects) = document.children.iter_mut().find(|c| c.name == "objects")
        && !objects.children.iter().any(|c| c.attr("id") == Some("-3"))
    {
        objects.children.push(Element {
            name: "customObject".into(),
            attributes: vec![("id".into(), "-3".into())],
            children: vec![],
            text: String::new(),
        });
    }
    let objects = document.child("objects").ok_or("the XIB has no objects")?;
    let mut encoder = Encoder {
        graph: Graph::default(),
        options,
        storyboard,
        ids: HashMap::new(),
        owners: Vec::new(),
        oids: Vec::new(),
        visible_windows: Vec::new(),
        images: HashMap::new(),
        image_ids: HashMap::new(),
        named_colors: HashMap::new(),
        constrained: HashSet::new(),
        baseline_measured: HashSet::new(),
        accessibility: Vec::new(),
        early_connections: Vec::new(),
        late_connections: Vec::new(),
        root_views: HashSet::new(),
    };
    for resource in document
        .children_named("resources")
        .flat_map(|r| r.children_named("image"))
    {
        if let Some(name) = resource.attr("name") {
            encoder.images.insert(
                name.to_string(),
                [
                    number(resource, "width", 0.0)?,
                    number(resource, "height", 0.0)?,
                ],
            );
        }
    }
    for color in document
        .children_named("resources")
        .flat_map(|r| r.children_named("namedColor"))
    {
        if let Some(name) = color.attr("name")
            && let Some(value) = color.child("color")
        {
            encoder.named_colors.insert(name.into(), value.clone());
        }
    }
    collect_constraints(
        objects,
        None,
        &mut encoder.constrained,
        &mut encoder.baseline_measured,
    );
    encoder.declare(objects, false)?;
    let owner = objects
        .children
        .iter()
        .find(|e| e.attr("id") == Some("-2"))
        .ok_or("the AppKit XIB has no File's Owner")?;
    let root_owner = encoder.object(owner)?;
    if owner.attr("customClass").is_some() {
        encoder.class_reference(owner, root_owner);
    }
    encoder.string(
        root_owner,
        "NSClassName",
        &custom_class(owner, options).unwrap_or_else(|| "NSObject".into()),
    );
    encoder.oids.push(root_owner);
    for element in &objects.children {
        if matches!(element.attr("id"), Some("-1" | "-2")) {
            continue;
        }
        encoder.encode(element, root_owner)?;
    }
    let mut connections = std::mem::take(&mut encoder.early_connections);
    connections.extend(encoder.connections_for(&document, false)?);
    connections.extend(encoder.connections_for(&document, true)?);
    // Assigning a window's content controller can load its view. Resolve
    // storyboard and other outlets before those runtime assignments.
    connections.extend(std::mem::take(&mut encoder.late_connections));
    Ok(encoder.archive(root_owner, &connections))
}

impl Encoder<'_> {
    fn archive(mut self, root_owner: Id, connections: &[Id]) -> Vec<u8> {
        let data = self.graph.object("NSIBObjectData");
        self.graph.set(data, "NSRoot", root_owner);
        let empty_set = self.graph.array("NSMutableSet", &self.visible_windows);
        self.graph.set(data, "NSVisibleWindows", empty_set);
        let connections = self.graph.array("NSMutableArray", connections);
        self.graph.set(data, "NSConnections", connections);
        let keys = self.owners.iter().map(|(k, _)| *k).collect::<Vec<_>>();
        let values = self.owners.iter().map(|(_, v)| *v).collect::<Vec<_>>();
        for (key, values) in [("NSObjectsKeys", keys), ("NSObjectsValues", values)] {
            let values = self.graph.array("NSArray", &values);
            self.graph.set(data, key, values);
        }
        let keys = self.graph.array("NSArray", &self.oids);
        let values = (1..=self.oids.len())
            .map(|index| {
                self.graph.shared(
                    "NSNumber",
                    vec![("NS.intval", Value::Int(i64::try_from(index).unwrap()))],
                )
            })
            .collect::<Vec<_>>();
        let values = self.graph.array("NSArray", &values);
        self.graph.set(data, "NSOidsKeys", keys);
        self.graph.set(data, "NSOidsValues", values);
        let connectors = self.graph.array("NSMutableArray", &self.accessibility);
        self.graph
            .set(data, "NSAccessibilityConnectors", connectors);
        let keys = self.graph.array("NSArray", &self.accessibility);
        self.graph.set(data, "NSAccessibilityOidsKeys", keys);
        let values = (0..self.accessibility.len())
            .map(|index| {
                self.graph.shared(
                    "NSNumber",
                    vec![(
                        "NS.intval",
                        Value::Int(i64::try_from(self.oids.len() + index + 1).unwrap()),
                    )],
                )
            })
            .collect::<Vec<_>>();
        let values = self.graph.array("NSArray", &values);
        self.graph.set(data, "NSAccessibilityOidsValues", values);
        let root = self.graph.object("NSObject");
        self.graph.set(root, "IB.objectdata", data);
        self.graph.set(root, "IB.systemFontUpdateVersion", 1_i64);
        self.graph.write(root)
    }

    fn declare(&mut self, element: &Element, top: bool) -> Result<(), String> {
        if let Some(id) = element.attr("id")
            && id != "-1"
        {
            let class = match element.name.as_str() {
                "customObject" if matches!(id, "-2" | "-3") => "NSCustomObject",
                "application" => "NSCustomObject",
                "windowController" if element.attr("customClass").is_none() => "NSWindowController",
                "viewController"
                    if element.attr("customClass").is_none() && yes(element, "ibtoolEmbedded") =>
                {
                    "NSViewController"
                }
                "splitViewController"
                    if element.attr("customClass").is_none() && yes(element, "ibtoolEmbedded") =>
                {
                    "NSSplitViewController"
                }
                "splitViewItem" => "NSSplitViewItem",
                "splitView" => "NSSplitView",
                "storyboardPlaceholder" => "NSNibExternalObjectPlaceholder",
                "storyboardShowSegue" => "NSStoryboardShowSegueTemplate",
                "storyboardEmbedSegue" => "NSStoryboardEmbedSegueTemplate",
                "customView" if !top && !self.storyboard && placeholder_view(element) => {
                    "NSCustomView"
                }
                "customObject"
                | "customView"
                | "viewController"
                | "windowController"
                | "splitViewController" => "NSClassSwapper",
                "view" | "containerView" => "NSView",
                "imageView" => "NSImageView",
                "textField" => "NSTextField",
                "textView" => "NSTextView",
                "tableView" => "NSTableView",
                "outlineView" => "NSOutlineView",
                "tableCellView" => "NSTableCellView",
                "box" => "NSBox",
                "collectionView" => "NSCollectionView",
                "collectionViewFlowLayout" => "NSCollectionViewFlowLayout",
                "tableColumn" => "NSTableColumn",
                "tableHeaderCell" => "NSTableHeaderCell",
                "secureTextField" => "NSSecureTextField",
                "textFieldCell" => "NSTextFieldCell",
                "secureTextFieldCell" => "NSSecureTextFieldCell",
                "button" => "NSButton",
                "buttonCell" => "NSButtonCell",
                "popUpButton" => "NSPopUpButton",
                "popUpButtonCell" => "NSPopUpButtonCell",
                "menu" => "NSMenu",
                "menuItem" => "NSMenuItem",
                "progressIndicator" => "NSProgressIndicator",
                "scrollView" => "NSScrollView",
                "clipView" => "NSClipView",
                "scroller" => "NSScroller",
                "stackView" => "NSStackView",
                "wkWebView" => "WKWebView",
                "gridView" => "NSGridView",
                "gridRow" => "NSGridRow",
                "gridColumn" => "NSGridColumn",
                "gridCell" => "NSGridCell",
                "window" => "NSWindowTemplate",
                "userDefaultsController" => "NSUserDefaultsController",
                "imageCell" => "NSImageCell",
                "constraint" => "NSLayoutConstraint",
                "outlet" | "action" | "binding" => return Ok(()),
                other => return Err(format!("unsupported AppKit object <{other}>")),
            };
            let class = if element.attr("customClass").is_some()
                && matches!(
                    element.name.as_str(),
                    "view"
                        | "imageView"
                        | "textField"
                        | "textView"
                        | "tableView"
                        | "outlineView"
                        | "tableCellView"
                        | "collectionView"
                        | "button"
                        | "popUpButton"
                        | "progressIndicator"
                ) {
                "NSClassSwapper"
            } else {
                class
            };
            let object = self.graph.object(class);
            self.ids.insert(id.to_string(), object);
        }
        for child in &element.children {
            self.declare(child, element.name == "objects")?;
        }
        Ok(())
    }

    fn object(&self, element: &Element) -> Result<Id, String> {
        self.lookup(id(element)?)
    }

    fn lookup(&self, id: &str) -> Result<Id, String> {
        self.ids
            .get(id)
            .copied()
            .ok_or_else(|| format!("missing AppKit object {id}"))
    }

    fn string(&mut self, object: Id, key: &str, value: &str) {
        let value = self.graph.string(value);
        self.graph.set(object, key, value);
    }

    fn text(&mut self, element: &Element, property: &str, value: &str) -> Id {
        if !self.options.localized || value.is_empty() {
            return self.graph.string(value);
        }
        let key = self.graph.string(&format!(
            "{}.{}",
            element.attr("id").unwrap_or(""),
            property
        ));
        let development = self.graph.string(value);
        self.graph.shared(
            "NSLocalizableString",
            vec![
                ("NS.bytes", Value::Data(value.as_bytes().to_vec())),
                ("NSDev", development.into()),
                ("NSKey", key.into()),
            ],
        )
    }

    fn image(&mut self, name: &str) -> Id {
        if let Some(image) = self.image_ids.get(name) {
            return *image;
        }
        let resource_name = name.to_string();
        // AppKit resolves built-in image names before consulting design-time dimensions.
        let [width, height] = match name {
            "NSAddTemplate" | "NSMenuCheckmark" => [18.0, 16.0],
            "NSMenuMixedState" | "NSRemoveTemplate" => [18.0, 4.0],
            "NSActionTemplate" => [19.0, 19.0],
            _ => self.images.get(name).copied().unwrap_or([0.0, 0.0]),
        };
        let size = self.graph.string(&format!("{{{width}, {height}}}"));
        let size = self.graph.object_with_values(
            "NSValue",
            vec![("NS.sizeval", size.into()), ("NS.special", Value::Int(2))],
        );
        let namespace =
            if name.starts_with("NS") && !matches!(name, "NSMenuCheckmark" | "NSMenuMixedState") {
                Value::Object(self.graph.string("system"))
            } else {
                Value::Nil
            };
        let class = self.graph.string("NSImage");
        let name = self.graph.string(name);
        let image = self.graph.shared(
            "NSCustomResource",
            vec![
                ("NSClassName", class.into()),
                ("NSResourceName", name.into()),
                ("IBDesignSize", size.into()),
                ("IBNamespaceID", namespace),
                ("IBDesignImageConfiguration", Value::Nil),
            ],
        );
        self.image_ids.insert(resource_name, image);
        image
    }

    fn encode(&mut self, element: &Element, parent: Id) -> Result<Id, String> {
        if element.name == "menu" {
            return self.menu(element, parent, None, false);
        }
        let object = self.object(element)?;
        self.owners.push((object, parent));
        self.oids.push(object);
        if element.name == "customObject" {
            if element.attr("id") == Some("-3") {
                if element.attr("customClass").is_some() {
                    self.class_reference(element, object);
                }
                self.string(object, "NSClassName", "NSApplication");
            } else {
                self.string(
                    object,
                    "NSClassName",
                    &custom_class(element, self.options).unwrap_or_else(|| "NSObject".into()),
                );
                self.string(object, "NSOriginalClassName", "NSObject");
                self.graph.set(object, "NSInitializeWithInit", true);
            }
        } else if element.name == "viewController" {
            self.storyboard_view_controller(element, object, parent)?;
        } else if element.name == "splitViewController" {
            self.split_controller(element, object, parent)?;
        } else if element.name == "splitViewItem" {
            self.split_item(element, object)?;
        } else if element.name == "windowController" {
            self.storyboard_window_controller(element, object)?;
        } else if matches!(
            element.name.as_str(),
            "storyboardShowSegue" | "storyboardEmbedSegue"
        ) {
            self.storyboard_segue_template(element, object)?;
        } else if element.name == "storyboardPlaceholder" {
            self.storyboard_placeholder(element, object)?;
        } else if element.name == "application" {
            self.string(object, "NSClassName", "NSApplication");
            if let Some(menu) = element.child("menu") {
                let menu = self.menu(menu, parent, None, false)?;
                self.graph.set(object, "IBMainMenu", menu);
                self.main_menu_items(menu);
            }
        } else if element.name == "userDefaultsController" {
            if yes(element, "representsSharedInstance") {
                self.graph.set(object, "NSSharedInstance", true);
            } else if !no(element, "appliesImmediately") {
                self.graph.set(object, "NSAppliesImmediately", true);
            }
        } else if element.name == "window" {
            self.window(element, object)?;
        } else {
            self.view(element, object, parent)?;
        }
        Ok(object)
    }

    fn class_reference(&mut self, element: &Element, object: Id) {
        let reference = self.graph.object("IBClassReference");
        self.string(
            reference,
            "IBClassName",
            element.attr("customClass").unwrap_or("NSObject"),
        );
        for (xml, key) in [
            ("customModule", "IBModuleName"),
            ("customModuleProvider", "IBModuleProvider"),
        ] {
            let value = if xml == "customModule"
                && element.attr("customModuleProvider") == Some("target")
            {
                Some(self.options.module)
            } else {
                element.attr(xml)
            };
            if let Some(value) = value {
                self.string(reference, key, value);
            } else {
                self.graph.set(reference, key, Value::Nil);
            }
        }
        self.graph.set(object, "IBClassReference", reference);
    }

    fn connections_for(
        &mut self,
        element: &Element,
        bindings_only: bool,
    ) -> Result<Vec<Id>, String> {
        let mut result = Vec::new();
        if !bindings_only
            && let Some(tooltip) = string_value(element, "toolTip")?.filter(|s| !s.is_empty())
        {
            let connector = self.graph.object("NSIBHelpConnector");
            let destination = self.object(element)?;
            self.graph.set(connector, "NSDestination", destination);
            self.string(connector, "NSFile", "NSToolTipHelpKey");
            let marker = self.text(element, "toolTip", &tooltip);
            self.graph.set(connector, "NSMarker", marker);
            self.oids.push(connector);
            result.push(connector);
        }
        if let Some(connections) = element.child("connections") {
            for connection in &connections.children {
                if (connection.name == "binding") != bindings_only {
                    continue;
                }
                if connection.name == "binding" {
                    let binding = self.binding(element, connection)?;
                    self.oids.push(binding);
                    result.push(binding);
                    continue;
                }
                let (class, target_key, label_key) = match connection.name.as_str() {
                    "outlet" => ("NSNibOutletConnector", "destination", "property"),
                    "action" => ("NSNibControlConnector", "target", "selector"),
                    other => return Err(format!("unsupported AppKit connection <{other}>")),
                };
                let object = self.graph.object(class);
                let source = self.object(element)?;
                let target = connection
                    .attr(target_key)
                    .ok_or("a connection has no destination")?;
                let destination = if target == "-1" {
                    Value::Nil
                } else {
                    self.lookup(target)?.into()
                };
                self.graph.set(object, "NSSource", source);
                if target != "-1" {
                    self.graph.set(object, "NSDestination", destination);
                }
                self.string(
                    object,
                    "NSLabel",
                    connection
                        .attr(label_key)
                        .ok_or("a connection has no label")?,
                );
                if connection.name == "outlet" && !yes(connection, "ibtoolRelationship") {
                    self.graph
                        .set(object, "NSChildControllerCreationSelectorName", Value::Nil);
                }
                self.oids.push(object);
                result.push(object);
            }
        }
        for child in &element.children {
            if self.storyboard && child.name == "prototypeCellViews" {
                continue;
            }
            result.extend(self.connections_for(child, bindings_only)?);
        }
        Ok(result)
    }
}
