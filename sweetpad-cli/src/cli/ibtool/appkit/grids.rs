use super::{Element, Encoder, Id, Value, id, number, yes};

fn placement(element: &Element, key: &str) -> Result<i64, String> {
    match element.attr(key) {
        None | Some("inherited") => Ok(0),
        Some("none") => Ok(1),
        Some("leading" | "top") => Ok(2),
        Some("trailing" | "bottom") => Ok(3),
        Some("center") => Ok(4),
        Some("fill") => Ok(5),
        Some(value) => Err(format!("unsupported grid {key}: {value}")),
    }
}
fn alignment(element: &Element) -> Result<i64, String> {
    match element.attr("rowAlignment") {
        None | Some("inherited") => Ok(0),
        Some("none") => Ok(1),
        Some("firstBaseline") => Ok(2),
        Some("lastBaseline") => Ok(3),
        Some(value) => Err(format!("unsupported grid alignment {value}")),
    }
}
pub(super) fn content(cell: &Element) -> Option<&Element> {
    cell.children
        .iter()
        .find(|v| v.attr("key") == Some("contentView"))
}
pub(super) fn visible(cell: &Element, grid: &Element) -> bool {
    [("row", "rows"), ("column", "columns")]
        .iter()
        .all(|(key, group)| {
            grid.child(group)
                .and_then(|list| {
                    list.children
                        .iter()
                        .find(|e| e.attr("id") == cell.attr(key))
                })
                .is_none_or(|e| !yes(e, "hidden"))
        })
}

fn dimension(cells: &[&Element], reference: &str, member: &str, axis: &str) -> Result<f64, String> {
    let mut maximum = 0.0_f64;
    for cell in cells
        .iter()
        .filter(|cell| cell.attr(reference) == Some(member))
    {
        if content(cell).is_some_and(|view| {
            matches!(
                view.name.as_str(),
                "textField" | "secureTextField" | "button" | "popUpButton" | "imageView"
            )
        }) {
            return Ok(f64::from(f32::MIN_POSITIVE));
        }
        if let Some(rect) = content(cell).and_then(|v| v.child("rect")) {
            maximum = maximum.max(number(rect, axis, 0.0)?);
        }
    }
    Ok(maximum)
}

impl Encoder<'_> {
    pub(super) fn grid(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for (key, attribute) in [
            ("NSGrid_rowSpacing", "rowSpacing"),
            ("NSGrid_columnSpacing", "columnSpacing"),
        ] {
            self.graph
                .set(object, key, number(element, attribute, 6.0)?);
        }
        self.graph.set(
            object,
            "NSGrid_xPlacement",
            placement(element, "xPlacement")?,
        );
        self.graph.set(
            object,
            "NSGrid_yPlacement",
            placement(element, "yPlacement")?,
        );
        self.graph
            .set(object, "NSGrid_alignment", alignment(element)?);
        let rows = element
            .child("rows")
            .map_or(&[][..], |r| r.children.as_slice());
        let columns = element
            .child("columns")
            .map_or(&[][..], |r| r.children.as_slice());
        let cells = element
            .child("gridCells")
            .map_or(Vec::new(), |r| r.children.iter().collect::<Vec<_>>());
        let mut row_objects = Vec::new();
        for row in rows {
            let row_object = self.object(row)?;
            self.owners.push((row_object, object));
            self.oids.push(row_object);
            self.graph.set(row_object, "NSGrid_owningGrid", object);
            self.graph.set(
                row_object,
                "NSGrid_yPlacement",
                placement(row, "yPlacement")?,
            );
            self.graph
                .set(row_object, "NSGrid_alignment", alignment(row)?);
            self.graph.set(
                row_object,
                "NSGrid_height",
                number(row, "height", dimension(&cells, "row", id(row)?, "height")?)?,
            );
            for (key, attribute) in [
                ("NSGrid_topPadding", "topPadding"),
                ("NSGrid_bottomPadding", "bottomPadding"),
            ] {
                self.graph
                    .set(row_object, key, number(row, attribute, 0.0)?);
            }
            self.graph
                .set(row_object, "NSGrid_hidden", yes(row, "hidden"));
            row_objects.push(row_object);
        }
        let column_objects = self.grid_columns(columns, &cells, object)?;
        for row in rows {
            let row_object = self.object(row)?;
            let mut cell_objects = Vec::new();
            for column in columns {
                let cell = cells
                    .iter()
                    .find(|c| {
                        c.attr("row") == row.attr("id") && c.attr("column") == column.attr("id")
                    })
                    .ok_or("a grid row has a missing cell")?;
                let cell_object = self.grid_cell(
                    cell,
                    object,
                    row_object,
                    self.object(column)?,
                    visible(cell, element),
                )?;
                cell_objects.push(cell_object);
            }
            let cells = self.graph.array("NSMutableArray", &cell_objects);
            self.graph.set(row_object, "NSGrid_cells", cells);
        }
        let rows = self.graph.array("NSMutableArray", &row_objects);
        let columns = self.graph.array("NSMutableArray", &column_objects);
        self.graph.set(object, "NSGrid_rows", rows);
        self.graph.set(object, "NSGrid_columns", columns);
        Ok(())
    }

    fn grid_columns(
        &mut self,
        columns: &[Element],
        cells: &[&Element],
        object: Id,
    ) -> Result<Vec<Id>, String> {
        let mut column_objects = Vec::new();
        for column in columns {
            let column_object = self.object(column)?;
            self.owners.push((column_object, object));
            self.oids.push(column_object);
            self.graph.set(column_object, "NSGrid_owningGrid", object);
            self.graph.set(
                column_object,
                "NSGrid_xPlacement",
                placement(column, "xPlacement")?,
            );
            self.graph.set(
                column_object,
                "NSGrid_width",
                number(
                    column,
                    "width",
                    dimension(cells, "column", id(column)?, "width")?,
                )?,
            );
            for (key, attribute) in [
                ("NSGrid_leadingPadding", "leadingPadding"),
                ("NSGrid_trailingPadding", "trailingPadding"),
            ] {
                self.graph
                    .set(column_object, key, number(column, attribute, 0.0)?);
            }
            self.graph
                .set(column_object, "NSGrid_hidden", yes(column, "hidden"));
            column_objects.push(column_object);
        }
        Ok(column_objects)
    }

    fn grid_cell(
        &mut self,
        element: &Element,
        grid: Id,
        row: Id,
        column: Id,
        visible: bool,
    ) -> Result<Id, String> {
        let object = self.object(element)?;
        self.owners.push((object, grid));
        self.oids.push(object);
        self.graph.set(object, "NSGrid_owningRow", row);
        self.graph.set(object, "NSGrid_owningColumn", column);
        self.graph.set(object, "NSGrid_mergeHead", Value::Nil);
        self.graph.set(
            object,
            "NSGrid_xPlacement",
            placement(element, "xPlacement")?,
        );
        self.graph.set(
            object,
            "NSGrid_yPlacement",
            placement(element, "yPlacement")?,
        );
        self.graph
            .set(object, "NSGrid_alignment", alignment(element)?);
        let view = if let Some(view) = content(element) {
            if !visible {
                self.root_views.insert(self.object(view)?);
            }
            let view = self.encode(view, grid)?;
            self.owners
                .iter_mut()
                .find(|(child, _)| *child == view)
                .ok_or("missing grid content ownership")?
                .1 = object;
            Value::Object(view)
        } else {
            Value::Nil
        };
        self.graph.set(object, "NSGrid_content", view);
        Ok(object)
    }
}
