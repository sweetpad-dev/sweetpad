//! Table controls, columns, and their header and data cells.
use super::controls::{cell_flags, enumeration};
use super::{Element, Encoder, Id, Value, no, number, yes};

fn resizing(column: &Element) -> i64 {
    column.child("tableColumnResizingMask").map_or(3, |mask| {
        i64::from(yes(mask, "resizeWithTable")) | (i64::from(yes(mask, "userResizable")) << 1)
    })
}

fn column_style(element: &Element) -> Result<i64, String> {
    enumeration(
        element,
        "columnAutoresizingStyle",
        &[
            ("none", 0),
            ("uniform", 1),
            ("sequential", 2),
            ("reverseSequential", 3),
            ("lastColumnOnly", 4),
            ("firstColumnOnly", 5),
        ],
        1,
    )
}

pub(super) fn column_widths(
    element: &Element,
    columns: &[&Element],
    enclosed: bool,
) -> Result<Vec<f64>, String> {
    let mut widths = columns
        .iter()
        .map(|c| number(c, "width", 100.0))
        .collect::<Result<Vec<_>, _>>()?;
    // Interface Builder fits a single autoresizing column before archiving.
    // With multiple columns it preserves widths and grows the document view.
    if enclosed && columns.len() == 1 && resizing(columns[0]) & 1 != 0 {
        let frame = element.child("rect").ok_or("a table has no frame")?;
        let spacing = element
            .children_named("size")
            .find(|s| s.attr("key") == Some("intercellSpacing"))
            .map_or(Ok(0.0), |s| number(s, "width", 0.0))?;
        if spacing > 3.0 {
            return Ok(widths);
        }
        let available = number(frame, "width", 0.0)? - 35.0 + spacing;
        widths[0] = widths[0]
            .min(available)
            .max(number(columns[0], "minWidth", 10.0)?);
    }
    Ok(widths)
}

pub(super) fn document_size(element: &Element) -> Result<[f64; 2], String> {
    let columns = element
        .child("tableColumns")
        .map_or(Vec::new(), |list| list.children.iter().collect::<Vec<_>>());
    let widths = column_widths(element, &columns, true)?;
    let frame = element.child("rect").ok_or("a table has no frame")?;
    let spacing = element
        .children_named("size")
        .find(|s| s.attr("key") == Some("intercellSpacing"))
        .map_or(Ok(0.0), |s| number(s, "width", 0.0))?;
    let gaps = f64::from(u32::try_from(columns.len().saturating_sub(1)).unwrap());
    Ok([
        (widths.iter().sum::<f64>() + 32.0 + spacing * gaps).max(number(frame, "width", 0.0)?),
        number(frame, "height", 0.0)?,
    ])
}

pub(super) fn prepare(element: &mut Element) -> Result<(), String> {
    if let Some(style) = element.attr("rowSizeStyle") {
        if style != "systemDefault" {
            return Err(format!("unsupported table row size style {style}"));
        }
        element.attributes.retain(|(key, _)| key != "rowHeight");
        element.attributes.push(("rowHeight".into(), "32".into()));
        if let Some(spacing) = element
            .children
            .iter_mut()
            .find(|c| c.attr("key") == Some("intercellSpacing"))
        {
            spacing.attributes.retain(|(key, _)| key != "height");
            spacing.attributes.push(("height".into(), "0".into()));
        }
    }
    Ok(())
}

