//! Table prototypes load from nested NSNib archives in the parent bundle.
use super::{Element, Encoder, Id, Value};

fn element(name: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
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
pub(super) fn prepare(document: &mut Element) -> Result<(), String> {
    fn walk(node: &mut Element, enclosed: bool, storyboard: bool) -> Result<(), String> {
        if !storyboard && matches!(node.name.as_str(), "tableView" | "outlineView") {
            let columns = node
                .child("tableColumns")
                .map_or(Vec::new(), |c| c.children.iter().collect::<Vec<_>>());
            let widths = super::tables::column_widths(node, &columns, enclosed)?;
            let spacing = node
                .children_named("size")
                .find(|s| s.attr("key") == Some("intercellSpacing"));
            let sx = spacing.map_or(Ok(3.0), |s| super::number(s, "width", 3.0))?;
            let sy = spacing.map_or(Ok(2.0), |s| super::number(s, "height", 2.0))?;
            let mut extra = 12.0 - sx;
            let mut remaining = f64::from(u32::try_from(widths.len()).unwrap());
            let mut x = 10.0 + (sx / 2.0).floor();
            if let Some(columns) = node.children.iter_mut().find(|c| c.name == "tableColumns") {
                for (column, width) in columns.children.iter_mut().zip(widths) {
                    let addition = (extra / remaining).round();
                    remaining -= 1.0;
                    extra -= addition;
                    let width = width + addition;
                    if let Some(cells) = column
                        .children
                        .iter_mut()
                        .find(|c| c.name == "prototypeCellViews")
                    {
                        let mut y = (sy / 2.0).floor();
                        for cell in &mut cells.children {
                            if let Some(rect) = cell
                                .children
                                .iter_mut()
                                .find(|c| c.attr("key") == Some("frame"))
                            {
                                for (key, value) in
                                    [("x", x), ("y", y), ("width", width), ("height", 17.0)]
                                {
                                    rect.attributes.retain(|(k, _)| k != key);
                                    rect.attributes.push((key.into(), value.to_string()));
                                }
                            }
                            y += 17.0 + sy;
                        }
                    }
                    x += width + sx;
                }
            }
        }
        let enclosed = node.name == "clipView" || node.name == "subviews" && enclosed;
        for child in &mut node.children {
            walk(child, enclosed, storyboard)?;
        }
        Ok(())
    }
    let storyboard = document
        .attr("type")
        .is_some_and(|kind| kind.contains("Storyboard"));
    walk(document, false, storyboard)
}

fn mark(node: &mut Element) {
    if node.name == "textField" {
        node.attributes
            .push(("ibtoolTablePrototype".into(), "YES".into()));
    }
    for child in &mut node.children {
        mark(child);
    }
}
fn prototype_origin(cell: &mut Element, storyboard: bool) {
    if storyboard
        && cell.child("subviews").is_none()
        && let Some(rect) = cell
            .children
            .iter_mut()
            .find(|c| c.attr("key") == Some("frame"))
    {
        rect.attributes.retain(|(k, _)| k != "y");
        rect.attributes.push(("y".into(), "0".into()));
    }
}
impl Encoder<'_> {
    pub(super) fn table_prototypes(&mut self, table: &Element, object: Id) -> Result<(), String> {
        let cells = table
            .child("tableColumns")
            .into_iter()
            .flat_map(|columns| &columns.children)
            .filter_map(|column| column.child("prototypeCellViews"))
            .flat_map(|cells| &cells.children)
            .collect::<Vec<_>>();
        if cells.is_empty() {
            return Ok(());
        }
        let mut entries = Vec::new();
        for cell in cells {
            let column = table
                .child("tableColumns")
                .unwrap()
                .children
                .iter()
                .find(|c| {
                    c.child("prototypeCellViews").is_some_and(|list| {
                        list.children
                            .iter()
                            .any(|e| e.attr("id") == cell.attr("id"))
                    })
                })
                .ok_or("missing prototype column")?;
            let parent = self.object(column)?;
            if !self.storyboard {
                self.root_views.insert(self.object(cell)?);
                self.encode(cell, parent)?;
            }
            let identifier = cell
                .attr("identifier")
                .ok_or("a prototype table cell has no identifier")?;
            let resources = self
                .images
                .iter()
                .map(|(name, [width, height])| {
                    element(
                        "image",
                        &[
                            ("name", name),
                            ("width", &width.to_string()),
                            ("height", &height.to_string()),
                        ],
                        vec![],
                    )
                })
                .collect();
            let mut cell = cell.clone();
            mark(&mut cell);
            prototype_origin(&mut cell, self.storyboard);
            let document = element(
                "document",
                if self.storyboard {
                    &[("type", "com.apple.InterfaceBuilder3.Cocoa.Storyboard.XIB")][..]
                } else {
                    &[][..]
                },
                vec![
                    element("resources", &[], resources),
                    element(
                        "objects",
                        &[],
                        vec![
                            element("customObject", &[("id", "-2")], vec![]),
                            cell.clone(),
                            element("customObject", &[("id", "-3")], vec![]),
                        ],
                    ),
                ],
            );
            let data = super::compile(&document, self.options)?;
            let data = self
                .graph
                .shared("NSData", vec![("NS.bytes", Value::Data(data))]);
            let nib = self.graph.object_with_values(
                "NSNib",
                vec![
                    ("NSNibFileData", data.into()),
                    ("NSNibFileIsKeyed", true.into()),
                    ("NSNibFileUseParentBundle", true.into()),
                    ("NSNibFileImages", Value::Nil),
                    ("NSNibFileSounds", Value::Nil),
                ],
            );
            entries.push((self.graph.string(identifier), nib));
        }
        let prototypes = self.graph.dictionary("NSMutableDictionary", &entries);
        self.graph
            .set(object, "NSTableViewArchivedReusableViewsKey", prototypes);
        let empty = self.graph.array("NSMutableArray", &[]);
        self.graph.set(object, "NSSubviews", empty);
        Ok(())
    }
}
