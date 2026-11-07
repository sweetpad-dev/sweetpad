//! The UIKit objects an Interface Builder document describes, archived the
//! way their classes' `encodeWithCoder:` archives them: views, labels,
//! image views, Auto Layout constraints and the safe area, with their
//! colors and fonts.
//!
//! An element or attribute this doesn't know is an error naming it, never
//! a nib that leaves it out.

use std::collections::{HashMap, HashSet};

use sweetpad_lib::xcscheme::Element;

use super::nib::{Graph, Id, Value, doubles};
use super::{Options, custom_class};
mod accessibility;
mod calibrated;
mod configuration;
mod controls;
mod effects;
mod fonts;
mod framework_views;
pub(super) mod gestures;
mod images;
mod layout_guides;
mod lists;
mod more_views;
mod navigation;
mod prototype;
mod traits;

/// Encodes one nib's objects, from the document they're in.
pub(super) struct Encoder<'a> {
    pub graph: Graph,
    pub frame_overrides: HashMap<String, [f64; 4]>,
    document: &'a Element,
    pub static_sources: Vec<Id>,
    collection_contents: HashSet<*const Element>,
    special_classes: HashMap<*const Element, &'static str>,
    constrained: HashSet<String>,
    /// The fallback values of the system colors the document uses.
    system_colors: HashMap<&'a str, &'a Element>,
    named_colors: HashMap<&'a str, &'a Element>,
    module: String,
    localized: bool,
    /// The archived object of each element, by its Interface Builder id.
    ids: HashMap<String, Id>,
    elements: HashMap<*const Element, Id>,
    /// The view each safe area layout guide belongs to, by the guide's id.
    guide_owners: HashMap<String, String>,
    /// The `UIViewSafeAreaLayoutGuide` of each view a constraint pins to
    /// its safe area, by the view's id.
    safe_areas: HashMap<String, Id>,
    scroll_guide_owners: HashMap<String, (String, bool)>,
    legacy_guide_definitions: HashMap<String, Vec<(String, bool)>>,
    legacy_guides: HashMap<String, (Vec<Id>, Vec<Id>)>,
    table_labels: HashSet<String>,
    /// What the nib's objects list holds besides its placeholders: each
    /// view, its layout guides and constraints, then its subviews.
    pub objects: Vec<Id>,
    pub key_value_pairs: Vec<Id>,
    pub accessibility_configurations: Vec<Id>,
    outlet_collections: HashMap<(Id, String), Id>,
    trait_storages: Vec<(Id, Id, Id)>,
    trait_descendants: Vec<(Id, Id)>,
    current_trait_root: Option<Id>,
}

/// The value `key` of an element's `rect`, `color` or other child.
fn keyed<'e>(element: &'e Element, name: &'e str, key: &str) -> Option<&'e Element> {
    element
        .children_named(name)
        .find(|child| child.attr("key") == Some(key))
}

fn yes(element: &Element, attribute: &str) -> bool {
    element.attr(attribute) == Some("YES")
}

fn no(element: &Element, attribute: &str) -> bool {
    element.attr(attribute) == Some("NO")
}

fn number(element: &Element, attribute: &str) -> Result<Option<f64>, String> {
    element
        .attr(attribute)
        .map(|text| {
            text.parse::<f64>()
                .map_err(|_| format!("{attribute}=\"{text}\" isn't a number"))
        })
        .transpose()
}

// IB accepts decimal spellings of whole numbers. The fallback checks the range
// and fractional part before conversion; integer spellings retain all 64 bits.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn integer(element: &Element, attribute: &str) -> Result<Option<i64>, String> {
    element
        .attr(attribute)
        .map(|text| {
            text.parse::<i64>().or_else(|_| {
                text.parse::<f64>()
                    .ok()
                    .filter(|n| {
                        n.is_finite()
                            && n.fract() == 0.0
                            && *n >= i64::MIN as f64
                            && *n < -(i64::MIN as f64)
                    })
                    .map(|n| n as i64)
                    .ok_or_else(|| format!("{attribute}=\"{text}\" isn't a whole number"))
            })
        })
        .transpose()
}

fn id_of(element: &Element) -> Result<&str, String> {
    element
        .attr("id")
        .ok_or_else(|| format!("a <{}> has no id", element.name))
}

/// The UIKit class of a view element.
fn view_class(element: &Element) -> Result<&'static str, String> {
    match element.name.as_str() {
        "view" | "containerView" | "collectionViewCellContentView" => Ok("UIView"),
        "label" => Ok("UILabel"),
        "imageView" => Ok("UIImageView"),
        "button" => Ok("UIButton"),
        "textField" => Ok("UITextField"),
        "switch" => Ok("UISwitch"),
        "scrollView" => Ok("UIScrollView"),
        "stackView" => Ok("UIStackView"),
        "tableViewCell" => Ok("UITableViewCell"),
        "tableViewCellContentView" => Ok("UITableViewCellContentView"),
        "navigationBar" => Ok("UINavigationBar"),
        "tableView" => Ok("UITableView"),
        "collectionView" => Ok("UICollectionView"),
        "collectionViewCell" => Ok("UICollectionViewCell"),
        "collectionReusableView" => Ok("UICollectionReusableView"),
        "pickerView" => Ok("UIPickerView"),
        "slider" => Ok("UISlider"),
        "activityIndicatorView" => Ok("UIActivityIndicatorView"),
        "visualEffectView" => Ok("UIVisualEffectView"),
        "refreshControl" => Ok("UIRefreshControl"),
        "progressView" => Ok("UIProgressView"),
        "stepper" => Ok("UIStepper"),
        "pageControl" => Ok("UIPageControl"),
        "segmentedControl" => Ok("UISegmentedControl"),
        "textView" => Ok("UITextView"),
        "searchBar" => Ok("UISearchBar"),
        "toolbar" => Ok("UIToolbar"),
        "tabBar" => Ok("UITabBar"),
        "wkWebView" => Ok("WKWebView"),
        "mapView" => Ok("MKMapView"),
        other => gestures::class(other)
            .ok_or_else(|| format!("<{other}> isn't a view SweetPad's ibtool compiles yet")),
    }
}

/// The view elements of `subviews`, in order.
fn subviews(element: &Element) -> impl Iterator<Item = &Element> {
    element
        .children_named("subviews")
        .flat_map(|subviews| subviews.children.iter())
        .chain(element.children_named("tableViewCellContentView"))
        .chain(element.children_named("collectionViewCellContentView"))
        .chain(
            element
                .children_named("view")
                .filter(|v| v.attr("key") == Some("contentView")),
        )
}

/// The elements a view may have that encode nothing: Interface Builder's
/// own bookkeeping.
const IGNORED: &[&str] = &[
    "point",
    "nil",
    "accessibility",
    "freeformSimulatedSizeMetrics",
    "simulatedScreenMetrics",
];

impl<'a> Encoder<'a> {
    fn large_content_metadata(&mut self, object: Id) {
        if self.document.attr("targetRuntime") != Some("AppleTV") {
            self.graph
                .set(object, "UIViewLargeContentStoredProperties", Value::Nil);
        }
    }

    pub fn lookup(&self, id: &str) -> Option<Id> {
        self.ids.get(id).copied()
    }

