//! Controller scenes, embedded child controllers, and the boundary between
//! a controller's nib and the separate nib that loads its view.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use sweetpad_lib::xcscheme::Element;

use super::{Compiled, nib_root, outlet, proxy};
use crate::cli::ibtool::nib::Id;
use crate::cli::ibtool::uikit::Encoder;
use crate::cli::ibtool::{Options, custom_class};
use crate::cli::keyed_archive::{self, Plist};

fn class(element: &Element) -> Result<&'static str, String> {
    match element.name.as_str() {
        "viewController" | "hostingController" | "viewControllerPlaceholder" => {
            Ok("UIViewController")
        }
        "navigationController" => Ok("UINavigationController"),
        "tableViewController" => Ok("UITableViewController"),
        "collectionViewController" => Ok("UICollectionViewController"),
        "splitViewController" => Ok("UISplitViewController"),
        "tabBarController" => Ok("UITabBarController"),
        "pageViewController" => Ok("UIPageViewController"),
        other => Err(format!("unsupported controller <{other}>")),
    }
}

fn id(element: &Element) -> Result<&str, String> {
    element
        .attr("id")
        .ok_or_else(|| "an object has no id".to_string())
}

fn view(element: &Element) -> Option<&Element> {
    element
        .children
        .iter()
        .find(|child| child.attr("key") == Some("view"))
}

fn name(element: &Element) -> String {
    element.attr("storyboardIdentifier").map_or_else(
        || {
            format!(
                "{}-{}",
                if element.name == "hostingController" {
                    "UIHostingController"
                } else {
                    class(element).unwrap_or("UIViewController")
                },
                element.attr("id").unwrap_or("")
            )
        },
        str::to_string,
    )
}

fn children(element: &Element) -> impl Iterator<Item = &str> {
    let mut children: Vec<_> = element
        .children_named("connections")
        .flat_map(|list| list.children_named("segue"))
        .filter(|s| s.attr("kind") == Some("relationship"))
        .collect();
    children.sort_by_key(|s| match s.attr("relationship") {
        Some("supplementaryViewController") => 1,
        Some("detailViewController") => 2,
        _ => 0,
    });
    children.into_iter().filter_map(|s| s.attr("destination"))
}

fn walk<'a>(element: &'a Element, visit: &mut impl FnMut(&'a Element)) {
    visit(element);
    for child in &element.children {
        walk(child, visit);
    }
}

fn view_ids(element: &Element) -> HashSet<String> {
    let mut ids = HashSet::new();
    walk(element, &mut |e| {
        if let Some(id) = e.attr("id") {
            ids.insert(id.to_string());
        }
    });
    ids
}

/// Every connection that crosses from this view to its controller archive
/// needs a named proxy in the view and an entry in the controller's table.
fn external_ids(controller: &Element) -> Vec<String> {
    let Some(view) = view(controller) else {
        return Vec::new();
    };
    let inside = view_ids(view);
    let mut external = BTreeSet::new();
    walk(controller, &mut |e| {
        if let Some(connections) = e.child("connections") {
            for connection in &connections.children {
                if connection.name == "segue"
                    && connection
                        .attr("kind")
                        .is_some_and(|kind| kind != "relationship")
                    && e.attr("id").is_some_and(|id| inside.contains(id))
                    && let Some(id) = connection.attr("id")
                {
                    external.insert(id.to_string());
                }
                if !matches!(
                    connection.name.as_str(),
                    "outlet" | "outletCollection" | "action"
                ) {
                    continue;
                }
                if let (Some(source), Some(destination)) =
                    (e.attr("id"), connection.attr("destination"))
                {
                    if inside.contains(source) && !inside.contains(destination) {
                        external.insert(if connection.name == "action" {
                            controller.attr("id").unwrap_or(destination).to_string()
                        } else {
                            destination.to_string()
                        });
                    }
                    if !inside.contains(source) && inside.contains(destination) {
                        external.insert(source.to_string());
                    }
                }
            }
        }
    });
    let mut result: Vec<_> = external.into_iter().collect();
    if let Some(index) = result
        .iter()
        .position(|id| Some(id.as_str()) == controller.attr("id"))
    {
        let owner = result.remove(index);
        result.insert(0, owner);
    }
    result
}

