//! Scroll containers, their clip view, and the two scroller controls.
use super::{Element, Encoder, Id, Value, no, number, yes};

fn element(name: &str, attributes: &[(&str, String)]) -> Element {
    Element {
        name: name.into(),
        attributes: attributes
            .iter()
            .map(|(k, v)| ((*k).into(), v.clone()))
            .collect(),
        children: Vec::new(),
        text: String::new(),
    }
}

fn dimensions(view: &Element) -> Result<[f64; 2], String> {
    let frame = view
        .child("rect")
        .ok_or("a scroll component has no frame")?;
    Ok([number(frame, "width", 0.0)?, number(frame, "height", 0.0)?])
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn coordinate(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let precision = (16 - value.abs().log10().floor() as i32).max(0) as usize;
    let text = format!("{value:.precision$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        text
    }
}

pub(super) fn prepare(document: &Element) -> Result<Element, String> {
    let mut document = document.clone();
    prepare_children(&mut document)?;
    Ok(document)
}

fn prepare_children(view: &mut Element) -> Result<(), String> {
    if matches!(view.name.as_str(), "tableView" | "outlineView") {
        super::tables::prepare(view)?;
    }
    if view.name == "textView" {
        super::texts::prepare(view)?;
    }
    if view.name == "scrollView" {
        retile_text(view)?;
        let clip = view
            .child("clipView")
            .ok_or("a scroll view has no clip view")?;
        let [width, height] = dimensions(clip)?;
        let document = clip
            .child("subviews")
            .and_then(|views| views.children.first())
            .map(|view| {
                if matches!(view.name.as_str(), "tableView" | "outlineView") {
                    super::tables::document_size(view)
                } else {
                    dimensions(view)
                }
            })
            .transpose()?
            .unwrap_or([0.0; 2]);
        let scale = number(view, "magnification", 1.0)?;
        let unused_horizontal = yes(view, "autohidesScrollers") && document[0] <= width / scale;
        for (key, horizontal, extent) in [
            ("horizontalScroller", true, width),
            ("verticalScroller", false, height),
        ] {
            if !view
                .children
                .iter()
                .any(|child| child.attr("key") == Some(key))
            {
                let mut scroller = element(
                    "scroller",
                    &[
                        ("key", key.into()),
                        ("horizontal", if horizontal { "YES" } else { "NO" }.into()),
                        ("id", format!("{}-{key}", super::id(view)?)),
                        ("hidden", "YES".into()),
                    ],
                );
                scroller.children.push(element(
                    "rect",
                    &[
                        ("key", "frame".into()),
                        ("x", "-100".into()),
                        ("y", "-100".into()),
                        (
                            "width",
                            if horizontal {
                                if unused_horizontal { 100.0 } else { extent }
                            } else {
                                17.0
                            }
                            .to_string(),
                        ),
                        (
                            "height",
                            if horizontal {
                                if unused_horizontal { 0.0 } else { 17.0 }
                            } else {
                                extent
                            }
                            .to_string(),
                        ),
                    ],
                ));
                scroller.children.push(element(
                    "autoresizingMask",
                    &[("key", "autoresizingMask".into())],
                ));
                view.children.push(scroller);
            }
            if no(
                view,
                if horizontal {
                    "hasHorizontalScroller"
                } else {
                    "hasVerticalScroller"
                },
            ) {
                let scroller = view
                    .children
                    .iter_mut()
                    .find(|child| child.attr("key") == Some(key))
                    .unwrap();
                scroller.attributes.retain(|(key, _)| key != "hidden");
                scroller.attributes.push(("hidden".into(), "YES".into()));
            }
        }
    }
    for child in &mut view.children {
        prepare_children(child)?;
    }
    Ok(())
}

fn replace_number(element: &mut Element, key: &str, value: f64) {
    element.attributes.retain(|(k, _)| k != key);
    element.attributes.push((key.into(), value.to_string()));
}

#[allow(clippy::float_cmp)] // Omit a saved minimum only when it equals the original size.
fn retile_text(scroll: &mut Element) -> Result<(), String> {
    let Some(clip) = scroll.child("clipView") else {
        return Ok(());
    };
    let Some(document) = clip.child("subviews").and_then(|s| s.children.first()) else {
        return Ok(());
    };
    if document.name != "textView" || !no(document, "editable") {
        return Ok(());
    }
    let frame = scroll.child("rect").ok_or("a scroll view has no frame")?;
    let border = if matches!(scroll.attr("borderType"), None | Some("none")) {
        0.0
    } else {
        1.0
    };
    let vertical = !no(scroll, "hasVerticalScroller") && !yes(scroll, "autohidesScrollers");
    let horizontal = !no(scroll, "hasHorizontalScroller") && !yes(scroll, "autohidesScrollers");
    let width = number(frame, "width", 0.0)? - border * 2.0 - if vertical { 17.0 } else { 0.0 };
    let height = number(frame, "height", 0.0)? - border * 2.0 - if horizontal { 17.0 } else { 0.0 };
    for child in &mut scroll.children {
        if child.name == "clipView" {
            if let Some(frame) = child
                .children
                .iter_mut()
                .find(|c| c.attr("key") == Some("frame"))
            {
                replace_number(frame, "width", width);
                replace_number(frame, "height", height);
            }
            if let Some(document) = child
                .children
                .iter_mut()
                .find(|c| c.name == "subviews")
                .and_then(|s| s.children.first_mut())
                && !yes(document, "horizontallyResizable")
            {
                let old_width = dimensions(document)?[0];
                for item in &mut document.children {
                    if item.attr("key") == Some("minSize")
                        && number(item, "width", 0.0)? == old_width
                    {
                        replace_number(item, "width", width);
                    }
                }
                if let Some(frame) = document
                    .children
                    .iter_mut()
                    .find(|c| c.attr("key") == Some("frame"))
                {
                    replace_number(frame, "width", width);
                }
            }
        } else if child.attr("key") == Some("horizontalScroller")
            && child
                .child("autoresizingMask")
                .is_some_and(|m| yes(m, "widthSizable"))
        {
            if let Some(frame) = child
                .children
                .iter_mut()
                .find(|c| c.attr("key") == Some("frame"))
            {
                replace_number(frame, "width", width);
            }
        } else if child.attr("key") == Some("verticalScroller")
            && vertical
            && let Some(frame) = child
                .children
                .iter_mut()
                .find(|c| c.attr("key") == Some("frame"))
        {
            replace_number(frame, "x", border + width);
            replace_number(frame, "width", 17.0);
            replace_number(frame, "height", height);
        }
    }
    scroll
        .attributes
        .push(("ibtoolRetiled".into(), "YES".into()));
    Ok(())
}

impl Encoder<'_> {
    pub(super) fn clip_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.graph.replace(object, "NSvFlags", 256_i64);
        let document = element.child("subviews").and_then(|s| s.children.first());
        if let Some(document) = document {
            let document = self.object(document)?;
            self.graph.set(object, "NSNextKeyView", document);
            self.graph.set(object, "NSDocView", document);
        }
        let default_color = super::Element {
            name: "color".into(),
            attributes: [
                ("name", "controlBackgroundColor"),
                ("catalog", "System"),
                ("colorSpace", "catalog"),
            ]
            .map(|(k, v)| (k.into(), v.into()))
            .into(),
            children: Vec::new(),
            text: String::new(),
        };
        let background = element
            .children_named("color")
            .find(|c| c.attr("key") == Some("backgroundColor"))
            .unwrap_or(&default_color);
        let color = self.color(background)?;
        if !no(element, "drawsBackground")
            || element
                .children_named("color")
                .any(|c| c.attr("key") == Some("backgroundColor"))
        {
            self.graph.set(object, "NSBGColor", color);
        }
        if element.child("cursor").is_some()
            || document.is_some_and(|v| v.name == "textView" && no(v, "editable"))
        {
            if element
                .child("cursor")
                .is_some_and(|cursor| cursor.attr("type") != Some("arrow"))
            {
                return Err("this AppKit clip cursor isn't compiled yet".into());
            }
            let hotspot = self.graph.string("{1, -1}");
            let cursor = self.graph.shared(
                "NSCursor",
                vec![
                    ("NSHotSpot", hotspot.into()),
                    ("NSCursorType", Value::Int(0)),
                ],
            );
            self.graph.set(object, "NSCursor", cursor);
        }
        let flags = i64::from(no(element, "copiesOnScroll")) * 2
            + i64::from(!no(element, "drawsBackground")) * 4;
        if flags != 0 {
            self.graph.set(object, "NScvFlags", flags);
        }
        // Xcode ignores this XML attribute and archives the decoder default.
        self.graph
            .set(object, "NSAutomaticallyAdjustsContentInsets", true);
        Ok(())
    }

    pub(super) fn scroll_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let clip = element
            .child("clipView")
            .ok_or("a scroll view has no clip view")?;
        let (viewport, origin) = self.magnified_clip(element, clip)?;
        let retiled = yes(element, "ibtoolRetiled")
            || viewport
                .iter()
                .chain(&origin)
                .any(|value| value.fract() != 0.0);
        let document = clip
            .child("subviews")
            .and_then(|s| s.children.first())
            .map(|view| {
                if matches!(view.name.as_str(), "tableView" | "outlineView") {
                    super::tables::document_size(view)
                } else {
                    dimensions(view)
                }
            })
            .transpose()?
            .unwrap_or([0.0; 2]);
        let clip_object = self.object(clip)?;
        self.graph.set(object, "NSNextKeyView", clip_object);
        self.graph.set(object, "NSContentView", clip_object);
        let border = match element.attr("borderType") {
            None | Some("none") => 0_i64,
            Some("line") => 1,
            Some("bezel") => 2,
            Some("groove") => 3,
            Some(other) => return Err(format!("unsupported scroll border {other}")),
        };
        let mut flags = 131_072_i64 + 2048 + border;
        if element.attr("borderType").is_none() {
            flags |= 2;
        }
        flags |= i64::from(!no(element, "usesPredominantAxisScrolling")) << 16;
        flags |= i64::from(!no(element, "hasVerticalScroller")) << 4;
        flags |= i64::from(!no(element, "hasHorizontalScroller")) << 5;
        flags |= i64::from(document[1] > viewport[1]) << 6;
        flags |= i64::from(document[0] > viewport[0]) << 7;
        flags |= i64::from(yes(element, "autohidesScrollers")) << 9;
        flags |= i64::from(yes(element, "allowsMagnification")) << 19;
        for (key, shift) in [
            ("verticalScrollElasticity", 12),
            ("horizontalScrollElasticity", 14),
        ] {
            let value = match element.attr(key) {
                None | Some("automatic") => 0,
                Some("none") => 1,
                Some("allowed") => 2,
                Some(other) => return Err(format!("unsupported scroll elasticity {other}")),
            };
            flags |= value << shift;
        }
        self.graph.set(object, "NSsFlags", flags);
        for (attribute, key, default) in [
            ("minMagnification", "NSMinMagnification", 0.25),
            ("maxMagnification", "NSMaxMagnification", 4.0),
            ("magnification", "NSMagnification", 1.0),
        ] {
            self.graph
                .set(object, key, number(element, attribute, default)?);
        }
        self.scroll_amounts(
            element,
            object,
            clip.child("subviews")
                .and_then(|views| views.children.first()),
        )?;
        for (key, archive, axis) in [
            ("horizontalScroller", "NSHScroller", 0),
            ("verticalScroller", "NSVScroller", 1),
        ] {
            let scroller = element
                .children
                .iter()
                .find(|c| c.attr("key") == Some(key))
                .ok_or("a scroll view has no scroller")?;
            let scroller_object = self.object(scroller)?;
            self.graph.set(object, archive, scroller_object);
            self.scroller(
                scroller,
                scroller_object,
                object,
                axis == 0,
                [document[axis], viewport[axis], origin[axis]],
                retiled,
            );
            if !retiled || yes(scroller, "wantsLayer") {
                self.graph
                    .replace(scroller_object, "NSViewIsLayerTreeHost", true);
            }
        }
        Ok(())
    }

    #[allow(clippy::float_cmp, clippy::cast_possible_truncation)]
    fn magnified_clip(
        &mut self,
        scroll: &Element,
        clip: &Element,
    ) -> Result<([f64; 2], [f64; 2]), String> {
        let size = dimensions(clip)?;
        let scale = number(scroll, "magnification", 1.0)?;
        if scale == 1.0 {
            return Ok((size, [0.0; 2]));
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err("scroll magnification must be finite and positive".into());
        }
        let [tx, ty] = size.map(|s| (s * (1.0 - scale) / 2.0).min(0.0));
        let values = [
            scale,
            0.0,
            0.0,
            scale,
            tx,
            ty,
            -tx * scale,
            -ty * scale,
            scale * scale,
            scale,
            scale,
            0.0,
        ];
        let matrix = self.graph.object_with_values(
            "NSPSMatrix",
            vec![(
                "NSMatrix",
                Value::Data(
                    values
                        .iter()
                        .flat_map(|v| (if *v == 0.0 { 0.0_f32 } else { *v as f32 }).to_be_bytes())
                        .collect(),
                ),
            )],
        );
        let object = self.object(clip)?;
        self.graph.set(object, "NSDrawMatrix", matrix);
        let inverse = 1.0 / scale;
        let origin = [-tx * inverse, -ty * inverse];
        let size = [
            ((size[0] * inverse) + origin[0]) - origin[0],
            ((size[1] * inverse) + origin[1]) - origin[1],
        ];
        self.string(
            object,
            "NSBounds",
            &format!(
                "{{{{{}, {}}}, {{{}, {}}}}}",
                coordinate(origin[0]),
                coordinate(origin[1]),
                coordinate(size[0]),
                coordinate(size[1])
            ),
        );
        Ok((size, origin))
    }

    #[allow(clippy::float_cmp, clippy::cast_possible_truncation)]
    fn scroll_amounts(
        &mut self,
        element: &Element,
        object: Id,
        document: Option<&Element>,
    ) -> Result<(), String> {
        let mut values = [
            "horizontalPageScroll",
            "verticalPageScroll",
            "horizontalLineScroll",
            "verticalLineScroll",
        ]
        .map(|key| number(element, key, 10.0))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
        if let Some(table) = document
            .filter(|document| matches!(document.name.as_str(), "tableView" | "outlineView"))
        {
            let spacing = table
                .children_named("size")
                .find(|size| size.attr("key") == Some("intercellSpacing"));
            let line = number(table, "rowHeight", 17.0)?
                + spacing.map_or(Ok(0.0), |size| number(size, "height", 0.0))?;
            values[2] = line;
            values[3] = line;
        }
        if values.iter().any(|v| *v != 10.0) {
            self.graph.set(
                object,
                "NSScrollAmts",
                Value::Data(
                    values
                        .iter()
                        .flat_map(|v| (*v as f32).to_be_bytes())
                        .collect(),
                ),
            );
        }
        Ok(())
    }

    fn scroller(
        &mut self,
        element: &Element,
        object: Id,
        parent: Id,
        horizontal: bool,
        metrics: [f64; 3],
        retiled: bool,
    ) {
        let [document, viewport, origin] = metrics;
        if document > viewport {
            self.graph.set(object, "NSEnabled", true);
        }
        self.graph.set(
            object,
            "NSAllowsLogicalLayoutDirection",
            self.options.localized,
        );
        for key in ["NSControlAction", "NSAction"] {
            self.string(object, key, "_doScroller:");
        }
        for key in ["NSControlTarget", "NSTarget"] {
            self.graph.set(object, key, parent);
        }
        for key in [
            "NSControlSize",
            "NSControlTextAlignment",
            "NSControlLineBreakMode",
            "NSControlWritingDirection",
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
        self.graph.set(object, "NSControlSendActionMask", 4_i64);
        if horizontal {
            self.graph.set(object, "NSsFlags", 1_i64);
        }
        if !yes(element, "hidden") && document > viewport {
            self.graph.set(object, "NSPercent", viewport / document);
        }
        let enabled_axis = matches!(self.graph.value(parent, "NSsFlags"), Some(Value::Int(flags)) if flags & (if horizontal {32} else {16}) != 0);
        if document > viewport
            && (!yes(element, "hidden") || (horizontal && retiled) || enabled_axis)
        {
            let current = if horizontal {
                origin / (document - viewport)
            } else {
                1.0 - origin / (document - viewport)
            };
            if (current - if horizontal { 0.0 } else { 0.5 }).abs() > f64::EPSILON {
                self.graph.set(object, "NSCurValue", current);
            }
        }
        if let Ok(value) = number(element, "doubleValue", if horizontal { 0.0 } else { 0.5 })
            && element.attr("doubleValue").is_some()
        {
            self.graph.set(object, "NSCurValue", value);
        }
    }
}
