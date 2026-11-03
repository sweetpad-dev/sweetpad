use std::collections::{BTreeMap, HashSet};

use sweetpad_lib::xcscheme::Element;

use super::{Options, custom_class, storyboard::Compiled};
use crate::cli::keyed_archive::{self, Plist};

fn id(element: &Element) -> Result<&str, String> {
    element
        .attr("id")
        .ok_or("a WatchKit object has no ID".into())
}

fn controller_name(element: &Element) -> Result<String, String> {
    if let Some(identifier) = element.attr("identifier") {
        Ok(identifier.to_string())
    } else {
        Ok(format!("controller-{}", id(element)?))
    }
}

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Compiled, String> {
    let controllers: Vec<_> = document
        .children_named("scenes")
        .flat_map(|scenes| scenes.children_named("scene"))
        .filter_map(|scene| scene.child("objects"))
        .flat_map(|objects| objects.children.iter())
        .filter(|element| matches!(element.name.as_str(), "controller" | "hostingController"))
        .collect();
    let mut output = BTreeMap::new();
    for controller in &controllers {
        let class =
            custom_class(controller, options).ok_or("a WatchKit controller has no custom class")?;
        let outlets: BTreeMap<_, _> = controller
            .children_named("connections")
            .flat_map(|connections| connections.children_named("outlet"))
            .filter_map(|outlet| Some((outlet.attr("destination")?, outlet.attr("property")?)))
            .collect();
        let mut fields = vec![("controllerClass".into(), Plist::String(class))];
        if let Some(items) = controller.child("items") {
            fields.push((
                "items".into(),
                Plist::Array(
                    items
                        .children
                        .iter()
                        .map(|item| compile_item(item, &outlets, options))
                        .collect::<Result<_, _>>()?,
                ),
            ));
        }
        output.insert(controller_name(controller)?, Plist::Dict(fields));
    }
    let mut root = Vec::new();
    let mut current = document.attr("initialViewController");
    let mut seen = HashSet::new();
    while let Some(current_id) = current {
        if !seen.insert(current_id) {
            return Err("cyclic WatchKit page relationships".into());
        }
        let controller = controllers
            .iter()
            .find(|controller| controller.attr("id") == Some(current_id))
            .ok_or("missing WatchKit page controller")?;
        root.push(Plist::String(controller_name(controller)?));
        current = controller
            .children_named("connections")
            .flat_map(|connections| connections.children_named("segue"))
            .find(|segue| segue.attr("relationship") == Some("nextPage"))
            .and_then(|segue| segue.attr("destination"));
    }
    let root = if root.len() == 1 {
        root.pop().unwrap()
    } else {
        Plist::Array(root)
    };
    let model = Plist::Dict(vec![
        (
            "controllers".into(),
            Plist::Dict(output.into_iter().collect()),
        ),
        ("root".into(), root),
    ]);
    Ok(Compiled {
        nibs: vec![("Interface.plist".to_string(), keyed_archive::write(&model))],
        info: Vec::new(),
    })
}

fn compile_item(
    element: &Element,
    outlets: &BTreeMap<&str, &str>,
    options: &Options<'_>,
) -> Result<Plist, String> {
    let kind = match element.name.as_str() {
        "imageView" => "image",
        "group" => "group",
        "label" => "label",
        "activity" => "activity",
        other => return Err(format!("unsupported WatchKit item <{other}>")),
    };
    let mut fields = vec![("type".into(), Plist::String(kind.into()))];
    for key in ["width", "height"] {
        if let Some(value) = element.attr(key) {
            fields.push((
                key.to_string(),
                Plist::Real(
                    value
                        .parse()
                        .map_err(|_| format!("invalid WatchKit {key}: {value}"))?,
                ),
            ));
        }
    }
    if element
        .attr("alignment")
        .is_some_and(|value| value != "left")
    {
        return Err("non-default WatchKit alignment requires an oracle fixture".into());
    }
    if let Some(property) = outlets.get(id(element)?) {
        fields.push(("property".into(), Plist::String((*property).into())));
    }
    if kind == "image" {
        fields.push(("duration".into(), Plist::Bool(false)));
    }
    if kind == "label" {
        if let Some(text) = element.attr("text") {
            let text = if options.localized {
                Plist::Dict(vec![
                    ("fallbackString".into(), Plist::String(text.into())),
                    (
                        "localizationKey".into(),
                        Plist::String(format!("{}.text", id(element)?)),
                    ),
                ])
            } else {
                Plist::String(text.into())
            };
            fields.push(("text".into(), text));
        }
        fields.push(("fontScale".into(), Plist::Real(1.0)));
    }
    if let Some(items) = element.child("items") {
        fields.push((
            "items".into(),
            Plist::Array(
                items
                    .children
                    .iter()
                    .map(|item| compile_item(item, outlets, options))
                    .collect::<Result<_, _>>()?,
            ),
        ));
    }
    Ok(Plist::Dict(fields))
}