    pub fn unused_safe_area(&mut self) {
        let guide = self.graph.object("UILayoutGuide");
        self.graph.set(guide, "UILayoutGuideOwningView", Value::Nil);
        let empty = self.graph.string("");
        self.graph.set(guide, "UILayoutGuideIdentifier", empty);
        self.graph
            .set(guide, "UILayoutGuideOwningViewIsLocked", false);
        self.graph
            .set(guide, "UILayoutGuideShouldBeArchived", false);
        self.graph
            .set(guide, "UILayoutGuideAllowsNegativeDimensions", false);
        self.objects.push(guide);
    }

    pub fn object_for(&self, element: &Element) -> Result<Id, String> {
        self.elements
            .get(&std::ptr::from_ref(element))
            .copied()
            .ok_or_else(|| format!("<{}> wasn't declared", element.name))
    }

    pub fn bind(&mut self, id: &str, object: Id) {
        self.ids.insert(id.to_string(), object);
    }

    pub fn connections(&mut self, element: &Element) -> Result<Vec<Id>, String> {
        self.connections_in_graph(element, true)
    }

    pub fn connections_in_graph(
        &mut self,
        element: &Element,
        strict: bool,
    ) -> Result<Vec<Id>, String> {
        let mut connections = Vec::new();
        if let Some(list) = element.child("connections") {
            let source = self.ids.get(id_of(element)?).copied();
            if source.is_none() && strict {
                return Err("a connection's source isn't in this nib".to_string());
            }
            let mut seen = HashSet::new();
            for connection in &list.children {
                let Some(source) = source else {
                    continue;
                };
                if connection.name == "segue" {
                    if element.name.ends_with("Controller") {
                        continue;
                    }
                    if connection.attr("kind") == Some("relationship") {
                        continue;
                    }
                    if let Some(template) = connection.attr("id").and_then(|id| self.lookup(id)) {
                        self.segue_connection(
                            element,
                            connection,
                            source,
                            template,
                            &mut connections,
                        )?;
                    } else if strict {
                        return Err("a segue template is missing".to_string());
                    }
                    continue;
                }
                if !matches!(
                    connection.name.as_str(),
                    "outlet" | "outletCollection" | "action"
                ) {
                    return Err(format!(
                        "<{}> connections aren't supported yet",
                        connection.name
                    ));
                }
                let destination = connection
                    .attr("destination")
                    .ok_or("an outlet has no destination")?;
                let Some(destination) = self.ids.get(destination).copied() else {
                    if strict {
                        return Err(format!(
                            "the connection destination {destination} isn't in this nib"
                        ));
                    }
                    continue;
                };
                let identity = format!(
                    "{} {source:?} {destination:?} {:?} {:?} {:?}",
                    connection.name,
                    connection.attr("property"),
                    connection.attr("selector"),
                    connection.attr("eventType")
                );
                if !seen.insert(identity) {
                    continue;
                }
                if connection.name == "outletCollection" {
                    let property = connection
                        .attr("property")
                        .ok_or("an outlet collection has no property")?;
                    let collection = connection.attr("collectionClass").unwrap_or("NSArray");
                    self.outlet_collection(
                        &mut connections,
                        source,
                        destination,
                        property,
                        collection,
                        yes(connection, "appends"),
                    );
                } else if connection.name == "outlet" {
                    let property = connection
                        .attr("property")
                        .ok_or("an outlet has no property")?;
                    connections.push(super::storyboard::outlet(
                        &mut self.graph,
                        property,
                        source,
                        destination,
                    ));
                } else {
                    connections.push(self.event_connection(connection, source, destination)?);
                }
            }
        }
        for child in &element.children {
            if child.name != "connections"
                && child.name != "prototypes"
                && !(element.name == "collectionView" && child.name == "cells")
            {
                connections.extend(self.connections_in_graph(child, strict)?);
            }
        }
        Ok(connections)
    }

    fn segue_connection(
        &mut self,
        element: &Element,
        segue: &Element,
        source: Id,
        template: Id,
        connections: &mut Vec<Id>,
    ) -> Result<(), String> {
        if segue.attr("kind") == Some("embed") {
            connections.push(super::storyboard::outlet(
                &mut self.graph,
                "containerView",
                template,
                source,
            ));
        } else if matches!(
            element.name.as_str(),
            "tableViewCell" | "collectionViewCell"
        ) {
            connections.push(super::storyboard::outlet(
                &mut self.graph,
                "selectionSegueTemplate",
                source,
                template,
            ));
        } else {
            let mask = match element.name.as_str() {
                "button" => Some(64_i64),
                "barButtonItem" => None,
                "switch" | "slider" | "segmentedControl" | "stepper" | "pageControl"
                | "datePicker" => Some(4096),
                other => return Err(format!("unsupported segue trigger <{other}>")),
            };
            let connection = self.graph.object("UIRuntimeEventConnection");
            let action = self.graph.string("perform:");
            self.graph.set(connection, "UILabel", action);
            self.graph.set(connection, "UISource", source);
            self.graph.set(connection, "UIDestination", template);
            if let Some(mask) = mask {
                self.graph.set(connection, "UIEventMask", mask);
            }
            connections.push(connection);
        }
        Ok(())
    }

    fn event_connection(
        &mut self,
        connection: &Element,
        source: Id,
        destination: Id,
    ) -> Result<Id, String> {
        let selector = connection
            .attr("selector")
            .ok_or("an action has no selector")?;
        let event = event_mask(connection)?;
        let object = self.graph.object("UIRuntimeEventConnection");
        let selector = self.graph.string(selector);
        self.graph.set(object, "UILabel", selector);
        self.graph.set(object, "UISource", source);
        self.graph.set(object, "UIDestination", destination);
        if event != 0 {
            self.graph.set(object, "UIEventMask", event);
        }
        Ok(object)
    }

    pub fn new(document: &'a Element, options: &Options<'_>) -> Self {
        let system_colors = document
            .children_named("resources")
            .flat_map(|resources| resources.children_named("systemColor"))
            .filter_map(|color| Some((color.attr("name")?, color.child("color")?)))
            .collect();
        let mut constrained = HashSet::new();
        collect_constraints(document, &mut constrained);
        Encoder {
            constrained,
            graph: Graph::default(),
            frame_overrides: HashMap::new(),
            document,
            static_sources: Vec::new(),
            collection_contents: HashSet::new(),
            special_classes: HashMap::new(),
            system_colors,
            named_colors: document
                .children_named("resources")
                .flat_map(|r| r.children_named("namedColor"))
                .filter_map(|c| Some((c.attr("name")?, c.child("color")?)))
                .collect(),
            module: options.module.to_string(),
            localized: options.localized,
            ids: HashMap::new(),
            elements: HashMap::new(),
            guide_owners: HashMap::new(),
            safe_areas: HashMap::new(),
            scroll_guide_owners: HashMap::new(),
            legacy_guide_definitions: HashMap::new(),
            legacy_guides: HashMap::new(),
            table_labels: HashSet::new(),
            objects: Vec::new(),
            key_value_pairs: Vec::new(),
            accessibility_configurations: Vec::new(),
            outlet_collections: HashMap::new(),
            trait_storages: Vec::new(),
            trait_descendants: Vec::new(),
            current_trait_root: None,
        }
    }