fn extras<'a>(document: &'a Element, controller: &Element) -> Vec<&'a Element> {
    document
        .children_named("scenes")
        .flat_map(|s| s.children_named("scene"))
        .filter_map(|s| s.child("objects"))
        .find(|objects| {
            objects
                .children
                .iter()
                .any(|e| e.attr("id") == controller.attr("id"))
        })
        .map(|objects| {
            objects
                .children
                .iter()
                .filter(|e| !matches!(e.name.as_str(), "placeholder" | "exit") && class(e).is_err())
                .collect()
        })
        .unwrap_or_default()
}

fn segues(controller: &Element) -> Vec<&Element> {
    let mut result = Vec::new();
    walk(controller, &mut |e| {
        if e.name == "segue" && e.attr("kind").is_some_and(|k| k != "relationship") {
            result.push(e);
        }
    });
    result
}

fn embedded<'a>(
    root: &'a Element,
    controllers: &HashMap<&str, &'a Element>,
    seen: &mut HashSet<String>,
    result: &mut Vec<&'a Element>,
) -> Result<(), String> {
    if !seen.insert(id(root)?.to_string()) {
        return Err("cyclic controller relationships".to_string());
    }
    result.push(root);
    for child in children(root) {
        let child = controllers
            .get(child)
            .ok_or_else(|| format!("missing child controller {child}"))?;
        embedded(child, controllers, seen, result)?;
    }
    Ok(())
}

fn reachable_controllers<'a>(
    document: &Element,
    controllers: &HashMap<&'a str, &'a Element>,
    order: &[&'a Element],
) -> Result<HashSet<&'a str>, String> {
    let mut reachable = HashSet::new();
    let mut pending: Vec<&str> = order
        .iter()
        .filter(|e| {
            e.attr("storyboardIdentifier").is_some()
                || e.attr("id") == document.attr("initialViewController")
        })
        .filter_map(|e| e.attr("id"))
        .collect();
    while let Some(current) = pending.pop() {
        if !reachable.insert(current) {
            continue;
        }
        let controller = *controllers
            .get(current)
            .ok_or("missing reachable controller")?;
        pending.extend(children(controller));
        pending.extend(
            segues(controller)
                .into_iter()
                .filter(|segue| segue.attr("kind") != Some("unwind"))
                .filter_map(|s| s.attr("destination")),
        );
    }
    Ok(reachable)
}

fn external_reference(controller: &Element) -> Result<(String, Plist), String> {
    if controller.attr("storyboardName").is_none() {
        return Err("an external storyboard reference has no storyboard name".into());
    }
    let fields = [
        ("bundleIdentifier", "UIReferencedBundleIdentifier"),
        ("storyboardName", "UIReferencedStoryboardName"),
        ("referencedIdentifier", "UIReferencedControllerIdentifier"),
    ]
    .into_iter()
    .filter_map(|(attribute, key)| {
        controller
            .attr(attribute)
            .map(|value| (key.to_string(), Plist::String(value.to_string())))
    })
    .collect();
    Ok((name(controller), Plist::Dict(fields)))
}

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Compiled, String> {
    if !matches!(
        document.attr("type"),
        Some(
            "com.apple.InterfaceBuilder3.CocoaTouch.Storyboard.XIB"
                | "com.apple.InterfaceBuilder.AppleTV.Storyboard"
        )
    ) {
        return Err("this isn't an iOS storyboard".to_string());
    }
    let mut controllers = HashMap::new();
    let mut order = Vec::new();
    for objects in document
        .children_named("scenes")
        .flat_map(|s| s.children_named("scene"))
        .filter_map(|s| s.child("objects"))
    {
        for element in &objects.children {
            if class(element).is_ok() {
                controllers.insert(id(element)?, element);
                order.push(element);
            } else if !matches!(element.name.as_str(), "placeholder" | "exit")
                && !matches!(element.name.as_str(), "view" | "imageView" | "label")
                && super::super::uikit::gestures::class(&element.name).is_none()
            {
                return Err(format!("unsupported scene object <{}>", element.name));
            }
        }
    }
    let reachable = reachable_controllers(document, &controllers, &order)?;
    let segue_targets: HashSet<&str> = order
        .iter()
        .filter(|e| e.attr("id").is_some_and(|id| reachable.contains(id)))
        .flat_map(|e| segues(e))
        .filter_map(|s| s.attr("destination"))
        .collect();
    let child_ids: HashSet<&str> = order.iter().flat_map(|e| children(e)).collect();
    let mut nibs = Vec::new();
    let mut identifiers = BTreeMap::new();
    let mut entry = None;
    let mut external_references = Vec::new();
    for controller in &order {
        let controller_id = id(controller)?;
        if !reachable.contains(controller_id) {
            continue;
        }
        if controller.name == "viewControllerPlaceholder" {
            external_references.push(external_reference(controller)?);
        }
        if let Some(view) = view(controller) {
            nibs.push((
                format!("{controller_id}-view-{}.nib", id(view)?),
                view_nib(document, controller, options)?,
            ));
        }
        if child_ids.contains(controller_id)
            && controller.attr("storyboardIdentifier").is_none()
            && !segue_targets.contains(controller_id)
            && document.attr("initialViewController") != Some(controller_id)
        {
            continue;
        }
        let name = name(controller);
        if document.attr("initialViewController") == Some(controller_id) {
            entry = Some(name.clone());
        }
        identifiers.insert(name.clone(), Plist::String(name.clone()));
        nibs.push((
            format!("{name}.nib"),
            controller_nib(document, controller, &controllers, options)?,
        ));
    }
    let mut info = Vec::new();
    if !external_references.is_empty() {
        info.push((
            "UIViewControllerIdentifiersToExternalStoryboardReferences".to_string(),
            Plist::Dict(external_references),
        ));
    }
    if let Some(entry) = entry {
        info.push((
            "UIStoryboardDesignatedEntryPointIdentifier".to_string(),
            Plist::String(entry),
        ));
    }
    info.push(("UIStoryboardVersion".to_string(), Plist::Int(1)));
    info.push((
        "UIViewControllerIdentifiersToNibNames".to_string(),
        Plist::Dict(identifiers.into_iter().collect()),
    ));
    Ok(Compiled {
        nibs,
        info: keyed_archive::write(&Plist::Dict(info)),
    })
}

