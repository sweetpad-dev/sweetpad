//! Controller scenes load their views from separate nibs.
use super::super::{Element, Encoder, Id, Options, custom_class, id};
use crate::cli::{
    ibtool::storyboard::Compiled,
    keyed_archive::{self, Plist},
};
use std::fmt::Write as _;

pub(super) fn element(name: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
    Element {
        name: name.into(),
        attributes: attrs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        children,
        text: String::new(),
    }
}
pub(super) fn outlet(property: &str, destination: &str) -> Element {
    element(
        "outlet",
        &[("property", property), ("destination", destination)],
        vec![],
    )
}
pub(super) fn xib(
    document: &Element,
    owner_connections: Vec<Element>,
    mut objects: Vec<Element>,
) -> Element {
    let mut result = document.clone();
    result.children.retain(|c| c.name != "scenes");
    let owner = element(
        "customObject",
        &[("id", "-2")],
        vec![element("connections", &[], owner_connections)],
    );
    objects.insert(0, owner);
    objects.push(element("customObject", &[("id", "-3")], vec![]));
    result.children.push(element("objects", &[], objects));
    result
}
pub(super) fn uuid(controller: &Element, options: &Options<'_>) -> String {
    let mut bytes =
        sweetpad_lib::xcode_hash::sha256(format!("{}:{controller:?}", options.module).as_bytes());
    bytes[6] = (bytes[6] & 15) | 0x50;
    bytes[8] = (bytes[8] & 63) | 0x80;
    let mut hex = String::new();
    for byte in &bytes[..16] {
        write!(hex, "{byte:02X}").unwrap();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
fn filename_component(value: &str) -> Result<(), String> {
    if value.contains(['/', '\0']) {
        return Err(format!("invalid storyboard filename component {value:?}"));
    }
    Ok(())
}

fn controller_nibs(
    document: &Element,
    objects: &Element,
    controller: &Element,
    options: &Options<'_>,
    name: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let identifier = id(controller)?;
    let mut nibs = Vec::new();
    let view = super::objects::view(controller)?;
    let view_id = id(view)?.to_string();
    filename_component(id(view)?)?;
    let view_name = format!("{identifier}-view-{}", id(view)?);
    let responders = objects
        .children
        .iter()
        .filter(|e| e.attr("sceneMemberID") == Some("firstResponder"))
        .filter_map(|e| e.attr("id"))
        .collect::<Vec<_>>();
    let mut view = view.clone();
    super::segues::actions(&mut view)?;
    let mut view_objects = vec![view];
    view_objects.extend(super::objects::placeholders(document, controller)?);
    let mut view_xib = xib(document, vec![outlet("view", &view_id)], view_objects);
    super::rename_responders(&mut view_xib, &responders);
    nibs.push((
        format!("{view_name}.nib"),
        super::super::compile(&view_xib, options)?,
    ));
    if embedded_unnamed(document, controller)? {
        return Ok(nibs);
    }
    let scene_controller = scene_object(document, controller, options)?;
    let placeholder = element(
        "storyboardPlaceholder",
        &[
            ("id", "ibtool-storyboard"),
            ("identifier", "NSStoryboardPlaceholder"),
        ],
        vec![],
    );
    let scene_xib = xib(
        document,
        vec![outlet("sceneController", identifier)],
        vec![scene_controller, placeholder],
    );
    nibs.push((
        format!("{name}.nib"),
        super::super::compile(&scene_xib, options)?,
    ));
    Ok(nibs)
}

fn embedded_unnamed(document: &Element, controller: &Element) -> Result<bool, String> {
    if controller
        .attr("storyboardIdentifier")
        .is_some_and(|name| !name.is_empty())
    {
        return Ok(false);
    }
    let identifier = id(controller)?;
    Ok(document
        .children_named("scenes")
        .flat_map(|s| s.children_named("scene"))
        .filter_map(|s| s.child("objects"))
        .flat_map(|o| &o.children)
        .filter(|e| matches!(e.name.as_str(), "windowController" | "splitViewController"))
        .filter_map(|e| e.child("connections"))
        .flat_map(|c| &c.children)
        .any(|c| {
            c.name == "segue"
                && c.attr("kind") == Some("relationship")
                && c.attr("destination") == Some(identifier)
        }))
}

fn incoming(node: &Element, identifier: &str) -> bool {
    node.name == "segue"
        && node.attr("destination") == Some(identifier)
        && matches!(node.attr("kind"), Some("relationship" | "embed"))
        || node
            .children
            .iter()
            .any(|child| incoming(child, identifier))
}
pub(super) fn embedded(document: &Element, controller: &Element) -> Result<bool, String> {
    if controller
        .attr("storyboardIdentifier")
        .is_some_and(|name| !name.is_empty())
    {
        return Ok(false);
    }
    Ok(incoming(document, id(controller)?))
}

fn presented(node: &Element, identifier: &str) -> bool {
    (node.name == "segue"
        && node.attr("destination") == Some(identifier)
        && matches!(node.attr("kind"), Some("show" | "presentation")))
        || node
            .children
            .iter()
            .any(|child| presented(child, identifier))
}

pub(super) fn scene_object(
    document: &Element,
    controller: &Element,
    options: &Options<'_>,
) -> Result<Element, String> {
    if controller.name == "splitViewController" {
        return super::splits::object(document, controller, options);
    }
    let view = super::objects::view(controller)?;
    let view_name = format!("{}-view-{}", id(controller)?, id(view)?);
    let uuid = uuid(controller, options);
    let mut scene_controller = controller.clone();
    if controller.attr("storyboardIdentifier").is_none() && presented(document, id(controller)?) {
        scene_controller
            .attributes
            .push(("ibtoolPresented".into(), "YES".into()));
    }

    scene_controller
        .children
        .retain(|e| e.attr("key") != Some("view") && e.name != "view" && e.name != "connections");
    for (key, value) in [("ibtoolNibName", view_name), ("ibtoolUUID", uuid)] {
        scene_controller.attributes.push((key.into(), value));
    }
    if !super::objects::connections(controller, true)?.is_empty() {
        scene_controller
            .attributes
            .push(("ibtoolHasViewOutlets".into(), "YES".into()));
    }
    let mut connections = super::objects::connections(controller, false)?;
    connections.push(outlet("storyboard", "ibtool-storyboard"));
    scene_controller
        .children
        .push(element("connections", &[], connections));
    scene_controller.children.push(element(
        "storyboardExternalObjects",
        &[],
        super::objects::external(document, controller)?,
    ));
    scene_controller.children.push(element(
        "storyboardSceneObjects",
        &[],
        super::objects::extras(document, controller)?
            .into_iter()
            .cloned()
            .collect(),
    ));
    scene_controller
        .children
        .push(super::segues::templates(document, controller, options)?);
    if embedded(document, controller)? {
        scene_controller
            .attributes
            .push(("ibtoolEmbedded".into(), "YES".into()));
    }
    Ok(scene_controller)
}

pub(super) fn scene_name(controller: &Element) -> Result<String, String> {
    let identifier = id(controller)?;
    filename_component(identifier)?;
    let name = controller
        .attr("storyboardIdentifier")
        .filter(|name| !name.is_empty())
        .map_or_else(
            || {
                format!(
                    "{}-{identifier}",
                    match controller.name.as_str() {
                        "windowController" => "NSWindowController",
                        "splitViewController" => "NSSplitViewController",
                        _ => "NSViewController",
                    }
                )
            },
            str::to_string,
        );
    filename_component(&name)?;
    Ok(name)
}

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Compiled, String> {
    let scenes = document
        .children_named("scenes")
        .flat_map(|s| s.children_named("scene"));
    let mut nibs = Vec::new();
    let mut names = Vec::new();
    let mut uuids = Vec::new();
    let mut entry = None;
    for scene in scenes {
        let objects = scene
            .child("objects")
            .ok_or("an AppKit scene has no objects")?;
        let controller = objects
            .children
            .iter()
            .find(|e| e.attr("sceneMemberID") == Some("viewController"))
            .ok_or("an AppKit scene has no controller")?;
        if controller.name == "application" {
            let mut menu_document = document.clone();
            menu_document
                .children
                .retain(|child| child.name != "scenes");
            menu_document
                .children
                .push(element("scenes", &[], vec![scene.clone()]));
            nibs.extend(super::compile_menu(&menu_document, options)?.nibs);
            names.push(("MainMenu".into(), Plist::String("MainMenu".into())));
            continue;
        }
        if !matches!(
            controller.name.as_str(),
            "viewController" | "windowController" | "splitViewController"
        ) {
            return Err(format!(
                "AppKit storyboard <{}> scenes aren't compiled yet",
                controller.name
            ));
        }
        let identifier = id(controller)?;
        let name = scene_name(controller)?;
        if names.iter().any(|(key, _)| key == &name) {
            return Err(format!("duplicate storyboard identifier {name}"));
        }
        let uuid = uuid(controller, options);
        if controller.name == "windowController" {
            nibs.extend(super::windows::compile(
                document, controller, options, &name, &uuid,
            )?);
        } else if controller.name == "splitViewController" {
            if !embedded_unnamed(document, controller)? {
                nibs.extend(super::splits::compile(
                    document, controller, options, &name,
                )?);
            }
        } else {
            nibs.extend(controller_nibs(
                document, objects, controller, options, &name,
            )?);
        }
        if document.attr("initialViewController") == Some(identifier) {
            entry = Some(name.clone());
        }
        if controller.name != "windowController" && embedded_unnamed(document, controller)? {
            continue;
        }
        names.push((name.clone(), Plist::String(name.clone())));
        uuids.push((name, Plist::String(uuid)));
    }
    let mut info = vec![
        (
            "NSStoryboardMainMenu".into(),
            Plist::String("MainMenu".into()),
        ),
        (
            "NSViewControllerIdentifiersToNibNames".into(),
            Plist::Dict(names),
        ),
        (
            "NSViewControllerIdentifiersToUUIDs".into(),
            Plist::Dict(uuids),
        ),
        ("NSStoryboardVersion".into(), Plist::Int(1)),
    ];
    if let Some(entry) = entry {
        info.push((
            "NSStoryboardDesignatedEntryPointIdentifier".into(),
            Plist::String(entry),
        ));
    }
    Ok(Compiled {
        nibs,
        info: keyed_archive::write(&Plist::Dict(info)),
    })
}

impl Encoder<'_> {
    pub(in super::super) fn storyboard_view_controller(
        &mut self,
        element: &Element,
        object: Id,
        parent: Id,
    ) -> Result<(), String> {
        if let Some(extras) = element.child("storyboardSceneObjects") {
            for extra in &extras.children {
                self.encode(extra, parent)?;
            }
        }
        let name = element
            .attr("ibtoolNibName")
            .ok_or("standalone AppKit view controllers aren't compiled yet")?;
        if self.graph.class(object) == "NSClassSwapper" {
            self.string(
                object,
                "NSClassName",
                &custom_class(element, self.options).unwrap_or_else(|| "NSViewController".into()),
            );
            self.string(object, "NSOriginalClassName", "NSViewController");
        }
        self.string(object, "NSNibName", name);
        if let Some(identifier) = element.attr("storyboardIdentifier") {
            self.string(object, "NSStoryboardIdentifier", identifier);
        }
        if let Some(title) = element.attr("title") {
            let title = self.text(element, "title", title);
            self.graph.set(object, "NSTitle", title);
        }
        let entries =
            element
                .child("storyboardExternalObjects")
                .map_or(Ok(Vec::new()), |entries| {
                    entries
                        .children
                        .iter()
                        .map(|entry| {
                            Ok::<_, String>((
                                self.graph.string(
                                    entry
                                        .attr("identifier")
                                        .ok_or("missing external object name")?,
                                ),
                                self.lookup(
                                    entry
                                        .attr("destination")
                                        .ok_or("missing external object destination")?,
                                )?,
                            ))
                        })
                        .collect()
                })?;
        if let Some(templates) = element.child("storyboardSegueTemplates")
            && !templates.children.is_empty()
        {
            let ids = templates
                .children
                .iter()
                .map(|e| self.encode(e, object))
                .collect::<Result<Vec<_>, _>>()?;
            let templates = self.graph.array("NSArray", &ids);
            self.graph
                .set(object, "NSStoryboardSegueTemplates", templates);
        }
        let external = self.graph.dictionary("NSDictionary", &entries);
        self.graph
            .set(object, "NSExternalObjectsTableForViewLoading", external);
        self.storyboard_presentation(element, object)?;
        if element.attr("ibtoolEmbedded") != Some("YES")
            && element.attr("ibtoolPresented") != Some("YES")
            && (element.attr("customClass").is_none()
                || element.attr("ibtoolHasViewOutlets") == Some("YES"))
        {
            self.graph.set(
                object,
                "connectionsRequireClassSwapperForStoryboardCompilation",
                true,
            );
        }
        Ok(())
    }
    pub(in super::super) fn storyboard_presentation(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        let presentation = match element.attr("showSeguePresentationStyle") {
            None | Some("multiple") => 0_i64,
            Some("single") => 1,
            Some(other) => {
                return Err(format!("unsupported show segue presentation style {other}"));
            }
        };
        self.graph
            .set(object, "showSeguePresentationStyle", presentation);
        self.string(
            object,
            "uniqueIdentifierForStoryboardCompilation",
            element
                .attr("ibtoolUUID")
                .ok_or("a storyboard controller has no UUID")?,
        );
        if presentation == 1 {
            let key = self.graph.string("NSSingleInstancePresentationIdentifier");
            let uuid = self.graph.string(
                element
                    .attr("ibtoolUUID")
                    .ok_or("a storyboard controller has no UUID")?,
            );
            let options = self.graph.dictionary("NSDictionary", &[(key, uuid)]);
            self.graph
                .set(object, "NSStoryboardSegueDestinationOptions", options);
        }
        Ok(())
    }

    pub(in super::super) fn storyboard_placeholder(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        self.string(
            object,
            "NSExternalObjectPlaceholderIdentifier",
            element
                .attr("identifier")
                .ok_or("a storyboard placeholder has no identifier")?,
        );
        Ok(())
    }
}