impl Encoder<'_> {
    pub(super) fn table_view(
        &mut self,
        element: &Element,
        object: Id,
        enclosed: bool,
    ) -> Result<(), String> {
        let columns = element
            .child("tableColumns")
            .map_or(Vec::new(), |list| list.children.iter().collect::<Vec<_>>());
        let widths = column_widths(element, &columns, enclosed)?;
        let spacing = element
            .children_named("size")
            .find(|s| s.attr("key") == Some("intercellSpacing"));
        let space = spacing.map_or(Ok([0.0; 2]), |s| {
            Ok::<_, String>([number(s, "width", 0.0)?, number(s, "height", 0.0)?])
        })?;
        let row_height = number(element, "rowHeight", 17.0)?;
        if enclosed {
            let [width, height] = document_size(element)?;
            let frame = element.child("rect").ok_or("a table has no frame")?;
            let x = number(frame, "x", 0.0)?;
            let y = number(frame, "y", 0.0)?;
            self.graph.remove(object, "NSFrame");
            self.graph.remove(object, "NSFrameSize");
            if x == 0.0 && y == 0.0 {
                self.string(object, "NSFrameSize", &format!("{{{width}, {height}}}"));
            } else {
                self.string(
                    object,
                    "NSFrame",
                    &format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}"),
                );
            }
            if let Some(Value::Int(flags)) = self.graph.value(object, "NSvFlags") {
                self.graph.replace(object, "NSvFlags", flags | 0x12);
            }
        } else {
            let frame = element.child("rect").ok_or("a table has no frame")?;
            let width = widths.iter().sum::<f64>()
                + 32.0
                + if spacing.is_none() {
                    3.0 * f64::from(u32::try_from(columns.len()).unwrap())
                } else {
                    0.0
                };
            // Interface Builder lays out its preview with the default row
            // height before applying the archived rowHeight property.
            let height =
                (17.0 + space[1]) * if yes(element, "viewBased") { 1.0 } else { 2.0 } + 1.0;
            let x = number(frame, "x", 0.0)?;
            let y = number(frame, "y", 0.0)?;
            self.graph.remove(object, "NSFrame");
            self.graph.remove(object, "NSFrameSize");
            if x == 0.0 && y == 0.0 {
                self.string(object, "NSFrameSize", &format!("{{{width}, {height}}}"));
            } else {
                self.string(
                    object,
                    "NSFrame",
                    &format!("{{{{{x}, {y}}}, {{{width}, {height}}}}}"),
                );
            }
        }
        self.table_control(element, object);
        let corner = self.table_corner();
        self.graph.set(object, "NSCornerView", corner);
        let columns = columns
            .iter()
            .zip(widths)
            .enumerate()
            .map(|(index, (column, width))| self.table_column(column, object, index, width))
            .collect::<Result<Vec<_>, _>>()?;
        let columns = self.graph.array("NSMutableArray", &columns);
        self.graph.set(object, "NSTableColumns", columns);
        self.graph.set(object, "NSIntercellSpacingWidth", space[0]);
        self.graph.set(object, "NSIntercellSpacingHeight", space[1]);
        for (key, archive, default) in [
            (
                "backgroundColor",
                "NSBackgroundColor",
                "controlBackgroundColor",
            ),
            ("gridColor", "NSGridColor", "gridColor"),
        ] {
            let color = if let Some(color) = element
                .children_named("color")
                .find(|c| c.attr("key") == Some(key))
            {
                self.color(color)?
            } else {
                self.named_color(default)?
            };
            let color = if self.storyboard && key == "gridColor" {
                self.named_color("separatorColor")?
            } else {
                color
            };
            self.graph.set(object, archive, color);
        }
        self.graph.set(object, "NSRowHeight", row_height);
        self.table_options(element, object, enclosed, spacing.is_some())?;
        self.table_prototypes(element, object)?;

        Ok(())
    }

    fn table_style(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(style) = element.attr("tableStyle") {
            let style = match style {
                "automatic" => 0_i64,
                "fullWidth" => 1,
                "inset" => 2,
                "sourceList" => 3,
                "plain" => 4,
                other => return Err(format!("unsupported table style {other}")),
            };
            self.graph.set(object, "NSTableViewStyle", style);
        }

        Ok(())
    }
    fn table_options(
        &mut self,
        element: &Element,
        object: Id,
        enclosed: bool,
        spacing: bool,
    ) -> Result<(), String> {
        let style = column_style(element)?;
        self.table_style(element, object)?;
        let columns = element
            .child("tableColumns")
            .map_or(Vec::new(), |list| list.children.iter().collect::<Vec<_>>());
        let widths = column_widths(element, &columns, enclosed)?;
        let resized = columns.iter().zip(widths).any(|(column, width)| {
            number(column, "width", 100.0).is_ok_and(|initial| initial.to_bits() != width.to_bits())
        });
        let grew = enclosed
            && document_size(element)?[0]
                > number(
                    element.child("rect").ok_or("a table has no frame")?,
                    "width",
                    0.0,
                )?;
        let geometry_flags = if element.attr("rowSizeStyle").is_some() {
            4
        } else if grew {
            6
        } else if enclosed
            && spacing
            && resized
            && element
                .children_named("size")
                .find(|s| s.attr("key") == Some("intercellSpacing"))
                .is_some_and(|s| number(s, "width", 0.0).is_ok_and(|n| n > 0.0))
        {
            4
        } else {
            2
        };
        let mut flags = 0x0200_0000_i64 | (geometry_flags << 20);
        for (attribute, bit, default) in [
            ("columnReordering", 31, true),
            ("columnResizing", 30, true),
            ("emptySelection", 28, true),
            ("multipleSelection", 27, true),
            ("columnSelection", 26, false),
            ("autosaveColumns", 24, true),
            ("alternatingRowBackgroundColors", 23, false),
        ] {
            flags |= i64::from(if default {
                !no(element, attribute)
            } else {
                yes(element, attribute)
            }) << bit;
        }
        if style == 1 {
            flags |= 1 << 15;
        }
        self.graph.set(
            object,
            "NSTvFlags",
            if flags >= 0x8000_0000 {
                flags - 0x1_0000_0000
            } else {
                flags
            },
        );
        self.graph.set(object, "NSDelegate", Value::Nil);
        self.graph.set(object, "NSDataSource", Value::Nil);
        self.graph.set(object, "NSColumnAutoresizingStyle", style);
        self.graph
            .set(object, "NSDraggingSourceMaskForLocal", -1_i64);
        self.graph
            .set(object, "NSDraggingSourceMaskForNonLocal", 0_i64);
        self.graph
            .set(object, "NSAllowsTypeSelect", !no(element, "typeSelect"));
        self.graph.set(
            object,
            "NSTableViewDraggingDestinationStyle",
            i64::from(element.attr("selectionHighlightStyle") == Some("sourceList")),
        );
        self.graph.set(object, "NSTableViewGroupRowStyle", 1_i64);
        if no(element, "floatsGroupRows") {
            self.graph
                .set(object, "NSTableViewShouldFloatGroupRows", false);
        }
        if element.attr("rowSizeStyle") == Some("systemDefault") {
            self.graph.set(object, "NSTableViewRowSizeStyle", -1_i64);
        }
        if let Some(style) = element.attr("selectionHighlightStyle") {
            let style = match style {
                "none" => -1_i64,
                "regular" => 0,
                "sourceList" => 1,
                other => return Err(format!("unsupported table selection style {other}")),
            };
            if style != 0 {
                self.graph
                    .set(object, "NSTableViewSelectionHighlightStyle", style);
            }
        }
        Ok(())
    }

    fn table_control(&mut self, element: &Element, object: Id) {
        self.graph.set(object, "NSEnabled", !no(element, "enabled"));
        self.graph.set(
            object,
            "NSAllowsLogicalLayoutDirection",
            self.options.localized,
        );
        if yes(element, "allowsExpansionToolTips") {
            self.graph
                .set(object, "NSControlAllowsExpansionToolTips", true);
        }
        for key in [
            "NSControlSize",
            "NSControlTextAlignment",
            "NSControlLineBreakMode",
            "NSControlWritingDirection",
            "NSControlSendActionMask",
        ] {
            self.graph.set(object, key, 0_i64);
        }
        for key in [
            "NSControlContinuous",
            "NSControlRefusesFirstResponder",
            "NSControlUsesSingleLineMode",
        ] {
            self.graph.set(object, key, false);
        }
    }

    fn table_corner(&mut self) -> Id {
        let corner = self.graph.object("_NSCornerView");
        for key in [
            "NSNextResponder",
            "NSNibTouchBar",
            "IBNSSafeAreaLayoutGuide",
            "IBNSLayoutMarginsGuide",
        ] {
            self.graph.set(corner, key, Value::Nil);
        }
        self.graph.set(corner, "NSvFlags", 256_i64);
        self.string(corner, "NSFrameSize", "{17, 28}");
        self.graph
            .set(corner, "NSViewWantsBestResolutionOpenGLSurface", true);
        self.graph.set(corner, "IBNSClipsToBounds", 0_i64);
        corner
    }

    fn table_column(
        &mut self,
        column: &Element,
        table: Id,
        index: usize,
        width: f64,
    ) -> Result<Id, String> {
        let object = self.object(column)?;
        self.owners.push((object, table));
        self.oids.push(object);
        self.string(
            object,
            "NSIdentifier",
            column
                .attr("identifier")
                .unwrap_or(&format!("AutomaticTableColumnIdentifier.{index}")),
        );
        self.graph.set(object, "NSWidth", width);
        self.graph
            .set(object, "NSMinWidth", number(column, "minWidth", 10.0)?);
        self.graph
            .set(object, "NSMaxWidth", number(column, "maxWidth", 1000.0)?);
        for (key, archive) in [("headerCell", "NSHeaderCell"), ("dataCell", "NSDataCell")] {
            if let Some(cell) = column
                .children
                .iter()
                .find(|cell| cell.attr("key") == Some(key))
            {
                let cell = self.table_cell(cell, column, object, table)?;
                self.graph.set(object, archive, cell);
            }
        }
        let mask = resizing(column);
        if mask != 0 {
            self.graph.set(object, "NSResizingMask", mask);
        }
        if mask != 0 {
            self.graph.set(object, "NSIsResizeable", true);
        }
        self.graph
            .set(object, "NSIsEditable", !no(column, "editable"));
        self.graph.set(object, "NSTableView", table);
        if yes(column, "hidden") {
            self.graph.set(object, "NSHidden", true);
        }
        Ok(object)
    }

    fn table_cell(
        &mut self,
        cell: &Element,
        column: &Element,
        parent: Id,
        table: Id,
    ) -> Result<Id, String> {
        let header = cell.name == "tableHeaderCell";
        if !header && cell.name != "textFieldCell" {
            return Err(format!("unsupported table data cell <{}>", cell.name));
        }
        let object = if cell.attr("id").is_some() {
            let object = self.object(cell)?;
            self.owners.push((object, parent));
            self.oids.push(object);
            object
        } else {
            self.graph.object(if header {
                "NSTableHeaderCell"
            } else {
                "NSTextFieldCell"
            })
        };
        let alignment = enumeration(
            cell,
            "alignment",
            &[
                ("left", 0),
                ("center", 1),
                ("right", 2),
                ("justified", 3),
                ("natural", 4),
            ],
            4,
        )?;
        let line_break = enumeration(
            cell,
            "lineBreakMode",
            &[
                ("wordWrapping", 0),
                ("charWrapping", 1),
                ("clipping", 2),
                ("truncatingHead", 3),
                ("truncatingTail", 4),
                ("truncatingMiddle", 5),
            ],
            0,
        )?;
        let (flags, flags2) = cell_flags(column, cell, alignment, line_break, 0);
        self.graph.set(object, "NSCellFlags", flags);
        self.graph.set(object, "NSCellFlags2", flags2);
        let title = super::string_value(cell, "title")?.unwrap_or_default();
        let title = if cell.attr("id").is_some() {
            self.text(cell, "title", &title)
        } else {
            self.text(
                column,
                if header {
                    "headerCell.title"
                } else {
                    "dataCell.title"
                },
                &title,
            )
        };
        self.graph.set(object, "NSContents", title);
        let font = if header
            && cell
                .child("font")
                .is_none_or(|font| font.attr("metaFont") == Some("smallSystem"))
        {
            let mut font = cell.child("font").cloned().unwrap_or(Element {
                name: "font".into(),
                attributes: vec![],
                children: vec![],
                text: String::new(),
            });
            font.attributes.retain(|(key, _)| key != "metaFont");
            font.attributes
                .push(("textStyle".into(), "subheadline".into()));
            self.font(&font)?
        } else {
            self.cell_font(cell)?
        };
        self.graph.set(object, "NSSupport", font);
        if !header {
            self.graph.set(object, "NSControlView", table);
        }
        self.text_cell_properties(cell, object)?;
        Ok(object)
    }
}