fn view_nib(
    document: &Element,
    controller: &Element,
    options: &Options<'_>,
) -> Result<Vec<u8>, String> {
    let view = view(controller).ok_or("a controller has no view")?;
    let mut encoder = Encoder::new(document, options);
    let owner = proxy(&mut encoder.graph, "IBFilesOwner");
    let responder = proxy(&mut encoder.graph, "IBFirstResponder");
    let mut top = vec![owner, responder];
    for (i, external) in external_ids(controller).iter().enumerate() {
        let object = proxy(
            &mut encoder.graph,
            &format!("UpstreamPlaceholder-{}", i + 1),
        );
        encoder.bind(external, object);
        top.push(object);
    }
    if let Some(owner_proxy) = encoder.lookup(id(controller)?) {
        let inside = view_ids(view);
        walk(view, &mut |element| {
            if let Some(list) = element.child("connections") {
                for action in list.children_named("action") {
                    if let Some(destination) = action.attr("destination")
                        && !inside.contains(destination)
                    {
                        encoder.bind(destination, owner_proxy);
                    }
                }
            }
        });
    }
    if controller.children.iter().any(|e| {
        e.attr("key") == Some("simulatedTopBarMetrics") && e.attr("translucent") == Some("NO")
    }) {
        let device = document.child("device").and_then(|d| d.attr("id"));
        let height = match device {
            Some("retina6_1") => Some(896.0),
            Some("retina5_9") => Some(812.0),
            Some("retina4_7") => Some(667.0),
            _ => None,
        };
        if let (Some(frame), Some(height)) = (
            view.children_named("rect")
                .find(|r| r.attr("key") == Some("frame")),
            height,
        ) {
            let width = frame
                .attr("width")
                .and_then(|w| w.parse().ok())
                .ok_or("a root view has no width")?;
            encoder
                .frame_overrides
                .insert(id(view)?.to_string(), [0.0, 0.0, width, height]);
        }
    }
    encoder.register_controller_guides(controller, view);
    let root_view = encoder.root_view(view)?;
    if document.attr("useSafeAreas") == Some("YES")
        && view.name == "view"
        && view.child("viewLayoutGuide").is_none()
    {
        encoder.unused_safe_area();
    }
    if let Some(refresh) = controller.child("refreshControl") {
        encoder.declare(refresh)?;
        let control = encoder.view(refresh)?;
        encoder.document_tint(control, refresh)?;
        encoder.graph.set(root_view, "UIRefreshControl", control);
    }
    let mut objects = top.clone();
    top.push(root_view);
    top.extend(encoder.static_sources.iter().copied());
    objects.extend(encoder.objects.iter().copied());
    // A table can query its data source as soon as that outlet is connected.
    // Install the controller's static sections first, as Xcode does, so the
    // table does not cache empty section headers during nib loading.
    let mut connections = Vec::new();
    for source in &encoder.static_sources {
        connections.push(outlet(
            &mut encoder.graph,
            "staticDataSource",
            owner,
            *source,
        ));
    }
    connections.extend(encoder.connections_in_graph(controller, false)?);
    connections.push(outlet(&mut encoder.graph, "view", owner, root_view));
    let root = nib_root(
        &mut encoder.graph,
        &top,
        &objects,
        &connections,
        &encoder.key_value_pairs,
    );
    encoder.install_accessibility(root);
    Ok(encoder.graph.write(root))
}

