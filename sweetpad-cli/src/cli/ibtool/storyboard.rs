//! A storyboard compiled into a `.storyboardc` folder: for each scene's
//! view controller, a nib of the controller and one of its view, and an
//! `Info.plist` naming the nib each controller loads from and the one the
//! storyboard starts with.

use sweetpad_lib::xcscheme::Element;

use super::Options;
use super::nib::{Graph, Id};

/// A compiled storyboard: its nibs by file name, and its `Info.plist`.
pub(super) struct Compiled {
    pub nibs: Vec<(String, Vec<u8>)>,
    pub info: Vec<u8>,
}

mod controllers;

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Compiled, String> {
    if document.attr("targetRuntime") == Some("MacOSX.Cocoa") {
        return super::appkit::compile_storyboard(document, options);
    }
    controllers::compile(document, options)
}

pub(super) fn proxy(graph: &mut Graph, identifier: &str) -> Id {
    let object = graph.object("UIProxyObject");
    let identifier = graph.string(identifier);
    graph.set(object, "UIProxiedObjectIdentifier", identifier);
    object
}

pub(super) fn outlet(graph: &mut Graph, label: &str, source: Id, destination: Id) -> Id {
    let connection = graph.object("UIRuntimeOutletConnection");
    let label = graph.string(label);
    graph.set(connection, "UILabel", label);
    graph.set(connection, "UISource", source);
    graph.set(connection, "UIDestination", destination);
    connection
}

/// The object `UINib` starts from: the top-level objects, every object,
/// and the connections between them.
pub(super) fn nib_root(
    graph: &mut Graph,
    top: &[Id],
    objects: &[Id],
    connections: &[Id],
    key_value_pairs: &[Id],
) -> Id {
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
    let pairs = graph.array("NSArray", key_value_pairs);
    graph.set(root, "UINibKeyValuePairsKey", pairs);
    graph.set(root, "IBUINibLNEVersionKey", 1_i64);
    root
}
