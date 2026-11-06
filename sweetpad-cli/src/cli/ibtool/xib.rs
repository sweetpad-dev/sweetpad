//! An iOS XIB is one nib containing its top-level objects and connections.

use sweetpad_lib::xcscheme::Element;

use super::Options;
use super::storyboard::{nib_root, proxy};
use super::uikit::Encoder;

pub(super) fn compile(document: &Element, options: &Options<'_>) -> Result<Vec<u8>, String> {
    if document.attr("type") == Some("com.apple.InterfaceBuilder3.Cocoa.XIB") {
        return super::appkit::compile(document, options);
    }
    if document.attr("type") != Some("com.apple.InterfaceBuilder3.CocoaTouch.XIB") {
        return Err("this isn't an iOS XIB".to_string());
    }
    let elements = document.child("objects").ok_or("the XIB has no objects")?;
    let mut encoder = Encoder::new(document, options);
    let mut top = Vec::new();
    for element in &elements.children {
        if element.name == "placeholder" {
            let identifier = element
                .attr("placeholderIdentifier")
                .ok_or("a placeholder has no identifier")?;
            let object = proxy(&mut encoder.graph, identifier);
            encoder.bind(element.attr("id").ok_or("a placeholder has no id")?, object);
            top.push(object);
        } else {
            encoder.declare(element)?;
        }
    }
    let mut objects = top.clone();
    for element in &elements.children {
        if element.name != "placeholder" {
            top.push(encoder.view(element)?);
        }
    }
    objects.extend(encoder.objects.iter().copied());
    let connections = encoder.connections(document)?;
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