fn controller_nib(
    document: &Element,
    root: &Element,
    controllers: &HashMap<&str, &Element>,
    options: &Options<'_>,
) -> Result<Vec<u8>, String> {
    let mut group = Vec::new();
    embedded(root, controllers, &mut HashSet::new(), &mut group)?;
    let mut encoder = Encoder::new(document, options);
    let mut placeholders = HashMap::new();
    let mut top = Vec::new();
    for child in group.iter().skip(1) {
        let object = proxy(&mut encoder.graph, "UIStoryboardPlaceholder");
        placeholders.insert(id(child)?, object);
        top.push(object);
    }
    let owner = proxy(&mut encoder.graph, "IBFilesOwner");
    let responder = proxy(&mut encoder.graph, "IBFirstResponder");
    let storyboard = proxy(&mut encoder.graph, "UIStoryboardPlaceholder");
    placeholders.insert(id(root)?, storyboard);
    top.extend([owner, responder, storyboard]);
    let wrapped = declare_controllers(
        &mut encoder,
        &group,
        controllers,
        document.attr("initialViewController"),
    )?;
    let root_object = encoder.lookup(id(root)?).unwrap();
    let bars = navigation_bars(&mut encoder, &group)?;
    let nav_items = navigation_items(&mut encoder, &group, &bars)?;
    let keep_alive = templates_and_extras(
        &mut encoder,
        document,
        &group,
        controllers,
        &mut top,
        options,
    )?;
    for controller in &group {
        let controller_id = id(controller)?;
        let object = encoder.lookup(controller_id).unwrap();
        if controller.name == "viewControllerPlaceholder" {
            let identifier = encoder.graph.string(&name(controller));
            encoder
                .graph
                .set(object, "UIStoryboardIdentifier", identifier);
            continue;
        }
        controller_metadata(
            &mut encoder,
            controller,
            object,
            keep_alive.get(controller_id),
            &nav_items,
        )?;
        controller_view(&mut encoder, controller, object)?;
        controller_flags(&mut encoder, controller, object);
        controller_relationships(&mut encoder, controller, object, &bars, &nav_items)?;
        split_properties(&mut encoder, controller, object)?;
        page_properties(&mut encoder, controller, object)?;
        controller_class(
            &mut encoder,
            controller,
            object,
            wrapped.contains(controller_id),
            options,
        )?;
    }
    let connections = controller_connections(
        &mut encoder,
        document,
        &group,
        &placeholders,
        owner,
        root_object,
    )?;
    Ok(finish_controller(encoder, top, root_object, &connections))
}

fn page_properties(
    encoder: &mut Encoder<'_>,
    controller: &Element,
    object: Id,
) -> Result<(), String> {
    if controller.name != "pageViewController" {
        return Ok(());
    }
    for (attribute, key, choices, default) in [
        (
            "transitionStyle",
            "UITransitionStyle",
            &["pageCurl", "scroll"][..],
            0_i64,
        ),
        (
            "navigationOrientation",
            "UINavigationOrientation",
            &["horizontal", "vertical"][..],
            0,
        ),
        (
            "spineLocation",
            "UISpineLocation",
            &["none", "min", "mid", "max"][..],
            2,
        ),
    ] {
        let value = match controller.attr(attribute) {
            Some(value) => i64::try_from(
                choices
                    .iter()
                    .position(|c| *c == value)
                    .ok_or_else(|| format!("unsupported page controller {attribute}={value}"))?,
            )
            .unwrap(),
            None => default,
        };
        encoder.graph.set(object, key, value);
    }
    encoder.graph.set(object, "UIPageSpacing", 0_i64);
    if controller.attr("doubleSided") == Some("YES") {
        encoder.graph.set(object, "UIDoubleSided", true);
    }
    Ok(())
}

