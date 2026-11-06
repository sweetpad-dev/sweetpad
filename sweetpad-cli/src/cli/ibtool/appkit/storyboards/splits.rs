//! Split controllers connect panes before applying their ordered item list.
use super::super::{Element, Encoder, Id, Options, Value, custom_class, id, number, yes};
use super::controllers::{element, outlet, scene_object, xib};

pub(super) fn object(
    document: &Element,
    source: &Element,
    options: &Options<'_>,
) -> Result<Element, String> {
    let mut controller = source.clone();
    let links = source
        .child("connections")
        .into_iter()
        .flat_map(|c| &c.children)
        .filter(|c| c.name == "segue" && c.attr("relationship") == Some("splitItems"))
        .collect::<Vec<_>>();
    let items = controller
        .children
        .iter_mut()
        .find(|c| c.name == "splitViewItems")
        .ok_or("a split controller has no items")?;
    if items.children.len() != links.len() {
        return Err("split items and controller relationships differ in length".into());
    }
    let mut children = Vec::new();
    for (item, link) in items.children.iter_mut().zip(links) {
        let destination = link
            .attr("destination")
            .ok_or("a split relationship has no destination")?;
        let target = document
            .children_named("scenes")
            .flat_map(|s| s.children_named("scene"))
            .filter_map(|s| s.child("objects"))
            .flat_map(|o| &o.children)
            .find(|e| e.attr("id") == Some(destination))
            .ok_or("missing split child controller")?;
        let mut child = scene_object(document, target, options)?;
        let placeholder_id = format!("{destination}-storyboard");
        super::objects::rename_storyboard(&mut child, &placeholder_id);
        let mut connection = outlet("viewController", destination);
        connection
            .attributes
            .push(("ibtoolRelationship".into(), "YES".into()));
        item.children
            .push(element("connections", &[], vec![connection]));
        children.push(child);
        children.push(element(
            "storyboardPlaceholder",
            &[
                ("id", &placeholder_id),
                ("identifier", "NSStoryboardPlaceholder"),
            ],
            vec![],
        ));
    }
    controller
        .children
        .push(element("storyboardSplitChildren", &[], children));
    controller.children.retain(|c| c.name != "connections");
    let mut connections = source.child("connections").map_or(Vec::new(), |c| {
        c.children
            .iter()
            .filter(|c| c.name != "segue")
            .cloned()
            .collect()
    });
    connections.push(outlet("storyboard", "ibtool-storyboard"));
    controller
        .children
        .push(element("connections", &[], connections));
    controller.attributes.push((
        "ibtoolUUID".into(),
        super::controllers::uuid(source, options),
    ));
    if super::controllers::embedded(document, source)? {
        controller
            .attributes
            .push(("ibtoolEmbedded".into(), "YES".into()));
    }
    Ok(controller)
}

pub(super) fn compile(
    document: &Element,
    source: &Element,
    options: &Options<'_>,
    name: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let controller = object(document, source, options)?;
    let placeholder = element(
        "storyboardPlaceholder",
        &[
            ("id", "ibtool-storyboard"),
            ("identifier", "NSStoryboardPlaceholder"),
        ],
        vec![],
    );
    let xib = xib(
        document,
        vec![outlet("sceneController", id(source)?)],
        vec![controller, placeholder],
    );
    Ok(vec![(
        format!("{name}.nib"),
        super::super::compile(&xib, options)?,
    )])
}
impl Encoder<'_> {
    pub(in super::super) fn split_controller(
        &mut self,
        element: &Element,
        object: Id,
        parent: Id,
    ) -> Result<(), String> {
        if self.graph.class(object) == "NSClassSwapper" {
            self.string(
                object,
                "NSClassName",
                &custom_class(element, self.options)
                    .unwrap_or_else(|| "NSSplitViewController".into()),
            );
            self.string(object, "NSOriginalClassName", "NSSplitViewController");
        }
        self.storyboard_presentation(element, object)?;
        if element.attr("ibtoolEmbedded") != Some("YES") {
            self.graph.set(
                object,
                "connectionsRequireClassSwapperForStoryboardCompilation",
                true,
            );
        }
        if let Some(children) = element.child("storyboardSplitChildren") {
            for child in &children.children {
                self.encode(child, parent)?;
            }
        }
        let items = element
            .child("splitViewItems")
            .ok_or("a split controller has no items")?
            .children
            .iter()
            .map(|item| self.encode(item, object))
            .collect::<Result<Vec<_>, _>>()?;
        let items = self.graph.array("NSArray", &items);
        self.graph
            .set(object, "privateRelationshipSegueTrackingItems", items);
        self.graph.set(object, "NSIsVertical", false);
        let view = element
            .child("splitView")
            .ok_or("a split controller has no split view")?;
        self.root_views.insert(self.object(view)?);
        let view = self.encode(view, object)?;
        self.graph.set(object, "NSSplitView", view);
        self.runtime_attributes(object, &[("splitViewItems", items)]);
        Ok(())
    }
    pub(in super::super) fn split_item(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        let behavior = match element.attr("behavior") {
            None | Some("default") => 0_i64,
            Some("sidebar") => 1,
            Some("contentList") => 2,
            Some(other) => return Err(format!("unsupported split item behavior {other}")),
        };
        if behavior != 0 {
            self.graph.set(object, "NSSplitViewItemBehavior", behavior);
        }
        self.graph
            .set(object, "NSSplitViewItemViewController", Value::Nil);
        let collapsed = self.graph.shared(
            "NSNumber",
            vec![("NS.boolval", Value::Bool(yes(element, "collapsed")))],
        );
        let can_collapse = self.graph.shared(
            "NSNumber",
            vec![("NS.boolval", Value::Bool(yes(element, "canCollapse")))],
        );
        let priority = self.graph.shared(
            "NSNumber",
            vec![(
                "NS.dblval",
                Value::Double(number(element, "holdingPriority", 250.0)?),
            )],
        );
        self.runtime_attributes(
            object,
            &[
                ("collapsed", collapsed),
                ("canCollapse", can_collapse),
                ("holdingPriority", priority),
            ],
        );
        Ok(())
    }
    pub(in super::super) fn runtime_attributes(&mut self, object: Id, entries: &[(&str, Id)]) {
        let connector = self
            .graph
            .object("NSIBUserDefinedRuntimeAttributesConnector");
        self.graph.set(connector, "NSObject", object);
        let values = self.graph.array(
            "NSArray",
            &entries.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
        );
        let keys = entries
            .iter()
            .map(|(key, _)| self.graph.string(key))
            .collect::<Vec<_>>();
        let keys = self.graph.array("NSArray", &keys);
        self.graph.set(connector, "NSValues", values);
        self.graph.set(connector, "NSKeyPaths", keys);
        self.oids.push(connector);
        self.late_connections.push(connector);
    }
    pub(in super::super) fn split_view(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        self.graph
            .set(object, "NSIsVertical", yes(element, "vertical"));
        let style = match element.attr("dividerStyle") {
            None | Some("thick") => 1_i64,
            Some("thin") => 2,
            Some("paneSplitter") => 3,
            Some(other) => return Err(format!("unsupported split divider style {other}")),
        };
        self.graph.set(object, "NSDividerStyle", style);
        Ok(())
    }
}
