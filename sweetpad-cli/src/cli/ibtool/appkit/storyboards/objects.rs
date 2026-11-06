//! Scene models stay with the controller; view archives refer to them by name.
use super::super::{Element, id};
use super::controllers::element;

pub(super) fn rename_storyboard(node: &mut Element, identifier: &str) {
    for (key, value) in &mut node.attributes {
        if key == "destination" && value == "ibtool-storyboard" {
            *value = identifier.into();
        }
    }
    for child in &mut node.children {
        rename_storyboard(child, identifier);
    }
}

pub(super) fn view(controller: &Element) -> Result<&Element, String> {
    controller
        .children
        .iter()
        .find(|child| child.attr("key") == Some("view"))
        .or_else(|| controller.child("view"))
        .ok_or_else(|| "an AppKit view controller has no view".into())
}
fn contains(element: &Element, identifier: &str) -> bool {
    element.attr("id") == Some(identifier)
        || element.children.iter().any(|e| contains(e, identifier))
}
fn references(element: &Element, identifier: &str) -> bool {
    element.attr("target") == Some(identifier)
        || element.attr("destination") == Some(identifier)
        || element.children.iter().any(|e| references(e, identifier))
}
pub(super) fn connections(controller: &Element, for_view: bool) -> Result<Vec<Element>, String> {
    let view = view(controller)?;
    Ok(controller.child("connections").map_or(Vec::new(), |c| {
        c.children
            .iter()
            .filter(|c| {
                c.name != "segue"
                    && (c
                        .attr("destination")
                        .is_some_and(|destination| contains(view, destination))
                        == for_view)
            })
            .cloned()
            .collect()
    }))
}
pub(super) fn extras<'a>(
    document: &'a Element,
    controller: &Element,
) -> Result<Vec<&'a Element>, String> {
    let identifier = id(controller)?;
    let objects = document
        .children_named("scenes")
        .flat_map(|s| s.children_named("scene"))
        .filter_map(|s| s.child("objects"))
        .find(|objects| {
            objects
                .children
                .iter()
                .any(|e| e.attr("id") == Some(identifier))
        })
        .ok_or("missing storyboard scene")?;
    Ok(objects
        .children
        .iter()
        .filter(|e| {
            e.attr("id") != Some(identifier) && e.attr("sceneMemberID") != Some("firstResponder")
        })
        .collect())
}
fn visit(node: &Element, extras: &[&Element], external: &mut Vec<String>) {
    for attribute in ["destination", "target"] {
        if let Some(destination) = node.attr(attribute)
            && extras
                .iter()
                .any(|extra| extra.attr("id") == Some(destination))
            && !external.iter().any(|id| id == destination)
        {
            external.push(destination.into());
        }
    }
    for child in &node.children {
        visit(child, extras, external);
    }
}
pub(super) fn external(document: &Element, controller: &Element) -> Result<Vec<Element>, String> {
    let mut external = Vec::new();
    if !connections(controller, true)?.is_empty() || references(view(controller)?, id(controller)?)
    {
        external.push(id(controller)?.to_string());
    }
    visit(
        view(controller)?,
        &extras(document, controller)?,
        &mut external,
    );
    for segue in super::segues::definitions(controller) {
        external.push(id(segue)?.to_string());
    }
    Ok(external
        .iter()
        .enumerate()
        .map(|(i, id)| {
            element(
                "externalObject",
                &[
                    ("destination", id),
                    ("identifier", &format!("UpstreamPlaceholder-{}", i + 1)),
                ],
                vec![],
            )
        })
        .collect())
}
pub(super) fn placeholders(
    document: &Element,
    controller: &Element,
) -> Result<Vec<Element>, String> {
    external(document, controller)?
        .iter()
        .map(|entry| {
            let destination = entry.attr("destination").unwrap();
            let connections = if destination == id(controller)? {
                vec![element("connections", &[], connections(controller, true)?)]
            } else if let Some(connection) =
                super::segues::container_outlet(controller, destination)
            {
                vec![element("connections", &[], vec![connection])]
            } else {
                vec![]
            };
            Ok(element(
                "storyboardPlaceholder",
                &[
                    ("id", destination),
                    ("identifier", entry.attr("identifier").unwrap()),
                ],
                connections,
            ))
        })
        .collect()
}