fn declare_controllers(
    encoder: &mut Encoder<'_>,
    group: &[&Element],
    controllers: &HashMap<&str, &Element>,
    initial: Option<&str>,
) -> Result<HashSet<String>, String> {
    let mut wrapped = HashSet::new();
    for controller in group {
        let controller_id = id(controller)?;
        let wrap = !controllers
            .values()
            .any(|parent| children(parent).any(|child| child == controller_id))
            || controller.attr("customClass").is_some()
            || initial == Some(controller_id);
        let object = encoder
            .graph
            .object(if controller.name == "viewControllerPlaceholder" {
                "UIStoryboardViewControllerPlaceholder"
            } else if wrap {
                "UIClassSwapper"
            } else {
                class(controller)?
            });
        encoder.bind(controller_id, object);
        encoder.objects.push(object);
        if wrap && controller.name != "viewControllerPlaceholder" {
            wrapped.insert(controller_id.to_string());
        }
    }
    Ok(wrapped)
}

fn navigation_bars(
    encoder: &mut Encoder<'_>,
    group: &[&Element],
) -> Result<HashMap<String, Id>, String> {
    let mut bars = HashMap::new();
    for controller in group {
        if let Some(bar) = controller
            .child("navigationBar")
            .or_else(|| controller.child("tabBar"))
        {
            encoder.declare(bar)?;
            if bar.name == "tabBar" {
                // Xcode 27 archives a controller-owned tab bar at its intrinsic
                // height, with its width supplied by the controller at load time.
                encoder
                    .frame_overrides
                    .insert(id(bar)?.to_string(), [0.0, 0.0, 0.0, 54.0]);
            }
            let object = encoder.view(bar)?;
            if bar.name == "tabBar" {
                encoder.graph.replace(object, "UIAutoresizingMask", 10_i64);
            }
            encoder.document_tint(object, bar)?;
            bars.insert(id(controller)?.to_string(), object);
        }
    }
    Ok(bars)
}

fn navigation_items(
    encoder: &mut Encoder<'_>,
    group: &[&Element],
    bars: &HashMap<String, Id>,
) -> Result<HashMap<String, Id>, String> {
    let mut nav_items = HashMap::new();
    for controller in group {
        if let Some(item) = controller.child("tabBarItem") {
            let item = encoder.tab_item(item)?;
            let controller = encoder
                .lookup(id(controller)?)
                .ok_or("missing tab item owner")?;
            encoder.graph.set(controller, "UITabBarItem", item);
        }
        if let Some(item) = controller.child("navigationItem") {
            let parent_bar = group
                .iter()
                .find(|parent| {
                    parent.name == "navigationController"
                        && children(parent).any(|child| Some(child) == controller.attr("id"))
                })
                .and_then(|parent| bars.get(parent.attr("id").unwrap_or("")))
                .copied();
            let object = encoder.navigation_item(item, parent_bar)?;
            nav_items.insert(id(controller)?.to_string(), object);
        } else if controller.name == "viewControllerPlaceholder"
            && let Some(bar) = group
                .iter()
                .find(|parent| {
                    parent.name == "navigationController"
                        && children(parent).any(|child| Some(child) == controller.attr("id"))
                })
                .and_then(|parent| bars.get(parent.attr("id").unwrap_or("")))
        {
            let object = encoder.graph.object("UINavigationItem");
            encoder.graph.set(object, "UILargeTitleDisplayMode", 0_i64);
            encoder.graph.set(object, "UINavigationBar", *bar);
            nav_items.insert(id(controller)?.to_string(), object);
        }
    }
    Ok(nav_items)
}

