//! An iOS XIB is one nib containing its top-level objects and connections.

use std::borrow::Cow;

use sweetpad_lib::xcscheme::Element;

use super::Options;
use super::storyboard::{nib_root, proxy};
use super::uikit::{Encoder, number, prepare_cell};

pub(super) fn compile(
    document: &Element,
    options: &Options<'_>,
    deployment_major: Option<u32>,
) -> Result<Vec<u8>, String> {
    if document.attr("type") == Some("com.apple.InterfaceBuilder3.Cocoa.XIB") {
        return super::appkit::compile(document, options);
    }
    if document.attr("type") != Some("com.apple.InterfaceBuilder3.CocoaTouch.XIB") {
        return Err("this isn't an iOS XIB".to_string());
    }
    let objects = document.child("objects").ok_or("the XIB has no objects")?;
    let elements: Vec<_> = objects
        .children
        .iter()
        .map(|e| {
            if e.name == "collectionViewCell" {
                Cow::Owned(prepare_cell(e))
            } else {
                Cow::Borrowed(e)
            }
        })
        .collect();
    let mut encoder = Encoder::new(document, options);
    let device_canvas = deployment_major
        .filter(|v| *v < 17)
        .filter(|_| document.attr("targetRuntime") == Some("iOS.CocoaTouch"))
        .map(|_| legacy_canvas(document.child("device")));
    let mut top = Vec::new();
    for element in &elements {
        if element.name == "placeholder" {
            let identifier = element
                .attr("placeholderIdentifier")
                .ok_or("a placeholder has no identifier")?;
            let object = proxy(&mut encoder.graph, identifier);
            encoder.bind(element.attr("id").ok_or("a placeholder has no id")?, object);
            top.push(object);
        } else {
            encoder.declare(element)?;
            // Before iOS 17, full-screen top-level views use the simulated device's
            // dimensions. Freeform views, cells and views inside them keep their own frames.
            if let Some((width, height)) = device_canvas
                && matches!(
                    element.name.as_str(),
                    "view" | "scrollView" | "tableView" | "collectionView" | "textView"
                )
                && !element.children.iter().any(|c| {
                    c.attr("key") == Some("simulatedDestinationMetrics")
                        && c.name == "freeformSimulatedSizeMetrics"
                })
                && let Some(frame) = element
                    .children_named("rect")
                    .find(|r| r.attr("key") == Some("frame"))
                && let Some(id) = element.attr("id")
            {
                encoder.frame_overrides.insert(
                    id.into(),
                    [
                        number(frame, "x")?.unwrap_or(0.0),
                        number(frame, "y")?.unwrap_or(0.0),
                        width,
                        height,
                    ],
                );
            }
        }
    }
    let mut objects = top.clone();
    for element in &elements {
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

fn legacy_canvas(device: Option<&Element>) -> (f64, f64) {
    let portrait = match device.and_then(|d| d.attr("id")) {
        Some("retina4_0") => (320.0, 568.0),
        Some("retina4_7") => (375.0, 667.0),
        Some("retina5_5") => (414.0, 736.0),
        Some("retina5_9") => (375.0, 812.0),
        Some("retina6_1" | "retina6_5") => (414.0, 896.0),
        Some("retina6_12" | "retina3_5") => (393.0, 852.0),
        Some("retina6_3") => (402.0, 874.0),
        Some("retina6_7") => (430.0, 932.0),
        Some("retina6_9") => (440.0, 956.0),
        Some("ipad9_7") => (768.0, 1024.0),
        Some("ipad10_2") => (810.0, 1080.0),
        Some("ipad7_9") => (744.0, 1133.0),
        _ => (600.0, 600.0),
    };
    if device.and_then(|d| d.attr("orientation")) == Some("landscape") {
        (portrait.1, portrait.0)
    } else {
        portrait
    }
}
