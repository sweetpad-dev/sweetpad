//! Interface Builder stores older collection cell constraints on the cell.
//! UIKit loads them on its content view and pins that content to the cell.
use super::Element;
fn element(name: &str, attributes: &[(&str, &str)]) -> Element {
    Element {
        name: name.to_string(),
        attributes: attributes
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        children: Vec::new(),
        text: String::new(),
    }
}
pub(in crate::cli::ibtool) fn prepare(source: &Element) -> Element {
    let mut cell = source.clone();
    if cell.name != "collectionViewCell" {
        return cell;
    }
    let Some(content_index) = cell
        .children
        .iter()
        .position(|e| e.attr("key") == Some("contentView"))
    else {
        return cell;
    };
    let cell_id = cell.attr("id").unwrap_or("cell").to_string();
    let content_id = cell.children[content_index]
        .attr("id")
        .map_or_else(|| format!("{cell_id}-contentView"), str::to_string);
    let legacy = cell.children[content_index].attr("id").is_none();
    let safe_area = cell
        .children
        .iter()
        .position(|e| e.name == "viewLayoutGuide" && e.attr("key") == Some("safeArea"));
    if legacy || safe_area.is_some() {
        let guide = safe_area.map(|index| cell.children.remove(index));
        let constraints = cell
            .children
            .iter()
            .position(|e| e.name == "constraints")
            .map(|index| cell.children.remove(index));
        let content = cell
            .children
            .iter_mut()
            .find(|e| e.attr("key") == Some("contentView"))
            .unwrap();
        if legacy {
            // UIKit creates this anonymous content view with its own drawing and clipping defaults.
            content.attributes = vec![
                ("key".to_string(), "contentView".to_string()),
                ("id".to_string(), content_id.clone()),
            ];
        }
        if let Some(guide) = guide {
            content.children.push(guide);
        }
        if let Some(mut constraints) = constraints {
            for constraint in &mut constraints.children {
                for (key, value) in &mut constraint.attributes {
                    if matches!(key.as_str(), "firstItem" | "secondItem") && *value == cell_id {
                        value.clone_from(&content_id);
                    }
                }
            }
            content.children.push(constraints);
        }
    }
    let content = cell
        .children
        .iter_mut()
        .find(|e| e.attr("key") == Some("contentView"))
        .unwrap();
    if content
        .child("constraints")
        .is_some_and(|c| !c.children.is_empty())
    {
        content.attributes.push((
            "translatesAutoresizingMaskIntoConstraints".to_string(),
            "NO".to_string(),
        ));
        let mut constraints = element("constraints", &[]);
        for attribute in ["leading", "trailing", "top", "bottom"] {
            let constraint = element(
                "constraint",
                &[
                    ("id", &format!("{cell_id}-{attribute}")),
                    ("firstItem", &cell_id),
                    ("secondItem", &content_id),
                    ("firstAttribute", attribute),
                    ("secondAttribute", attribute),
                    ("identifier", "UIIBSystemGenerated"),
                ],
            );
            constraints.children.push(constraint);
        }
        cell.children.push(constraints);
    }
    cell
}