fn segue_template(
    encoder: &mut Encoder<'_>,
    segue: &Element,
    controllers: &HashMap<&str, &Element>,
    options: &Options<'_>,
) -> Result<Id, String> {
    let kind = segue.attr("kind").ok_or("a segue has no kind")?;
    let class = match kind {
        "show" | "showDetail" => "UIStoryboardShowSegueTemplate",
        "presentation" => "UIStoryboardPresentationSegueTemplate",
        "custom" => "UIStoryboardSegueTemplate",
        "unwind" => "UIStoryboardUnwindSegueTemplate",
        "embed" => "UIStoryboardEmbedSegueTemplate",
        other => return Err(format!("unsupported segue kind {other:?}")),
    };
    let template = encoder.graph.object(class);
    if let Some(identifier) = segue.attr("identifier") {
        let value = encoder.graph.string(identifier);
        encoder.graph.set(template, "UIIdentifier", value);
    }
    if kind == "custom" {
        let class = custom_class(segue, options).ok_or("a custom segue has no class")?;
        let value = encoder.graph.string(&class);
        encoder.graph.set(template, "UISegueClassName", value);
    }
    if kind == "unwind" {
        encoder.graph.set(
            template,
            "UIDestinationViewControllerIdentifier",
            super::super::nib::Value::Nil,
        );
        let action = encoder.graph.string(
            segue
                .attr("unwindAction")
                .ok_or("an unwind segue has no action")?,
        );
        encoder.graph.set(template, "UIAction", action);
    } else {
        let target = controllers
            .get(
                segue
                    .attr("destination")
                    .ok_or("a segue has no destination")?,
            )
            .ok_or("a segue target is missing")?;
        let target = encoder.graph.string(&name(target));
        encoder
            .graph
            .set(template, "UIDestinationViewControllerIdentifier", target);
    }
    match kind {
        "show" | "showDetail" => {
            let action = encoder.graph.string(if kind == "show" {
                "showViewController:sender:"
            } else {
                "showDetailViewController:sender:"
            });
            encoder.graph.set(template, "UIActionName", action);
        }
        "embed" => {
            encoder.graph.set(template, "UIPerformOnViewLoad", true);
            encoder
                .graph
                .set(template, "UIContainerView", super::super::nib::Value::Nil);
        }
        "presentation" => {
            if segue.attr("animates") == Some("NO") {
                encoder.graph.set(template, "UIAnimates", false);
            }
            presentation_styles(encoder, segue, template)?;
        }
        _ => {}
    }
    Ok(template)
}

fn presentation_styles(
    encoder: &mut Encoder<'_>,
    segue: &Element,
    template: Id,
) -> Result<(), String> {
    for (attribute, key, values) in [
        (
            "modalPresentationStyle",
            "UIModalPresentationStyle",
            &[
                ("fullScreen", 0),
                ("pageSheet", 1),
                ("formSheet", 2),
                ("currentContext", 3),
                ("custom", 4),
                ("overFullScreen", 5),
                ("overCurrentContext", 6),
                ("popover", 7),
                ("automatic", -2),
            ][..],
        ),
        (
            "modalTransitionStyle",
            "UIModalTransitionStyle",
            &[
                ("coverVertical", 0),
                ("flipHorizontal", 1),
                ("crossDissolve", 2),
                ("partialCurl", 3),
            ][..],
        ),
    ] {
        if let Some(value) = segue.attr(attribute) {
            let (_, value) = values
                .iter()
                .find(|(name, _)| *name == value)
                .ok_or_else(|| format!("unsupported {attribute}: {value}"))?;
            encoder.graph.set(template, key, *value);
        }
    }
    Ok(())
}

fn templates_and_extras(
    encoder: &mut Encoder<'_>,
    document: &Element,
    group: &[&Element],
    controllers: &HashMap<&str, &Element>,
    top: &mut Vec<Id>,
    options: &Options<'_>,
) -> Result<HashMap<String, Vec<Id>>, String> {
    let mut keep_alive = HashMap::new();
    for controller in group {
        let mut views = Vec::new();
        for extra in extras(document, controller) {
            views.push(encoder.root_view(extra)?);
        }
        if !views.is_empty() {
            top.extend(views.iter().copied());
            keep_alive.insert(id(controller)?.to_string(), views);
        }
        for segue in segues(controller) {
            let template = segue_template(encoder, segue, controllers, options)?;
            encoder.bind(id(segue)?, template);
            encoder.objects.push(template);
        }
    }
    Ok(keep_alive)
}

fn controller_metadata(
    encoder: &mut Encoder<'_>,
    controller: &Element,
    object: Id,
    keep_alive: Option<&Vec<Id>>,
    nav_items: &HashMap<String, Id>,
) -> Result<(), String> {
    let controller_id = id(controller)?;
    let templates: Vec<Id> = segues(controller)
        .iter()
        .filter_map(|s| encoder.lookup(s.attr("id")?))
        .collect();
    if !templates.is_empty() {
        let array = encoder.graph.array("NSArray", &templates);
        encoder
            .graph
            .set(object, "UIStoryboardSegueTemplates", array);
    }
    if let Some(views) = keep_alive {
        let array = encoder.graph.array("NSArray", views);
        encoder
            .graph
            .set(object, "UITopLevelObjectsToKeepAliveFromStoryboard", array);
    }
    if let Some(items) = controller
        .child("toolbarItems")
        .filter(|i| !i.children.is_empty())
    {
        let mut buttons = Vec::new();
        for button in items.children_named("barButtonItem") {
            buttons.push(encoder.bar_button(button)?);
        }
        let array = encoder.graph.array("NSArray", &buttons);
        encoder.graph.set(object, "UIToolbarItems", array);
    }
    if let Some(title) = controller.attr("title") {
        let title = encoder.text(controller, "title", title);
        encoder.graph.set(object, "UITitle", title);
    }
    if let Some(item) = nav_items.get(controller_id) {
        encoder.graph.set(object, "UINavigationItem", *item);
    }
    Ok(())
}