    /// The archived view `element` and everything below it.
    pub fn root_view(&mut self, element: &Element) -> Result<Id, String> {
        self.declare(element)?;
        let view = self.view(element)?;
        if gestures::class(&element.name).is_none() {
            self.document_tint(view, element)?;
        }
        Ok(view)
    }

    /// Gives each view below `element` its object, so a constraint can
    /// refer to a view it comes before.
    pub fn declare(&mut self, element: &Element) -> Result<(), String> {
        if element.name == "tableViewCell" {
            for key in ["textLabel", "detailTextLabel"] {
                if let Some(id) = element.attr(key) {
                    self.table_labels.insert(id.to_string());
                }
            }
        }

        if element.name == "collectionViewCell"
            && let Some(content) = subviews(element).find(|e| e.attr("key") == Some("contentView"))
        {
            self.collection_contents.insert(std::ptr::from_ref(content));
        }
        if element.name == "visualEffectView"
            && let Some(content) = subviews(element).find(|e| e.attr("key") == Some("contentView"))
        {
            self.special_classes
                .insert(std::ptr::from_ref(content), "_UIVisualEffectContentView");
        }
        let class = if let Some(&class) = self.special_classes.get(&std::ptr::from_ref(element)) {
            class
        } else if element.name == "label"
            && element
                .attr("id")
                .is_some_and(|id| self.table_labels.contains(id))
        {
            "UITableViewLabel"
        } else {
            view_class(element)?
        };
        let id = element.attr("id").unwrap_or("<anonymous>").to_string();
        for guide in element.children_named("viewLayoutGuide") {
            if guide.attr("key") == Some("safeArea") {
                self.guide_owners
                    .insert(id_of(guide)?.to_string(), id.clone());
            } else if matches!(
                guide.attr("key"),
                Some("contentLayoutGuide" | "frameLayoutGuide")
            ) {
                self.scroll_guide_owners.insert(
                    id_of(guide)?.to_string(),
                    (id.clone(), guide.attr("key") == Some("contentLayoutGuide")),
                );
            }
        }
        let object = self.graph.object(if element.attr("customClass").is_some() {
            "UIClassSwapper"
        } else {
            class
        });
        self.ids.insert(id.clone(), object);
        self.install_legacy_guides(&id, object);
        self.elements.insert(std::ptr::from_ref(element), object);
        for subview in subviews(element) {
            if element.name == "stackView"
                && let Some(id) = subview.attr("id")
            {
                self.constrained.insert(id.to_string());
            }
            self.declare(subview)?;
        }
        for header in element
            .children
            .iter()
            .filter(|e| matches!(e.attr("key"), Some("tableHeaderView" | "tableFooterView")))
        {
            self.declare(header)?;
        }
        for section in element
            .children_named("sections")
            .flat_map(|sections| sections.children_named("tableViewSection"))
        {
            for cell in section
                .children_named("cells")
                .flat_map(|cells| cells.children.iter())
            {
                self.declare(cell)?;
            }
        }
        Ok(())
    }

