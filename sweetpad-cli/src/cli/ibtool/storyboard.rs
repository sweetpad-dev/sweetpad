//! A storyboard compiled into a `.storyboardc` folder: for each scene's
//! view controller, a nib of the controller and one of its view, and an
//! `Info.plist` naming the nib each controller loads from and the one the
//! storyboard starts with.

use sweetpad_lib::xcscheme::Element;

use super::nib::{Graph, Id};
use super::uikit::Encoder;
use crate::cli::keyed_archive::{self, Plist};

/// A compiled storyboard: its nibs by file name, and its `Info.plist`.
pub(super) struct Compiled {
    pub nibs: Vec<(String, Vec<u8>)>,
    pub info: Vec<u8>,
}

/// The view controllers SweetPad compiles, and those it names in an error.
const CONTROLLERS: &[&str] = &[
    "viewController",
    "navigationController",
    "tableViewController",
    "tabBarController",
    "collectionViewController",
    "splitViewController",
    "pageViewController",
    "hostingController",
];

pub(super) fn compile(document: &Element) -> Result<Compiled, String> {
    if document.attr("type") != Some("com.apple.InterfaceBuilder3.CocoaTouch.Storyboard.XIB") {
        return Err("this isn't an iOS storyboard".to_string());
    }
    let mut nibs = Vec::new();
    let mut identifiers = Vec::new();
    let mut entry = None;
    for scene in document
        .children_named("scenes")
        .flat_map(|scenes| scenes.children_named("scene"))
    {
        let Some(objects) = scene.child("objects") else {
            continue;
        };
        for object in &objects.children {
            match object.name.as_str() {
                "viewController" => {
                    let id = object.attr("id").ok_or("a view controller has no id")?;
                    let (controller, view) = view_controller(document, object)?;
                    let name = format!("UIViewController-{id}");
                    let identifier = object
                        .attr("storyboardIdentifier")
                        .map_or_else(|| name.clone(), str::to_string);
                    if document.attr("initialViewController") == Some(id) {
                        entry = Some(identifier.clone());
                    }
                    identifiers.push((identifier, name.clone()));
                    nibs.push((format!("{name}.nib"), controller));
                    if let Some((view_name, view)) = view {
                        nibs.push((format!("{view_name}.nib"), view));
                    }
                }
                "placeholder" => {}
                name if CONTROLLERS.contains(&name) => {
                    return Err(format!(
                        "<{name}> isn't a view controller SweetPad's ibtool compiles yet"
                    ));
                }
                name => {
                    return Err(format!(
                        "<{name}> in a scene isn't something SweetPad's ibtool compiles yet"
                    ));
                }
            }
        }
    }
    let mut info = Vec::new();
    if let Some(entry) = entry {
        info.push((
            "UIStoryboardDesignatedEntryPointIdentifier".to_string(),
            Plist::String(entry),
        ));
    }
    info.push(("UIStoryboardVersion".to_string(), Plist::Int(1)));
    info.push((
        "UIViewControllerIdentifiersToNibNames".to_string(),
        Plist::Dict(
            identifiers
                .into_iter()
                .map(|(identifier, nib)| (identifier, Plist::String(nib)))
                .collect(),
        ),
    ));
    Ok(Compiled {
        nibs,
        info: keyed_archive::write(&Plist::Dict(info)),
    })
}