fn controller_view(
    encoder: &mut Encoder<'_>,
    controller: &Element,
    object: Id,
) -> Result<(), String> {
    let controller_id = id(controller)?;
    if let Some(view) = view(controller) {
        let mut external = Vec::new();
        for (i, external_id) in external_ids(controller).iter().enumerate() {
            let key = encoder
                .graph
                .string(&format!("UpstreamPlaceholder-{}", i + 1));
            let value = encoder
                .lookup(external_id)
                .ok_or_else(|| format!("unresolved controller object {external_id}"))?;
            external.push((key, value));
        }
        let table = encoder.graph.dictionary("NSDictionary", &external);
        encoder
            .graph
            .set(object, "UIExternalObjectsTableForViewLoading", table);
        let nib = encoder
            .graph
            .string(&format!("{controller_id}-view-{}", id(view)?));
        encoder.graph.set(object, "UINibName", nib);
    }
    Ok(())
}

fn controller_flags(encoder: &mut Encoder<'_>, controller: &Element, object: Id) {
    if let Some(identifier) = controller.attr("storyboardIdentifier") {
        let identifier = encoder.graph.string(identifier);
        encoder
            .graph
            .set(object, "UIStoryboardIdentifier", identifier);
        if controller.attr("useStoryboardIdentifierAsRestorationIdentifier") == Some("YES") {
            encoder
                .graph
                .set(object, "UIRestorationIdentifier", identifier);
        }
    }
    for (attribute, key, default) in [
        (
            "extendedLayoutIncludesOpaqueBars",
            "UIKeyExtendedLayoutIncludesOpaqueBars",
            false,
        ),
        (
            "automaticallyAdjustsScrollViewInsets",
            "UIKeyAutomaticallyAdjustsScrollViewInsets",
            true,
        ),
        (
            "clearsSelectionOnViewWillAppear",
            "UIClearsSelectionOnViewWillAppear",
            true,
        ),
    ] {
        if let Some(value) = controller.attr(attribute) {
            let value = value == "YES";
            if value != default {
                if attribute == "clearsSelectionOnViewWillAppear" {
                    encoder.graph.set(object, key, value);
                } else {
                    encoder.graph.set(object, key, i64::from(value));
                }
            }
        }
    }
}

