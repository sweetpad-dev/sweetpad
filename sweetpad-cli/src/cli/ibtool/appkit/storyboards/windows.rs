//! Window scenes retain a window template and its content controller.
use super::super::{Element, Encoder, Id, Options, custom_class, id};
use super::controllers::{element, outlet, scene_object, xib};

fn prepare_window(window: &mut Element) -> Result<String, String> {
    let window_id = id(window)?.to_string();
    window
        .attributes
        .push(("ibtoolStoryboardWindow".into(), "YES".into()));
    if !window
        .children
        .iter()
        .any(|c| c.attr("key") == Some("contentView"))
    {
        let frame = window
            .children_named("rect")
            .find(|r| r.attr("key") == Some("contentRect"))
            .ok_or("a window has no content rect")?;
        let synthetic_id = format!("{window_id}-contentView");
        let view = element(
            "view",
            &[("id", &synthetic_id), ("key", "contentView")],
            vec![
                element(
                    "rect",
                    &[
                        ("key", "frame"),
                        ("x", "0"),
                        ("y", "0"),
                        ("width", frame.attr("width").unwrap_or("0")),
                        ("height", frame.attr("height").unwrap_or("0")),
                    ],
                    vec![],
                ),
                element("autoresizingMask", &[("key", "autoresizingMask")], vec![]),
            ],
        );
        window.children.push(view);
    }
    Ok(window_id)
}

pub(super) fn compile(
    document: &Element,
    controller: &Element,
    options: &Options<'_>,
    name: &str,
    uuid: &str,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut scene = controller.clone();
    let relationship = controller
        .child("connections")
        .into_iter()
        .flat_map(|c| &c.children)
        .filter(|c| c.name == "segue")
        .map(|segue| {
            if segue.attr("kind") != Some("relationship")
                || segue.attr("relationship") != Some("window.shadowedContentViewController")
            {
                return Err("this AppKit window segue isn't compiled yet".to_string());
            }
            segue
                .attr("destination")
                .ok_or_else(|| "a window relationship has no destination".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if relationship.len() > 1 {
        return Err("a window has multiple content-controller relationships".into());
    }
    let mut placeholders = vec![element(
        "storyboardPlaceholder",
        &[
            ("id", "ibtool-storyboard"),
            ("identifier", "NSStoryboardPlaceholder"),
        ],
        vec![],
    )];
    let mut connections = controller.child("connections").map_or(Vec::new(), |c| {
        c.children
            .iter()
            .filter(|c| c.name != "segue")
            .cloned()
            .collect()
    });
    scene.children.retain(|c| c.name != "connections");
    let window = scene
        .children
        .iter_mut()
        .find(|c| c.name == "window")
        .ok_or("a window controller has no window")?;
    let window_id = prepare_window(window)?;
    if let Some(destination) = relationship.first() {
        let source = document
            .children_named("scenes")
            .flat_map(|s| s.children_named("scene"))
            .filter_map(|s| s.child("objects"))
            .flat_map(|o| &o.children)
            .find(|e| e.attr("id") == Some(destination))
            .ok_or("missing window content controller")?;
        if !matches!(
            source.name.as_str(),
            "viewController" | "splitViewController"
        ) {
            return Err(format!(
                "window content <{}> isn't compiled yet",
                source.name
            ));
        }
        let mut content = scene_object(document, source, options)?;
        super::objects::rename_storyboard(&mut content, "ibtool-content-storyboard");
        scene.children.push(content);
        placeholders.push(element(
            "storyboardPlaceholder",
            &[
                ("id", "ibtool-content-storyboard"),
                ("identifier", "NSStoryboardPlaceholder"),
            ],
            vec![],
        ));
    }
    scene.attributes.push(("ibtoolUUID".into(), uuid.into()));
    connections.push(outlet("storyboard", "ibtool-storyboard"));
    connections.push(outlet("window", &window_id));
    scene
        .children
        .push(element("connections", &[], connections));
    let mut objects = vec![scene];
    objects.extend(placeholders);
    let xib = xib(
        document,
        vec![outlet("sceneController", id(controller)?)],
        objects,
    );
    Ok(vec![(
        format!("{name}.nib"),
        super::super::compile(&xib, options)?,
    )])
}

impl Encoder<'_> {
    pub(in super::super) fn storyboard_window_controller(
        &mut self,
        element: &Element,
        object: Id,
    ) -> Result<(), String> {
        if let Some(class) = custom_class(element, self.options) {
            self.string(object, "NSClassName", &class);
            self.string(object, "NSOriginalClassName", "IBNSWindowController");
        }
        let window = element
            .child("window")
            .ok_or("a window controller has no window")?;
        let window = self.encode(window, object)?;
        self.graph.set(object, "IBWindowTemplate", window);
        let content = if let Some(content) = element
            .children
            .iter()
            .find(|e| matches!(e.name.as_str(), "viewController" | "splitViewController"))
        {
            let content = self.encode(content, window)?;
            self.graph
                .set(object, "IBWindowTemplateContentViewController", content);
            content
        } else {
            self.graph.shared("NSNull", vec![])
        };
        if let Some(identifier) = element.attr("storyboardIdentifier") {
            self.string(object, "explicitStoryboardIdentifier", identifier);
        }
        self.storyboard_presentation(element, object)?;
        let connector = self
            .graph
            .object("NSIBUserDefinedRuntimeAttributesConnector");
        self.graph.set(connector, "NSObject", window);
        let values = self.graph.array("NSArray", &[content]);
        self.graph.set(connector, "NSValues", values);
        let key = self.graph.string("contentViewController");
        let keys = self.graph.array("NSArray", &[key]);
        self.graph.set(connector, "NSKeyPaths", keys);
        self.oids.push(connector);
        self.late_connections.push(connector);
        Ok(())
    }
}