/// The nib of a plain `UIViewController`, and its view's nib by name.
#[allow(clippy::type_complexity)]
fn view_controller(
    document: &Element,
    element: &Element,
) -> Result<(Vec<u8>, Option<(String, Vec<u8>)>), String> {
    let id = element.attr("id").unwrap_or_default();
    for (attribute, _) in &element.attributes {
        if !matches!(
            attribute.as_str(),
            "id" | "sceneMemberID" | "storyboardIdentifier"
        ) {
            return Err(format!(
                "the view controller {id}'s {attribute} isn't something SweetPad's ibtool compiles yet"
            ));
        }
    }
    for child in &element.children {
        if !matches!(
            child.name.as_str(),
            "view" | "extendedEdge" | "size" | "nil"
        ) {
            return Err(format!(
                "<{}> in the view controller {id} isn't something SweetPad's ibtool compiles yet",
                child.name
            ));
        }
    }
    let view_element = element
        .children_named("view")
        .find(|view| view.attr("key") == Some("view"));
    let view = match view_element {
        Some(view_element) => {
            let view_id = view_element.attr("id").ok_or("a view has no id")?;
            let name = format!("{id}-view-{view_id}");
            Some((name, view_nib(document, view_element)?))
        }
        None => None,
    };

    let mut graph = Graph::default();
    let owner = proxy(&mut graph, "IBFilesOwner");
    let responder = proxy(&mut graph, "IBFirstResponder");
    let placeholder = proxy(&mut graph, "UIStoryboardPlaceholder");
    let controller = graph.object("UIClassSwapper");
    let table = graph.dictionary("NSDictionary", &[]);
    graph.set(controller, "UIExternalObjectsTableForViewLoading", table);
    if let Some((name, _)) = &view {
        let name = graph.string(name);
        graph.set(controller, "UINibName", name);
    }
    let class = graph.string("UIViewController");
    graph.set(controller, "UIClassName", class);
    graph.set(controller, "UIOriginalClassName", class);
    let top = [owner, responder, placeholder, controller];
    let connections = [
        outlet(&mut graph, "storyboard", controller, placeholder),
        outlet(&mut graph, "sceneViewController", owner, controller),
    ];
    let root = nib_root(&mut graph, &top, &top, &connections);
    Ok((graph.write(root), view))
}

fn view_nib(document: &Element, view: &Element) -> Result<Vec<u8>, String> {
    let mut encoder = Encoder::new(document);
    let owner = proxy(&mut encoder.graph, "IBFilesOwner");
    let responder = proxy(&mut encoder.graph, "IBFirstResponder");
    let root_view = encoder.root_view(view)?;
    let mut objects = vec![owner, responder];
    objects.extend(encoder.objects.iter().copied());
    let graph = &mut encoder.graph;
    let connections = [outlet(graph, "view", owner, root_view)];
    let root = nib_root(
        graph,
        &[owner, responder, root_view],
        &objects,
        &connections,
    );
    Ok(encoder.graph.write(root))
}

fn proxy(graph: &mut Graph, identifier: &str) -> Id {
    let object = graph.object("UIProxyObject");
    let identifier = graph.string(identifier);
    graph.set(object, "UIProxiedObjectIdentifier", identifier);
    object
}

fn outlet(graph: &mut Graph, label: &str, source: Id, destination: Id) -> Id {
    let connection = graph.object("UIRuntimeOutletConnection");
    let label = graph.string(label);
    graph.set(connection, "UILabel", label);
    graph.set(connection, "UISource", source);
    graph.set(connection, "UIDestination", destination);
    connection
}

/// The object `UINib` starts from: the top-level objects, every object,
/// and the connections between them.
fn nib_root(graph: &mut Graph, top: &[Id], objects: &[Id], connections: &[Id]) -> Id {
    let root = graph.object("NSObject");
    let top = graph.array("NSArray", top);
    let objects = graph.array("NSArray", objects);
    let connections = graph.array("NSArray", connections);
    let empty = graph.array("NSArray", &[]);
    graph.set(root, "UINibTopLevelObjectsKey", top);
    graph.set(root, "UINibObjectsKey", objects);
    graph.set(root, "UINibConnectionsKey", connections);
    graph.set(root, "UINibVisibleWindowsKey", empty);
    graph.set(root, "UINibAccessibilityConfigurationsKey", empty);
    graph.set(root, "UINibTraitStorageListsKey", empty);
    graph.set(root, "UINibKeyValuePairsKey", empty);
    graph.set(root, "IBUINibLNEVersionKey", 1_i64);
    root
}