fn controller_relationships(
    encoder: &mut Encoder<'_>,
    controller: &Element,
    object: Id,
    bars: &HashMap<String, Id>,
    nav_items: &HashMap<String, Id>,
) -> Result<(), String> {
    let controller_id = id(controller)?;
    let mut child_objects = Vec::new();
    for child_id in children(controller) {
        let child = encoder
            .lookup(child_id)
            .ok_or("missing embedded controller")?;
        encoder.graph.set(child, "UIParentViewController", object);
        child_objects.push(child);
    }
    if !child_objects.is_empty() {
        let array = encoder.graph.array("NSMutableArray", &child_objects);
        encoder.graph.set(object, "UIChildViewControllers", array);
        let array = encoder.graph.array("NSMutableArray", &child_objects);
        if class(controller)? != "UISplitViewController" {
            encoder.graph.set(object, "UIViewControllers", array);
        }
    }
    if controller.name == "tabBarController" {
        let customizable = encoder.graph.array("NSMutableArray", &child_objects);
        encoder
            .graph
            .set(object, "UICustomizableViewControllers", customizable);
        if let Some(bar) = bars.get(controller_id) {
            encoder.graph.set(object, "UITabBar", *bar);
            encoder.graph.set(*bar, "UIDelegate", object);
            let items = child_objects
                .iter()
                .filter_map(|child| match encoder.graph.value(*child, "UITabBarItem") {
                    Some(super::super::nib::Value::Object(item)) => Some(item),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let items = encoder.graph.array("NSArray", &items);
            encoder.graph.replace(*bar, "UIItems", items);
        }
    } else if let Some(bar) = bars.get(controller_id) {
        encoder.graph.set(object, "UINavigationBar", *bar);
        let items: Vec<Id> = children(controller)
            .filter_map(|child| nav_items.get(child).copied())
            .collect();
        if !items.is_empty() {
            let items = encoder.graph.array("NSMutableArray", &items);
            encoder.graph.set(*bar, "UIItems", items);
        }
        encoder.graph.set(*bar, "UIDelegate", object);
    }
    Ok(())
}

fn split_properties(
    encoder: &mut Encoder<'_>,
    controller: &Element,
    object: Id,
) -> Result<(), String> {
    if class(controller)? == "UISplitViewController" {
        if controller.attr("allowDoubleColumnStyle") == Some("YES")
            || children(controller).count() > 2
        {
            encoder.graph.set(
                object,
                "UISplitViewControllerStyle",
                if children(controller).count() > 2 {
                    2_i64
                } else {
                    1
                },
            );
        }
        for (attr, key, values) in [
            (
                "primaryBackgroundStyle",
                "UISplitViewControllerPrimaryBackgroundStyle",
                &[("none", 0_i64), ("sidebar", 1)][..],
            ),
            (
                "preferredDisplayMode",
                "UISplitViewControllerPreferredDisplayMode",
                &[
                    ("automatic", 0),
                    ("secondaryOnly", 1),
                    ("beside", 2),
                    ("over", 3),
                ][..],
            ),
            (
                "behavior",
                "UISplitViewControllerPreferredSplitBehavior",
                &[
                    ("automatic", 0),
                    ("tile", 1),
                    ("overlay", 2),
                    ("displace", 3),
                ][..],
            ),
        ] {
            if let Some(value) = controller.attr(attr) {
                let value = values
                    .iter()
                    .find(|(name, _)| *name == value)
                    .ok_or_else(|| format!("unknown split view {attr}={value}"))?
                    .1;
                encoder.graph.set(object, key, value);
            }
        }
        encoder
            .graph
            .set(object, "UISplitViewControllerPrimaryEdge", 0_i64);
    }
    Ok(())
}

fn controller_class(
    encoder: &mut Encoder<'_>,
    controller: &Element,
    object: Id,
    wrapped: bool,
    options: &Options<'_>,
) -> Result<(), String> {
    if wrapped {
        let custom = custom_class(controller, options)
            .unwrap_or_else(|| class(controller).unwrap().to_string());
        let custom = encoder.graph.string(&custom);
        encoder.graph.set(object, "UIClassName", custom);
        let original = encoder.graph.string(class(controller)?);
        encoder.graph.set(object, "UIOriginalClassName", original);
    }
    Ok(())
}

fn controller_connections(
    encoder: &mut Encoder<'_>,
    document: &Element,
    group: &[&Element],
    placeholders: &HashMap<&str, Id>,
    owner: Id,
    root_object: Id,
) -> Result<Vec<Id>, String> {
    let mut connections = Vec::new();
    for controller in group {
        let object = encoder.lookup(id(controller)?).unwrap();
        for segue in segues(controller) {
            let template = encoder.lookup(id(segue)?).unwrap();
            connections.push(outlet(
                &mut encoder.graph,
                "viewController",
                template,
                object,
            ));
        }
        if controller.name != "viewControllerPlaceholder" {
            connections.push(outlet(
                &mut encoder.graph,
                "storyboard",
                object,
                placeholders[id(controller)?],
            ));
        }
    }
    connections.push(outlet(
        &mut encoder.graph,
        "sceneViewController",
        owner,
        root_object,
    ));
    for controller in group {
        connections.extend(encoder.connections_in_graph(controller, false)?);
        for extra in extras(document, controller) {
            connections.extend(encoder.connections_in_graph(extra, false)?);
        }
    }
    Ok(connections)
}

fn finish_controller(
    mut encoder: Encoder<'_>,
    mut top: Vec<Id>,
    root_object: Id,
    connections: &[Id],
) -> Vec<u8> {
    let mut objects = top.clone();
    top.push(root_object);
    for object in &encoder.objects {
        if !objects.contains(object) {
            objects.push(*object);
        }
    }
    let root = nib_root(
        &mut encoder.graph,
        &top,
        &objects,
        connections,
        &encoder.key_value_pairs,
    );
    encoder.install_accessibility(root);
    encoder.graph.write(root)
}