    fn validate_view(element: &Element) -> Result<(), String> {
        let id = element.attr("id").unwrap_or("<anonymous>");
        for child in &element.children {
            match child.name.as_str() {
                "gestureRecognizers" if child.children.is_empty() => {}
                "rect"
                | "autoresizingMask"
                | "subviews"
                | "color"
                | "constraints"
                | "viewLayoutGuide"
                | "fontDescription"
                | "string"
                | "connections"
                | "state"
                | "textInputTraits"
                | "dataDetectorType"
                | "edgeInsets"
                | "directionalEdgeInsets"
                | "inset"
                | "tableViewCellContentView"
                | "collectionViewFlowLayout"
                | "sections"
                | "prototypes"
                | "cells"
                | "view"
                | "collectionViewCellContentView"
                | "size"
                | "blurEffect"
                | "buttonConfiguration"
                | "preferredSymbolConfiguration"
                | "imageReference"
                | "collectionReusableView"
                | "userDefinedRuntimeAttributes"
                | "variation"
                | "segments"
                | "items"
                | "wkWebViewConfiguration" => {}
                name if IGNORED.contains(&name) => {}
                name => {
                    return Err(format!(
                        "<{name}> in <{}> {id} isn't something SweetPad's ibtool compiles yet",
                        element.name
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn view(&mut self, element: &Element) -> Result<Id, String> {
        let object = self.object_for(element)?;
        if gestures::class(&element.name).is_some() {
            self.gesture(element, object)?;
            self.objects.push(object);
            return Ok(object);
        }
        self.ui_view(element)
    }

    fn ui_view(&mut self, element: &Element) -> Result<Id, String> {
        let id = element.attr("id").unwrap_or("<anonymous>");
        let object = self.object_for(element)?;
        let previous_trait_root = self.current_trait_root;
        self.current_trait_root.get_or_insert(object);
        if !self
            .collection_contents
            .contains(&std::ptr::from_ref(element))
            || (element.name == "collectionViewCellContentView"
                && element
                    .child("constraints")
                    .is_some_and(|c| !c.children.is_empty()))
        {
            self.objects.push(object);
        }
        Self::validate_view(element)?;
        let [x, y, width, height] = self.view_geometry(element, object)?;
        let mut constraints = self.constraint_objects(element)?;
        if let Some((_, legacy)) = self.legacy_guides.get(id) {
            constraints.extend(legacy);
        }
        let children: Vec<&Element> = subviews(element).collect();
        self.encode_subviews(element, object, &children, width, height)?;
        if let Some(color) = keyed(element, "color", "backgroundColor") {
            let color = self.color(color)?;
            self.graph.set(object, "UIBackgroundColor", color);
        }
        self.appearance(element, object)?;
        if !constraints.is_empty() {
            let array = self.graph.array("NSMutableArray", &constraints);
            self.graph.set(object, "UIViewAutolayoutConstraints", array);
        }
        if let Some(&guide) = self.safe_areas.get(id) {
            let array = self.graph.array("NSMutableArray", &[guide]);
            self.graph.set(object, "UIViewLayoutGuides", array);
        }
        self.graph.set(
            object,
            "UIViewSemanticContentAttribute",
            semantic_content(element)?,
        );
        self.large_content_metadata(object);
        if let Some(color) = keyed(element, "color", "tintColor") {
            let color = self.color(color)?;
            self.graph.set(object, "UITintColor", color);
        }

        match element.name.as_str() {
            "label" => {
                self.label(element, object)?;
                if let Some(width) = number(element, "preferredMaxLayoutWidth")? {
                    self.graph
                        .set(object, "UIPreferredMaxLayoutWidth", single(width));
                }
                if yes(element, "adjustsLetterSpacingToFitWidth") {
                    self.graph.set(object, "UIAdjustsLetterSpacingToFit", true);
                }
            }
            "imageView" => self.image_view(element, object)?,
            "button" => self.button(element, object)?,
            "textField" => self.text_field(element, object)?,
            "switch" => self.switch(element, object, [x, y, width, height])?,
            "scrollView" => self.scroll_view(element, object),
            "stackView" => self.stack_view(element, object, &children)?,
            "tableViewCell" => self.table_cell(element, object)?,
            "navigationBar" => self.navigation_bar(element, object),
            "tableView" => self.table_view(element, object)?,
            "collectionView" => self.collection_view(element, object)?,
            "collectionViewCell" | "collectionReusableView" => {
                self.collection_cell(element, object)?;
            }
            "pickerView"
            | "slider"
            | "activityIndicatorView"
            | "visualEffectView"
            | "refreshControl" => self.additional_control(element, object)?,
            "progressView" | "stepper" | "pageControl" | "segmentedControl" | "textView"
            | "searchBar" | "toolbar" | "tabBar" => self.more_view(element, object)?,
            "wkWebView" | "mapView" => self.framework_view(element, object)?,
            _ => {}
        }
        for child in children {
            self.view(child)?;
        }
        self.view_custom_class(element, object)?;
        self.runtime_attributes(element, object)?;
        self.accessibility(element, object)?;
        self.trait_variations(element, object)?;
        self.current_trait_root = previous_trait_root;
        Ok(object)
    }

    fn encode_subviews(
        &mut self,
        element: &Element,
        object: Id,
        children: &[&Element],
        width: f64,
        height: f64,
    ) -> Result<(), String> {
        let id = element.attr("id").unwrap_or("<anonymous>");
        let headers: Vec<&Element> = element
            .children
            .iter()
            .filter(|e| matches!(e.attr("key"), Some("tableHeaderView" | "tableFooterView")))
            .collect();
        if (!children.is_empty() || !headers.is_empty() || self.legacy_guides.contains_key(id))
            && element.name != "visualEffectView"
        {
            let mut views = Vec::new();
            for child in children.iter().chain(headers.iter()) {
                views.push(self.object_for(child)?);
            }
            if let Some((guides, _)) = self.legacy_guides.get(id) {
                views.extend(guides);
            }
            if element.name == "tableViewCell" {
                views.push(self.cell_separator(element, width, height)?);
            }
            let array = self.graph.array("NSMutableArray", &views);
            self.graph.set(object, "UISubviews", array);
        }
        Ok(())
    }

    fn view_custom_class(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(class) = custom_class(
            element,
            &Options {
                module: &self.module,
                localized: self.localized,
            },
        ) {
            let class = self.graph.string(&class);
            let original = self.graph.string(view_class(element)?);
            self.graph.set(object, "UIClassName", class);
            self.graph.set(object, "UIOriginalClassName", original);
        }
        Ok(())
    }

    fn view_geometry(&mut self, element: &Element, object: Id) -> Result<[f64; 4], String> {
        let id = element.attr("id").unwrap_or("<anonymous>");
        for guide in element.children_named("viewLayoutGuide") {
            if !matches!(
                guide.attr("key"),
                Some("safeArea" | "contentLayoutGuide" | "frameLayoutGuide")
            ) {
                return Err(format!(
                    "the {} layout guide of {id} isn't something SweetPad's ibtool compiles yet",
                    guide.attr("key").unwrap_or("unnamed")
                ));
            }
            let placeholder = self.graph.object("UILayoutGuide");
            self.graph
                .set(placeholder, "UILayoutGuideOwningView", Value::Nil);
            let empty = self.graph.string("");
            self.graph
                .set(placeholder, "UILayoutGuideIdentifier", empty);
            self.graph
                .set(placeholder, "UILayoutGuideOwningViewIsLocked", false);
            self.graph
                .set(placeholder, "UILayoutGuideShouldBeArchived", false);
            self.graph
                .set(placeholder, "UILayoutGuideAllowsNegativeDimensions", false);
            self.objects.push(placeholder);
        }

        // Older documents can omit the frame. ibtool's default canvas is
        // 1000 by 1000; UIKit applies the document's constraints on load.
        let (x, y, width, height) = if let Some(frame) = keyed(element, "rect", "frame") {
            (
                number(frame, "x")?.unwrap_or(0.0),
                number(frame, "y")?.unwrap_or(0.0),
                number(frame, "width")?.unwrap_or(0.0),
                number(frame, "height")?.unwrap_or(0.0),
            )
        } else {
            (0.0, 0.0, 1000.0, 1000.0)
        };
        let (width, height) = if width == 0.0
            && height == 0.0
            && !no(element, "translatesAutoresizingMaskIntoConstraints")
        {
            (1000.0, 1000.0)
        } else {
            (width, height)
        };
        let y = if matches!(
            element.attr("key"),
            Some("tableHeaderView" | "tableFooterView")
        ) {
            53.0
        } else {
            y
        };
        let [x, y, width, height] = self
            .frame_overrides
            .get(id)
            .copied()
            .unwrap_or([x, y, width, height]);
        let (x, y, width, height) = if element.name == "navigationBar" {
            (x, y, 0.0, 54.0)
        } else {
            (x, y, width, height)
        };
        // Bar-style progress views reserve one point and keep a three-point
        // minimum in the design archive, measured at several source heights.
        let height =
            if element.name == "progressView" && element.attr("progressViewStyle") == Some("bar") {
                (height - 1.0).max(3.0)
            } else {
                height
            };
        if width != 0.0 || height != 0.0 {
            self.graph
                .set(object, "UIBounds", doubles(&[0.0, 0.0, width, height]));
        }
        self.graph.set(
            object,
            "UICenter",
            doubles(&[x + width / 2.0, y + height / 2.0]),
        );

        Ok([x, y, width, height])
    }

    /// How a view draws and takes part in layout: the flags and priorities
    /// it has in the document.
    fn layout_margins(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if element.name == "navigationBar" && self.document.attr("targetRuntime") != Some("AppleTV")
        {
            self.graph
                .set(object, "UIViewLayoutMargins", doubles(&[0.0; 4]));
            self.graph
                .set(object, "UIViewLayoutMarginsAreDirectional", true);
        }
        if let Some(margins) = keyed(element, "directionalEdgeInsets", "directionalLayoutMargins") {
            let mut values = Vec::new();
            for key in ["top", "leading", "bottom", "trailing"] {
                values.push(number(margins, key)?.unwrap_or(0.0));
            }
            self.graph
                .set(object, "UIViewLayoutMargins", doubles(&values));
            self.graph
                .set(object, "UIViewLayoutMarginsAreDirectional", true);
        } else if let Some(margins) = keyed(element, "edgeInsets", "layoutMargins") {
            let mut values = Vec::new();
            for key in ["top", "left", "bottom", "right"] {
                values.push(number(margins, key)?.unwrap_or(0.0));
            }
            self.graph
                .set(object, "UIViewLayoutMargins", doubles(&values));
        } else if matches!(element.name.as_str(), "stackView" | "mapView") {
            self.graph
                .set(object, "UIViewLayoutMargins", doubles(&[0.0; 4]));
        }
        Ok(())
    }

    fn appearance(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.graph.set(object, "UIDeepDrawRect", true);
        if yes(element, "pointerInteractionEnabled") || yes(element, "pointerInteraction") {
            self.graph.set(object, "UIPointerInteractionEnabled", true);
        }
        if yes(element, "hidden") {
            self.graph.set(object, "UIHidden", true);
        }
        if yes(element, "multipleTouchEnabled") {
            self.graph.set(object, "UIMultipleTouchEnabled", true);
        }
        if !no(element, "opaque")
            && (!matches!(element.name.as_str(), "navigationBar" | "searchBar")
                || yes(element, "opaque"))
            && (element.child("imageReference").is_none() || yes(element, "opaque"))
        {
            self.graph.set(object, "UIOpaque", true);
        }
        if no(element, "userInteractionEnabled") {
            self.graph.set(object, "UIUserInteractionDisabled", true);
        }
        if !no(element, "autoresizesSubviews") {
            self.graph.set(object, "UIAutoresizeSubviews", true);
        }
        let mask = if element.name == "navigationBar" {
            2
        } else {
            autoresizing_mask(element)
        };
        if mask != 0 {
            self.graph.set(object, "UIAutoresizingMask", mask);
        }
        let mode = content_mode(element)?;
        if mode != 0 {
            self.graph.set(object, "UIContentMode", mode);
        }
        if element.name == "visualEffectView"
            || self.special_classes.get(&std::ptr::from_ref(element))
                == Some(&"_UIVisualEffectContentView")
        {
            self.graph.set(object, "UIShouldNotAdaptToMaterials", true);
        }
        if yes(element, "clipsSubviews") {
            self.graph.set(object, "UIClipsToBounds", true);
        }
        if yes(element, "preservesSuperviewLayoutMargins") {
            self.graph
                .set(object, "UIViewPreservesSuperviewMargins", true);
            self.graph
                .set(object, "UIViewPreservesSpecificSuperviewMargins", 15_i64);
        }
        if no(element, "insetsLayoutMarginsFromSafeArea") {
            self.graph
                .set(object, "UIViewInsetsLayoutMarginsFromSafeArea", 0_i64);
        }
        self.layout_margins(element, object)?;
        if let Some(tag) = integer(element, "tag")?
            && tag != 0
        {
            self.graph.set(object, "UITag", tag);
        }
        if let Some(alpha) = number(element, "alpha")?
            && alpha.to_bits() != 1.0_f64.to_bits()
        {
            self.graph.set(object, "UIAlpha", single(alpha));
        }
        if no(element, "clearsContextBeforeDrawing") {
            self.graph
                .set(object, "UIClearsContextBeforeDrawing", false);
        }
        if let Some(identifier) = element.attr("restorationIdentifier") {
            let identifier = self.graph.string(identifier);
            self.graph
                .set(object, "UIRestorationIdentifier", identifier);
        }
        self.view_priorities(element, object)?;
        Ok(())
    }

    #[allow(clippy::float_cmp)] // Archive only exact departures from UIKit's defaults.
    fn view_priorities(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if (self
            .collection_contents
            .contains(&std::ptr::from_ref(element))
            || no(element, "translatesAutoresizingMaskIntoConstraints"))
            && (element.child("constraints").is_some()
                || element
                    .attr("id")
                    .is_some_and(|id| self.constrained.contains(id)))
        {
            self.graph.set(
                object,
                "UIViewDoesNotTranslateAutoresizingMaskIntoConstraints",
                true,
            );
        }
        for (key, horizontal, vertical) in [
            (
                "UIViewContentHuggingPriority",
                "horizontalHuggingPriority",
                "verticalHuggingPriority",
            ),
            (
                "UIViewContentCompressionResistancePriority",
                "horizontalCompressionResistancePriority",
                "verticalCompressionResistancePriority",
            ),
        ] {
            let constrained_switch = element.name == "switch"
                && (element.child("constraints").is_some()
                    || element
                        .attr("id")
                        .is_some_and(|id| self.constrained.contains(id)));
            if constrained_switch || element.name == "activityIndicatorView" {
                let h = number(element, horizontal)?.unwrap_or(750.0);
                let v = number(element, vertical)?.unwrap_or(750.0);
                if h != 750.0 || v != 750.0 {
                    let text = self.graph.string(&format!("{{{h}, {v}}}"));
                    self.graph.set(object, key, text);
                }
                continue;
            }
            if (!yes(element, "fixedFrame")
                && (element.attr(horizontal).is_some() || element.attr(vertical).is_some()))
                || (matches!(
                    element.name.as_str(),
                    "switch"
                        | "navigationBar"
                        | "slider"
                        | "pickerView"
                        | "progressView"
                        | "stepper"
                        | "pageControl"
                        | "segmentedControl"
                        | "searchBar"
                        | "toolbar"
                        | "tabBar"
                ) && key == "UIViewContentHuggingPriority")
            {
                let default = if key == "UIViewContentHuggingPriority" {
                    250.0
                } else {
                    750.0
                };
                let h = number(element, horizontal)?.unwrap_or(default);
                let v = number(element, vertical)?.unwrap_or(default);
                let text = self.graph.string(&format!("{{{h}, {v}}}"));
                self.graph.set(object, key, text);
            }
        }
        Ok(())
    }

    /// The constraints `element` holds, which are in the objects list right
    /// after it.
    fn constraint_objects(&mut self, element: &Element) -> Result<Vec<Id>, String> {
        let owner = element.attr("id").unwrap_or("<anonymous>");
        let mut constraints = Vec::new();
        for constraint in element
            .children_named("constraints")
            .flat_map(|list| list.children.iter())
        {
            if yes(constraint, "placeholder") {
                continue;
            }
            let object = self.constraint(constraint, owner)?;
            if constraint.attr("identifier") != Some("UIIBSystemGenerated") {
                self.objects.push(object);
            }
            constraints.push(object);
        }
        Ok(constraints)
    }

    fn constraint(&mut self, element: &Element, owner: &str) -> Result<Id, String> {
        let id = id_of(element)?;
        validate_constraint(element, id)?;
        let first = self.item(element.attr("firstItem").unwrap_or(owner))?;
        let first_attribute = layout_attribute(element.attr("firstAttribute"), id)?;
        let mut second_attribute = 0_i64;
        let object = self.graph.object("NSLayoutConstraint");
        self.bind(id, object);
        self.graph.set(object, "NSFirstItem", first);
        self.graph
            .set(object, "NSFirstAttributeV2", first_attribute);
        self.graph.set(
            object,
            "NSFirstAttribute",
            legacy_attribute(first_attribute),
        );
        let relation = match element.attr("relation") {
            None => 0,
            Some("lessThanOrEqual") => -1,
            Some("greaterThanOrEqual") => 1,
            Some(other) => {
                return Err(format!("the constraint {id}'s relation {other} is unknown"));
            }
        };
        if relation != 0 {
            self.graph.set(object, "NSRelation", relation);
        }
        if let Some(multiplier) = element.attr("multiplier") {
            let value = ratio(multiplier).ok_or_else(|| {
                format!("the constraint {id}'s multiplier {multiplier} isn't a number")
            })?;
            #[allow(clippy::cast_possible_truncation)]
            let value = f64::from(value as f32);
            if (value - 1.0).abs() > f64::EPSILON {
                self.graph.set(object, "NSMultiplier", value);
            }
        }
        if let Some(second) = element.attr("secondItem") {
            let second = self.item(second)?;
            second_attribute = layout_attribute(element.attr("secondAttribute"), id)?;
            self.graph.set(object, "NSSecondItem", second);
            self.graph
                .set(object, "NSSecondAttributeV2", second_attribute);
            self.graph.set(
                object,
                "NSSecondAttribute",
                legacy_attribute(second_attribute),
            );
        }
        // A symbolic constraint is Interface Builder's standard spacing,
        // which UIKit works out itself, so its constant is the system's.
        let symbolic = match element.attr("symbolic") {
            None | Some("NO") => false,
            Some("YES") => true,
            Some(other) => {
                return Err(format!("the constraint {id}'s symbolic {other} is unknown"));
            }
        };
        let constant = if symbolic {
            let space = self.graph.string("NSSpace");
            self.graph.set(object, "NSSymbolicConstant", space);
            0.0
        } else {
            number(element, "constant")?.unwrap_or(0.0)
        };
        if constant != 0.0 {
            self.graph.set(object, "NSConstantV2", constant);
        }
        if let Some(priority) = integer(element, "priority")?
            && priority != 1000
        {
            self.graph.set(object, "NSPriority", priority);
        }
        self.graph.set(object, "NSShouldBeArchived", true);
        let legacy_constant =
            constant - margin_offset(first_attribute) + margin_offset(second_attribute);
        if legacy_constant != 0.0 {
            self.graph.set(object, "NSConstant", legacy_constant);
        }
        if let Some(identifier) = element.attr("identifier") {
            let identifier = self.graph.string(identifier);
            self.graph.set(object, "NSLayoutIdentifier", identifier);
        }
        self.constraint_variations(element, object)?;
        Ok(object)
    }

    /// The object a constraint names: a view, or a view's safe area.
    fn item(&mut self, ib_id: &str) -> Result<Id, String> {
        if let Some(&object) = self.ids.get(ib_id) {
            return Ok(object);
        }
        if let Some((owner, content)) = self.scroll_guide_owners.get(ib_id).cloned() {
            return Ok(self.scroll_layout_guide(ib_id, &owner, content));
        }
        let Some(owner) = self.guide_owners.get(ib_id).cloned() else {
            return Err(format!(
                "a constraint refers to {ib_id}, which isn't in this view"
            ));
        };
        if let Some(&guide) = self.safe_areas.get(&owner) {
            return Ok(guide);
        }
        let view = self.ids[&owner];
        let guide = self.graph.object("UILayoutGuide");
        self.safe_areas.insert(owner, guide);
        self.graph.set(guide, "UILayoutGuideOwningView", view);
        let identifier = self.graph.string("UIViewSafeAreaLayoutGuide");
        self.graph.set(guide, "UILayoutGuideIdentifier", identifier);
        self.graph
            .set(guide, "UILayoutGuideOwningViewIsLocked", true);
        self.graph.set(guide, "UILayoutGuideShouldBeArchived", true);
        self.graph
            .set(guide, "UILayoutGuideAllowsNegativeDimensions", true);
        // What pins the guide to the view's edges until UIKit insets it.
        let mut system = Vec::new();
        for (edge, attribute, guide_first) in [
            ("top", 3, true),
            ("left", 1, true),
            ("bottom", 4, false),
            ("right", 2, false),
        ] {
            let (first, second) = if guide_first {
                (guide, view)
            } else {
                (view, guide)
            };
            let constraint = self.graph.object("NSLayoutConstraint");
            self.graph.set(constraint, "NSFirstItem", first);
            self.graph
                .set(constraint, "NSFirstAttributeV2", i64::from(attribute));
            self.graph
                .set(constraint, "NSFirstAttribute", i64::from(attribute));
            self.graph.set(constraint, "NSSecondItem", second);
            self.graph
                .set(constraint, "NSSecondAttributeV2", i64::from(attribute));
            self.graph
                .set(constraint, "NSSecondAttribute", i64::from(attribute));
            self.graph.set(constraint, "NSShouldBeArchived", false);
            let name = self
                .graph
                .string(&format!("UIViewSafeAreaLayoutGuide-{edge}"));
            self.graph.set(constraint, "NSLayoutIdentifier", name);
            system.push(constraint);
        }
        let system = self.graph.array("NSArray", &system);
        self.graph
            .set(guide, "UILayoutGuideSystemConstraints", system);
        Ok(guide)
    }

    fn label(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let text = element
            .attr("text")
            .map(str::to_string)
            .or_else(|| keyed(element, "string", "text").map(|string| string.text.clone()));
        if let Some(text) = text {
            let text = self.text(element, "text", &text);
            self.graph.set(object, "UIText", text);
        }
        let description = keyed(element, "fontDescription", "fontDescription");
        let size = description
            .map(|d| number(d, "pointSize"))
            .transpose()?
            .flatten()
            .unwrap_or(17.0);
        let font = self.font(description)?;
        self.graph.set(object, "UIFont", font);
        let color = match keyed(element, "color", "textColor") {
            Some(color) => self.color(color)?,
            None => self.system_color("labelColor", Fallback::White(0.0, 1.0)),
        };
        self.graph.set(object, "UITextColor", color);
        if let Some(color) = keyed(element, "color", "highlightedColor") {
            let color = self.color(color)?;
            self.graph.set(object, "UIHighlightedColor", color);
        } else if !element
            .children_named("nil")
            .any(|e| e.attr("key") == Some("highlightedColor"))
        {
            let color = self.system_color("blackColor", Fallback::White(0.0, 1.0));
            self.graph.set(object, "UIHighlightedColor", color);
        }
        if yes(element, "adjustsFontForContentSizeCategory") {
            self.graph
                .set(object, "UIAdjustsFontForContentSizeCategory", true);
        }
        self.graph
            .set(object, "UIShadowOffset", doubles(&[0.0, -1.0]));
        let baseline = match element.attr("baselineAdjustment") {
            Some("alignBaselines") => 0_i64,
            None | Some("alignCenters") => 1,
            Some("none") => 2,
            Some(other) => return Err(format!("unknown baseline adjustment {other}")),
        };
        if baseline != 0 {
            self.graph.set(object, "UIBaselineAdjustment", baseline);
        }
        let lines = integer(element, "numberOfLines")?.unwrap_or(1);
        if lines != 1 {
            self.graph.set(object, "UINumberOfLines", lines);
        }
        let minimum_size =
            number(element, "minimumFontSize")?.or(number(element, "minimumScaleFactor")?
                .filter(|_| !no(element, "adjustsFontSizeToFit"))
                .map(|scale| scale * size));
        let adjusts = !no(element, "adjustsFontSizeToFit");
        if adjusts {
            self.graph.set(object, "UIAdjustsFontSizeToFit", true);
        }
        if let Some(minimum) = minimum_size.filter(|n| *n != 0.0) {
            #[allow(clippy::cast_possible_truncation)]
            self.graph.set(object, "UIMinimumFontSize", minimum as f32);
        }
        let line_break = match element.attr("lineBreakMode") {
            None | Some("tailTruncation") => 4,
            Some("wordWrap") => 0,
            Some("characterWrap") => 1,
            Some("clip") => 2,
            Some("headTruncation") => 3,
            Some("middleTruncation") => 5,
            Some(other) => return Err(format!("the line break mode {other} is unknown")),
        };
        if line_break != 4 {
            self.graph.set(object, "UILineBreakMode", line_break);
        }
        let alignment = match element.attr("textAlignment") {
            None | Some("left") => 0,
            Some("natural") => 4,
            Some("center") => 1,
            Some("right") => 2,
            Some("justified") => 3,
            Some(other) => return Err(format!("the text alignment {other} is unknown")),
        };
        self.graph.set(object, "UITextAlignment", alignment);
        let scale = match (number(element, "minimumScaleFactor")?, minimum_size) {
            (Some(factor), _) => Some(factor),
            (None, Some(minimum)) => Some(minimum / size),
            (None, None) => None,
        };
        if let Some(scale) = scale.filter(|n| *n != 0.0) {
            #[allow(clippy::cast_possible_truncation)]
            self.graph.set(object, "UIMinimumScaleFactor", scale as f32);
        }
        self.graph.set(
            object,
            "UIDisableUpdateTextColorOnTraitCollectionChange",
            !element
                .children_named("nil")
                .any(|e| e.attr("key") == Some("textColor")),
        );
        Ok(())
    }

    pub fn text(&mut self, element: &Element, key: &str, text: &str) -> Id {
        if self.localized && !text.is_empty() {
            let dev = self.graph.string(text);
            let key = self
                .graph
                .string(&format!("{}.{}", element.attr("id").unwrap_or(""), key));
            self.graph.shared(
                "NSLocalizableString",
                vec![
                    ("NS.bytes", Value::Data(text.as_bytes().to_vec())),
                    ("NSDev", Value::Object(dev)),
                    ("NSKey", Value::Object(key)),
                ],
            )
        } else {
            self.graph.string(text)
        }
    }

    /// The color a `<color>` element describes.
    fn color(&mut self, element: &Element) -> Result<Id, String> {
        if let Some(name) = element
            .attr("systemColor")
            .or(element.attr("xcode11CocoaTouchSystemColor"))
        {
            let fallback = match self.system_colors.get(name) {
                _ if name == "separatorColor" => Fallback::Rgb(
                    0.235_294_117_647_058_82,
                    0.235_294_117_647_058_82,
                    0.262_745_098_039_215_7,
                    0.12,
                ),
                _ if name == "systemBackgroundColor" => Fallback::White(1.0, 1.0),
                _ if name == "labelColor" => Fallback::White(0.0, 1.0),
                Some(color) => components(color)?,
                None if element.attr("xcode11CocoaTouchSystemColor").is_some() => {
                    legacy_system_color(element.attr("cocoaTouchSystemColor"))?
                }
                None => {
                    return Err(format!(
                        "the system color {name} has no fallback in the document"
                    ));
                }
            };
            return Ok(self.system_color(name, fallback));
        }
        if let Some(name) = element.attr("cocoaTouchSystemColor") {
            return Ok(self.system_color(name, legacy_system_color(Some(name))?));
        }
        if let Some(name) = element.attr("name") {
            let fallback = self
                .named_colors
                .get(name)
                .ok_or_else(|| format!("the named color {name} has no fallback in the document"))?;
            let fallback = components(fallback)?;
            let name = self.graph.string(name);
            let mut values = vec![
                ("UIDynamicCatalogName", Value::Object(name)),
                ("UIDynamicCatalogUseNibBundle", Value::Bool(true)),
            ];
            values.extend(color_values_precision(fallback, false));
            return Ok(self.graph.shared("UIColor", values));
        }
        let fallback = components(element)?;
        if matches!(fallback, Fallback::White(0.0, 0.0)) {
            return Ok(self.system_color("clearColor", fallback));
        }
        let calibrated = element
            .attr("colorSpace")
            .is_some_and(|s| s.starts_with("calibrated"))
            || element
                .attr("customColorSpace")
                .is_some_and(|s| s.starts_with("calibrated"));
        let values = color_values_precision(fallback, !calibrated);
        Ok(self.graph.shared("UIColor", values))
    }

    fn system_color(&mut self, name: &str, fallback: Fallback) -> Id {
        let name = self.graph.string(name);
        let mut values = vec![("UISystemColorName", Value::Object(name))];
        values.extend(color_values(fallback));
        self.graph.shared("UIColor", values)
    }
}

/// What a color archives of its components.
fn color_values(fallback: Fallback) -> Vec<(&'static str, Value)> {
    color_values_precision(fallback, true)
}

fn color_values_precision(fallback: Fallback, round: bool) -> Vec<(&'static str, Value)> {
    match fallback {
        Fallback::White(white, alpha) => {
            let text = color_text(&[white], alpha);
            let mut values = vec![
                ("UIColorComponentCount", Value::Int(2)),
                ("UIWhite", Value::Float(single(white))),
                ("UIAlpha", Value::Float(single(alpha))),
                ("NSWhite", Value::Data(text.into_bytes())),
                ("NSColorSpace", Value::Int(4)),
            ];
            for (key, value) in [("UIWhite-Double", white), ("UIAlpha-Double", alpha)] {
                if f64::from(single(value)).to_bits() != value.to_bits() {
                    values.push((key, Value::Double(value)));
                }
            }
            values
        }
        Fallback::Rgb(red, green, blue, alpha) => {
            let mut values = vec![("UIColorComponentCount", Value::Int(4))];
            for (key, double_key, value) in [
                ("UIRed", "UIRed-Double", red),
                ("UIGreen", "UIGreen-Double", green),
                ("UIBlue", "UIBlue-Double", blue),
                ("UIAlpha", "UIAlpha-Double", alpha),
            ] {
                let value = if round { rounded(value) } else { value };
                values.push((key, Value::Float(single(value))));
                if f64::from(single(value)).to_bits() != value.to_bits() {
                    values.push((double_key, Value::Double(value)));
                }
            }
            let text = color_text(&[red, green, blue], alpha);
            values.push(("NSRGB", Value::Data(text.into_bytes())));
            values.push(("NSColorSpace", Value::Int(2)));
            values
        }
    }
}

/// A color's components where it has no system color to resolve to.
#[derive(Debug, Clone, Copy)]
enum Fallback {
    White(f64, f64),
    Rgb(f64, f64, f64, f64),
}

fn components(element: &Element) -> Result<Fallback, String> {
    let space = element
        .attr("customColorSpace")
        .or_else(|| element.attr("colorSpace"))
        .unwrap_or("");
    let alpha = number(element, "alpha")?.unwrap_or(1.0);
    match space {
        "sRGB" => Ok(Fallback::Rgb(
            number(element, "red")?.unwrap_or(0.0),
            number(element, "green")?.unwrap_or(0.0),
            number(element, "blue")?.unwrap_or(0.0),
            alpha,
        )),
        "genericGamma22GrayColorSpace" => Ok(Fallback::White(
            number(element, "white")?.unwrap_or(0.0),
            alpha,
        )),
        "calibratedWhite" => {
            let white = number(element, "white")?.unwrap_or(0.0);
            Ok(Fallback::White(calibrated::gray(white)?, alpha))
        }
        "calibratedRGB" => {
            let [r, g, b] = calibrated::rgb([
                number(element, "red")?.unwrap_or(0.0),
                number(element, "green")?.unwrap_or(0.0),
                number(element, "blue")?.unwrap_or(0.0),
            ])?;
            Ok(Fallback::Rgb(r, g, b, alpha))
        }
        other => Err(format!(
            "the color space {other} isn't one SweetPad's ibtool compiles yet"
        )),
    }
}

/// The color an Xcode 10 document gives a system color for older iOS.
fn legacy_system_color(name: Option<&str>) -> Result<Fallback, String> {
    match name {
        Some("whiteColor") => Ok(Fallback::White(1.0, 1.0)),
        Some("blackColor" | "darkTextColor") => Ok(Fallback::White(0.0, 1.0)),
        Some("clearColor") => Ok(Fallback::White(0.0, 0.0)),
        Some("groupTableViewBackgroundColor") => Ok(Fallback::Rgb(
            242.0 / 255.0,
            242.0 / 255.0,
            247.0 / 255.0,
            1.0,
        )),
        Some(other) => Err(format!(
            "the system color {other} isn't one SweetPad's ibtool compiles yet"
        )),
        None => Err("a system color has no fallback".to_string()),
    }
}

/// A component as Interface Builder keeps it: to ten significant digits.
fn rounded(value: f64) -> f64 {
    if value == 0.0 {
        return value;
    }
    format!("{value:.9e}").parse().unwrap_or(value)
}

#[allow(clippy::cast_possible_truncation)]
fn single(value: f64) -> f32 {
    value as f32
}

/// AppKit's text for a color's components: three significant digits each,
/// the alpha only when it isn't 1.
fn color_text(components: &[f64], alpha: f64) -> String {
    let mut parts: Vec<String> = components.iter().map(|&c| short(c)).collect();
    if (alpha - 1.0).abs() > f64::EPSILON {
        parts.push(short(alpha));
    }
    parts.join(" ")
}

fn short(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    #[allow(clippy::cast_possible_truncation)]
    let digits = (2 - value.abs().log10().floor() as i32).max(0);
    let text = format!("{value:.*}", usize::try_from(digits).unwrap_or(0));
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// `1/3`, `16:9` or a plain number.
fn ratio(text: &str) -> Option<f64> {
    if let Some((a, b)) = text.split_once(['/', ':']) {
        return Some(a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?);
    }
    text.parse().ok()
}

fn layout_attribute(name: Option<&str>, constraint: &str) -> Result<i64, String> {
    Ok(match name {
        Some("left") => 1,
        Some("right") => 2,
        Some("top") => 3,
        Some("bottom") => 4,
        Some("leading") => 5,
        Some("trailing") => 6,
        Some("width") => 7,
        Some("height") => 8,
        Some("centerX") => 9,
        Some("centerY") => 10,
        Some("baseline" | "lastBaseline") => 11,
        Some("firstBaseline") => 12,
        Some("leftMargin") => 13,
        Some("rightMargin") => 14,
        Some("topMargin") => 15,
        Some("bottomMargin") => 16,
        Some("leadingMargin") => 17,
        Some("trailingMargin") => 18,
        Some("centerXWithinMargins") => 19,
        Some("centerYWithinMargins") => 20,
        Some(other) => {
            return Err(format!(
                "the constraint {constraint}'s attribute {other} is unknown"
            ));
        }
        None => return Err(format!("the constraint {constraint} has no attribute")),
    })
}

fn autoresizing_mask(element: &Element) -> i64 {
    let Some(mask) = keyed(element, "autoresizingMask", "autoresizingMask") else {
        return 36;
    };
    [
        ("flexibleMinX", 1),
        ("widthSizable", 2),
        ("flexibleMaxX", 4),
        ("flexibleMinY", 8),
        ("heightSizable", 16),
        ("flexibleMaxY", 32),
    ]
    .iter()
    .filter(|(attribute, _)| yes(mask, attribute))
    .map(|(_, bit)| bit)
    .sum()
}

fn content_mode(element: &Element) -> Result<i64, String> {
    Ok(match element.attr("contentMode") {
        None | Some("scaleToFill") => 0,
        Some("scaleAspectFit") => 1,
        Some("scaleAspectFill") => 2,
        Some("redraw") => 3,
        Some("center") => 4,
        Some("top") => 5,
        Some("bottom") => 6,
        Some("left") => 7,
        Some("right") => 8,
        Some("topLeft") => 9,
        Some("topRight") => 10,
        Some("bottomLeft") => 11,
        Some("bottomRight") => 12,
        Some(other) => return Err(format!("the content mode {other} is unknown")),
    })
}

fn legacy_attribute(attribute: i64) -> i64 {
    match attribute {
        13..=18 => attribute - 12,
        19..=20 => attribute - 10,
        _ => attribute,
    }
}
fn margin_offset(attribute: i64) -> f64 {
    match attribute {
        13 | 15 | 17 => 8.0,
        14 | 16 | 18 => -8.0,
        _ => 0.0,
    }
}

fn event_mask(connection: &Element) -> Result<i64, String> {
    Ok(match connection.attr("eventType") {
        None => 0_i64,
        Some("touchDown") => 1_i64,
        Some("touchDownRepeat") => 2,
        Some("touchDragInside") => 4,
        Some("touchDragOutside") => 8,
        Some("touchDragEnter") => 16,
        Some("touchDragExit") => 32,
        Some("touchUpInside") => 64,
        Some("touchUpOutside") => 128,
        Some("touchCancel") => 256,
        Some("valueChanged") => 4096,
        Some("primaryActionTriggered") => 8192,
        Some("editingDidBegin") => 65536,
        Some("editingChanged") => 131_072,
        Some("editingDidEnd") => 262_144,
        Some("editingDidEndOnExit") => 524_288,
        other => return Err(format!("unsupported action event {other:?}")),
    })
}

fn validate_constraint(element: &Element, id: &str) -> Result<(), String> {
    if element.name != "constraint" {
        return Err(format!("<{}> {id} isn't a constraint", element.name));
    }
    for (attribute, _) in &element.attributes {
        if !matches!(
            attribute.as_str(),
            "id" | "firstItem"
                | "firstAttribute"
                | "secondItem"
                | "secondAttribute"
                | "constant"
                | "multiplier"
                | "relation"
                | "priority"
                | "identifier"
                | "userLabel"
                | "symbolic"
        ) {
            return Err(format!(
                "the constraint {id}'s {attribute} isn't something SweetPad's ibtool compiles yet"
            ));
        }
    }
    Ok(())
}

fn collect_constraints(element: &Element, ids: &mut HashSet<String>) {
    if element.name == "constraint" {
        for key in ["firstItem", "secondItem"] {
            if let Some(id) = element.attr(key) {
                ids.insert(id.to_string());
            }
        }
    }
    for child in &element.children {
        collect_constraints(child, ids);
    }
}

fn semantic_content(element: &Element) -> Result<i64, String> {
    match element.attr("semanticContentAttribute") {
        None | Some("unspecified") => Ok(0),
        Some("playback") => Ok(1),
        Some("spatial") => Ok(2),
        Some("forceLeftToRight") => Ok(3),
        Some("forceRightToLeft") => Ok(4),
        Some(value) => Err(format!("unsupported semantic content attribute {value}")),
    }
}
