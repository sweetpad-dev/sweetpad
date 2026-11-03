//! Mac storyboard packaging. Menu-only storyboards contain one MainMenu nib.
use super::{Element, Options};
use crate::cli::{
    ibtool::storyboard::Compiled,
    keyed_archive::{self, Plist},
};
mod controllers;
mod objects;
mod segues;
mod splits;
mod windows;

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Compiled, String> {
    if document
        .children_named("scenes")
        .flat_map(|s| s.children_named("scene"))
        .filter_map(|s| s.child("objects"))
        .any(|o| {
            o.children.iter().any(|e| {
                e.name != "application" && e.attr("sceneMemberID") == Some("viewController")
            })
        })
    {
        return controllers::compile(document, options);
    }
    compile_menu(document, options)
}

fn compile_menu(document: &Element, options: &Options<'_>) -> Result<Compiled, String> {
    let objects = document
        .children_named("scenes")
        .flat_map(|scenes| scenes.children_named("scene"))
        .filter_map(|scene| scene.child("objects"))
        .flat_map(|objects| objects.children.iter())
        .collect::<Vec<_>>();
    if !objects.iter().any(|object| object.name == "application")
        || objects.iter().any(|object| {
            !matches!(
                object.name.as_str(),
                "application" | "customObject" | "menu"
            )
        })
    {
        return Err("AppKit storyboard controller scenes aren't compiled yet".into());
    }
    let responders = objects
        .iter()
        .filter(|object| object.attr("sceneMemberID") == Some("firstResponder"))
        .filter_map(|object| object.attr("id"))
        .collect::<Vec<_>>();
    let mut xib = document.clone();
    xib.attributes.retain(|(key, _)| key != "type");
    xib.attributes.push((
        "type".into(),
        "com.apple.InterfaceBuilder3.Cocoa.XIB".into(),
    ));
    xib.children.retain(|child| child.name != "scenes");
    let mut elements = Element {
        name: "objects".into(),
        attributes: Vec::new(),
        children: Vec::new(),
        text: String::new(),
    };
    for identifier in ["-2", "-3"] {
        elements.children.push(Element {
            name: "customObject".into(),
            attributes: vec![("id".into(), identifier.into())],
            children: Vec::new(),
            text: String::new(),
        });
    }
    for object in objects {
        let mut object = object.clone();
        rename_responders(&mut object, &responders);
        elements.children.push(object);
    }
    xib.children.push(elements);
    let nib = super::compile(&xib, options)?;
    let info = keyed_archive::write(&Plist::Dict(vec![
        (
            "NSStoryboardMainMenu".into(),
            Plist::String("MainMenu".into()),
        ),
        (
            "NSViewControllerIdentifiersToNibNames".into(),
            Plist::Dict(vec![("MainMenu".into(), Plist::String("MainMenu".into()))]),
        ),
        ("NSStoryboardVersion".into(), Plist::Int(1)),
    ]));
    Ok(Compiled {
        nibs: vec![("MainMenu.nib".into(), nib)],
        info,
    })
}

fn rename_responders(element: &mut Element, responders: &[&str]) {
    for (key, value) in &mut element.attributes {
        if matches!(key.as_str(), "id" | "target" | "destination")
            && responders.contains(&value.as_str())
        {
            *value = "-1".into();
        }
    }
    for child in &mut element.children {
        rename_responders(child, responders);
    }
}
