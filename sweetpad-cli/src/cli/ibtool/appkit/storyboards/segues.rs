//! Button actions target templates supplied by the controller's view-loading table.
use super::super::{Element, Encoder, Id, id};
use super::controllers::{element, outlet, scene_name, uuid};

pub(super) fn definitions(controller: &Element) -> Vec<&Element> {
    fn collect<'a>(node: &'a Element, result: &mut Vec<&'a Element>) {
        if node.name == "segue" && matches!(node.attr("kind"), Some("show" | "embed")) {
            result.push(node);
        }
        for child in &node.children {
            collect(child, result);
        }
    }
    let mut result = Vec::new();
    collect(controller, &mut result);
    result
}
pub(super) fn container_outlet(controller: &Element, identifier: &str) -> Option<Element> {
    if controller.name == "containerView"
        && controller.child("connections").is_some_and(|c| {
            c.children
                .iter()
                .any(|e| e.attr("id") == Some(identifier) && e.attr("kind") == Some("embed"))
        })
    {
        return Some(outlet("containerView", controller.attr("id")?));
    }
    controller
        .children
        .iter()
        .find_map(|c| container_outlet(c, identifier))
}
pub(super) fn actions(view: &mut Element) -> Result<(), String> {
    if view.name == "containerView" {
        for connections in view.children.iter_mut().filter(|c| c.name == "connections") {
            connections
                .children
                .retain(|c| !(c.name == "segue" && c.attr("kind") == Some("embed")));
        }
    }
    if view.name == "segue" {
        if view.attr("kind") != Some("show") {
            return Err(format!(
                "AppKit {} segues aren't compiled yet",
                view.attr("kind").unwrap_or("unknown")
            ));
        }
        let target = id(view)?.to_string();
        *view = element(
            "action",
            &[("target", &target), ("selector", "perform:")],
            vec![],
        );
    }
    for child in &mut view.children {
        actions(child)?;
    }
    Ok(())
}
pub(super) fn templates(
    document: &Element,
    controller: &Element,
    options: &super::super::Options<'_>,
) -> Result<Element, String> {
    let mut templates = Vec::new();
    for segue in definitions(controller) {
        let destination = segue
            .attr("destination")
            .ok_or("a segue has no destination")?;
        let target = document
            .children_named("scenes")
            .flat_map(|s| s.children_named("scene"))
            .filter_map(|s| s.child("objects"))
            .flat_map(|o| &o.children)
            .find(|e| e.attr("id") == Some(destination))
            .ok_or("missing segue destination controller")?;
        let mut template = element(
            if segue.attr("kind") == Some("embed") {
                "storyboardEmbedSegue"
            } else {
                "storyboardShowSegue"
            },
            &[
                ("id", id(segue)?),
                ("destinationName", &scene_name(target)?),
                ("destinationUUID", &uuid(target, options)),
            ],
            vec![],
        );
        if let Some(identifier) = segue.attr("identifier") {
            template
                .attributes
                .push(("identifier".into(), identifier.into()));
        }
        template.children.push(element(
            "connections",
            &[],
            vec![
                outlet("controller", id(controller)?),
                outlet("storyboard", "ibtool-storyboard"),
            ],
        ));
        templates.push(template);
    }
    Ok(element("storyboardSegueTemplates", &[], templates))
}

impl Encoder<'_> {
    pub(in super::super) fn storyboard_segue_template(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        let embed = element.name == "storyboardEmbedSegue";
        self.string(
            object,
            "NSTrigger",
            if embed {
                "IBSegueTriggerViewDidLoad"
            } else {
                "IBSegueTriggerAction"
            },
        );
        self.string(
            object,
            "NSDestinationControllerIdentifier",
            element
                .attr("destinationName")
                .ok_or("a segue has no controller name")?,
        );
        if embed {
            self.graph
                .set(object, "NSContainerView", super::super::Value::Nil);
        } else {
            self.string(
                object,
                "NSUniqueControllerIdentifier",
                element
                    .attr("destinationUUID")
                    .ok_or("a segue has no controller UUID")?,
            );
        }
        if let Some(identifier) = element.attr("identifier") {
            self.string(object, "NSIdentifier", identifier);
        }
        Ok(())
    }
}
