//! Update ordinary sibling views using Auto Layout and native control metrics.
//! Container-specific layouts remain with their individual encoders.
use std::collections::HashMap;

use cassowary::{
    Constraint, Expression, Solver, Variable,
    WeightedRelation::{EQ, GE, LE},
    strength::REQUIRED,
};

use super::{Element, no, number, yes};

mod metrics;
type Frames = Vec<(String, [f64; 4])>;
type Solution = (Frames, bool);

struct View<'a> {
    element: &'a Element,
    parent: Option<usize>,
    variables: [Variable; 4],
    frame: [f64; 4],
    metrics: Option<metrics::Metrics>,
}

fn frame(view: &Element) -> Option<[f64; 4]> {
    let rect = view
        .children_named("rect")
        .find(|r| r.attr("key") == Some("frame"))?;
    let values = ["x", "y", "width", "height"]
        .map(|key| number(rect, key, 0.0))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if !values.iter().all(|value| value.is_finite()) {
        return None;
    }
    values.try_into().ok()
}

impl View<'_> {
    fn attribute(&self, attribute: &str) -> Option<Expression> {
        let [x, y, w, h] = self.variables;
        let inset = metrics::inset(self.element);
        let vertical = metrics::vertical_inset(self.element);
        Some(match attribute {
            "leading" | "left" => x + inset,
            "trailing" | "right" => x + w - inset,
            "top" => y + vertical,
            "bottom" => y + h - vertical,
            "width" => w - 2.0 * inset,
            "height" => h - 2.0 * vertical,
            "centerX" => x + w * 0.5,
            "centerY" => y + h * 0.5,
            "baseline" | "firstBaseline" | "lastBaseline" => y + self.metrics.as_ref()?.baseline,
            _ => return None,
        })
    }
}

fn strength(priority: f64) -> f64 {
    if priority >= 1000.0 {
        REQUIRED
    } else {
        2.0_f64.powf(priority / 40.0)
    }
}

fn constraint(
    element: &Element,
    owner: &str,
    views: &HashMap<&str, &View<'_>>,
) -> Option<Constraint> {
    if yes(element, "placeholder") || no(element, "active") {
        return None;
    }
    let first = views
        .get(element.attr("firstItem").unwrap_or(owner))?
        .attribute(element.attr("firstAttribute")?)?;
    let mut second = if let Some(id) = element.attr("secondItem") {
        views.get(id)?.attribute(element.attr("secondAttribute")?)?
    } else {
        Expression::from(0.0)
    };
    if let Some(multiplier) = element.attr("multiplier") {
        let multiplier = if let Some((a, b)) = multiplier.split_once(':') {
            a.parse::<f64>().ok()? / b.parse::<f64>().ok()?
        } else {
            multiplier.parse().ok()?
        };
        if !multiplier.is_finite() {
            return None;
        }
        second = second * multiplier;
    }
    let constant = if yes(element, "symbolic") {
        let first = element.attr("firstItem").unwrap_or(owner);
        let second = element.attr("secondItem").unwrap_or(owner);
        if first == owner || second == owner {
            20.0
        } else if views.get(first).is_some_and(|v| v.element.name == "button")
            && views
                .get(second)
                .is_some_and(|v| v.element.name == "button")
        {
            let check = |id| {
                views.get(id).is_some_and(|v| {
                    v.element
                        .child("buttonCell")
                        .is_some_and(|c| matches!(c.attr("type"), Some("check" | "radio")))
                })
            };
            match (check(first), check(second)) {
                (true, true) => 6.0,
                (false, false) => 12.0,
                _ => 20.0,
            }
        } else {
            let checkbox = |id| {
                views.get(id).is_some_and(|v| {
                    v.element
                        .child("buttonCell")
                        .is_some_and(|c| matches!(c.attr("type"), Some("check" | "radio")))
                })
            };
            let bezel = |id| {
                views.get(id).is_some_and(|v| {
                    v.element
                        .child("textFieldCell")
                        .is_some_and(|c| c.attr("borderStyle") == Some("bezel"))
                })
            };
            if matches!(element.attr("firstAttribute"), Some("top" | "bottom"))
                && bezel(first)
                && bezel(second)
            {
                10.0
            } else if matches!(element.attr("firstAttribute"), Some("top" | "bottom"))
                && ((checkbox(first) && bezel(second)) || (checkbox(second) && bezel(first)))
            {
                14.0
            } else {
                8.0
            }
        }
    } else {
        number(element, "constant", 0.0).ok()?
    };
    second = second + constant;
    let priority = number(element, "priority", 1000.0).ok()?;
    if !constant.is_finite() || !priority.is_finite() {
        return None;
    }
    let strength = strength(priority);
    Some(match element.attr("relation") {
        Some("lessThanOrEqual") => first | LE(strength) | second,
        Some("greaterThanOrEqual") => first | GE(strength) | second,
        _ => first | EQ(strength) | second,
    })
}

fn model(root: &Element, deep: bool) -> Option<Vec<View<'_>>> {
    fn add<'a>(
        node: &'a Element,
        parent: Option<usize>,
        deep: bool,
        views: &mut Vec<View<'a>>,
    ) -> Option<()> {
        let rect = frame(node)?;
        let index = views.len();
        let local = if let Some(parent) = parent {
            [
                rect[0],
                frame(views[parent].element)?[3] - rect[1] - rect[3],
                rect[2],
                rect[3],
            ]
        } else {
            [0.0, 0.0, rect[2], rect[3]]
        };
        views.push(View {
            element: node,
            parent,
            variables: std::array::from_fn(|_| Variable::new()),
            frame: local,
            metrics: metrics::control(node),
        });
        if parent.is_none() || deep {
            descendants(node, index, deep, views)?;
        }
        Some(())
    }
    fn descendants<'a>(
        node: &'a Element,
        parent: usize,
        deep: bool,
        views: &mut Vec<View<'a>>,
    ) -> Option<()> {
        if matches!(
            node.name.as_str(),
            "tableView" | "outlineView" | "collectionView"
        ) {
            return Some(());
        }
        for child in &node.children {
            if frame(child).is_some() {
                add(child, Some(parent), deep, views)?;
            } else if matches!(child.name.as_str(), "subviews" | "gridCells" | "gridCell") {
                descendants(child, parent, deep, views)?;
            }
        }
        Some(())
    }
    let mut views = Vec::new();
    add(root, None, deep, &mut views)?;
    Some(views)
}

fn constrain_view(solver: &mut Solver, view: &View<'_>, parent: Option<&View<'_>>) -> Option<()> {
    for axis in 0..4 {
        let mut value: Expression = view.frame[axis].into();
        if axis < 2
            && let Some(parent) = parent
        {
            value = value + parent.variables[axis];
        }
        let managed = no(view.element, "translatesAutoresizingMaskIntoConstraints")
            && !yes(view.element, "fixedFrame");
        let container =
            parent.is_some_and(|p| matches!(p.element.name.as_str(), "gridView" | "stackView"));
        solver
            .add_constraint(
                view.variables[axis]
                    | EQ(if !managed || (container && axis < 2) {
                        REQUIRED
                    } else {
                        0.01
                    })
                    | value,
            )
            .ok()?;
    }
    if no(view.element, "translatesAutoresizingMaskIntoConstraints")
        && !yes(view.element, "fixedFrame")
        && let Some(metrics) = &view.metrics
    {
        for (axis, value, hugging, compression) in [
            (
                2,
                metrics.width,
                "horizontalHuggingPriority",
                "horizontalCompressionResistancePriority",
            ),
            (
                3,
                Some(metrics.height),
                "verticalHuggingPriority",
                "verticalCompressionResistancePriority",
            ),
        ] {
            if let Some(value) = value {
                let value = value.ceil();
                solver
                    .add_constraint(
                        view.variables[axis]
                            | GE(strength(
                                number(view.element, compression, 750.0).ok()? + 1.0,
                            ))
                            | value,
                    )
                    .ok()?;
                solver
                    .add_constraint(
                        view.variables[axis]
                            | LE(strength(
                                number(
                                    view.element,
                                    hugging,
                                    if axis == 3 { 750.0 } else { 250.0 },
                                )
                                .ok()?,
                            ))
                            | value,
                    )
                    .ok()?;
            }
        }
    }
    if let Ok(height) = number(view.element, "ibtoolFittedHeight", f64::NAN)
        && height.is_finite()
    {
        solver
            .add_constraint(view.variables[3] | EQ(strength(250.0)) | height)
            .ok()?;
    }
    if view.element.name == "textView" {
        for (key, relation) in [("minSize", true), ("maxSize", false)] {
            if let Some(size) = view
                .element
                .children_named("size")
                .find(|s| s.attr("key") == Some(key))
            {
                for (axis, name) in [(2, "width"), (3, "height")] {
                    let value = number(size, name, 0.0).ok()?;
                    if value > 0.0 {
                        solver
                            .add_constraint(if relation {
                                view.variables[axis] | GE(REQUIRED) | value
                            } else {
                                view.variables[axis] | LE(REQUIRED) | value
                            })
                            .ok()?;
                    }
                }
            }
        }
    }
    Some(())
}

fn solve(root: &Element, fitting: bool) -> Option<Solution> {
    let mut views = model(root, false)?;
    let ids = views
        .iter()
        .filter_map(|v| v.element.attr("id"))
        .collect::<Vec<_>>();
    let needs_deep = root.child("constraints").is_some_and(|g| {
        g.children.iter().any(|c| {
            ["firstItem", "secondItem"]
                .iter()
                .any(|key| c.attr(key).is_some_and(|id| !ids.contains(&id)))
        })
    });
    if needs_deep {
        views = model(root, true)?;
    }

    let constraints = views
        .iter()
        .flat_map(|view| {
            view.element
                .child("constraints")
                .into_iter()
                .flat_map(move |group| {
                    group
                        .children
                        .iter()
                        .map(move |c| (view.element.attr("id").unwrap_or(""), c))
                })
        })
        .collect::<Vec<_>>();
    // Leave incomplete, fixed-frame documents at their saved canvas geometry.
    if !views.iter().skip(1).any(|view| {
        no(view.element, "translatesAutoresizingMaskIntoConstraints")
            && constraints.iter().any(|(_, c)| {
                c.attr("firstItem") == view.element.attr("id")
                    && matches!(c.attr("firstAttribute"), Some("top" | "bottom" | "centerY"))
            })
    }) {
        return None;
    }
    let by_id = views
        .iter()
        .filter_map(|view| Some((view.element.attr("id")?, view)))
        .collect::<HashMap<_, _>>();
    let mut solver = Solver::new();
    let [x, y, w, h] = views[0].variables;
    solver
        .add_constraints(&[
            x | EQ(REQUIRED) | 0.0,
            y | EQ(REQUIRED) | 0.0,
            w | EQ(strength(100.0)) | number(root, "ibtoolLayoutWidth", views[0].frame[2]).ok()?,
            w | GE(REQUIRED) | number(root, "ibtoolLayoutWidth", views[0].frame[2]).ok()?,
            h | EQ(strength(100.0))
                | if fitting && root.attr("key") != Some("contentView") {
                    if views.iter().skip(1).any(|v| v.metrics.is_some()) {
                        0.0
                    } else {
                        200.0
                    }
                } else {
                    views[0].frame[3]
                },
        ])
        .ok()?;
    for view in views.iter().skip(1) {
        constrain_view(&mut solver, view, view.parent.map(|i| &views[i]))?;
    }
    for (owner, element) in constraints {
        if yes(element, "placeholder") || no(element, "active") {
            continue;
        }
        let rule = constraint(element, owner, &by_id)?;
        solver.add_constraint(rule).ok()?;
    }
    Some((solved_frames(root, fitting, &views, &solver)?, needs_deep))
}

fn solved_frames(
    root: &Element,
    fitting: bool,
    views: &[View<'_>],
    solver: &Solver,
) -> Option<Frames> {
    let height = solver.get_value(views[0].variables[3]);
    let mut result = Vec::new();
    for view in views {
        let [x, y, w, h] = view.variables.map(|v| solver.get_value(v));
        if ![x, y, w, h].iter().all(|v| v.is_finite()) || w < 0.0 || h < 0.0 {
            return None;
        }
        let (parent_y, parent_height) = view.parent.map_or((0.0, height), |i| {
            (
                solver.get_value(views[i].variables[1]),
                solver.get_value(views[i].variables[3]),
            )
        });
        let bottom = parent_height - (y - parent_y) - h;
        let x = x - view
            .parent
            .map_or(0.0, |i| solver.get_value(views[i].variables[0]));
        result.push([
            (x + 0.5).floor(),
            (bottom + 0.5).floor(),
            (x + w + 0.5).floor() - (x + 0.5).floor(),
            (bottom + h + 0.5).floor() - (bottom + 0.5).floor(),
        ]);
    }
    result[0][0] = frame(root)?[0];
    result[0][1] = frame(root)?[1]
        + if fitting {
            0.0
        } else {
            frame(root)?[3] - result[0][3]
        };
    Some(
        views
            .iter()
            .zip(result)
            .filter_map(|(view, rect)| Some((view.element.attr("id")?.to_string(), rect)))
            .collect(),
    )
}

// Some IB containers are laid out before the root view receives its saved
// canvas size. Measure their contents first, then align siblings while keeping
// the root's archived canvas. Cross-container constraints use the deep solver.
fn partial(root: &Element) -> Option<Solution> {
    let views = model(root, false)?;
    let ids = views
        .iter()
        .filter_map(|v| v.element.attr("id"))
        .collect::<Vec<_>>();
    let unknown = |c: &Element| {
        ["firstItem", "secondItem"]
            .iter()
            .any(|key| c.attr(key).is_some_and(|id| !ids.contains(&id)))
    };
    if root
        .child("constraints")
        .is_none_or(|g| g.children.iter().any(&unknown))
    {
        return None;
    }
    let children = views
        .iter()
        .skip(1)
        .filter(|v| {
            v.element
                .child("constraints")
                .is_some_and(|g| g.children.iter().any(&unknown))
        })
        .map(|v| v.element.attr("id").map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    if children.is_empty() {
        return None;
    }
    let mut prepared = root.clone();
    for child in prepared
        .children
        .iter_mut()
        .filter(|g| g.name == "subviews")
        .flat_map(|g| &mut g.children)
    {
        if !child
            .attr("id")
            .is_some_and(|id| children.iter().any(|c| c == id))
        {
            continue;
        }
        let (frames, _) = solve(child, true)?;
        let rect = frames
            .iter()
            .find(|(id, _)| child.attr("id") == Some(id))?
            .1;
        set_frame(child, rect);
        child
            .attributes
            .push(("ibtoolFittedHeight".into(), rect[3].to_string()));
        if let Some(group) = child.children.iter_mut().find(|g| g.name == "constraints") {
            group.children.retain(|c| !unknown(c));
        }
    }
    let (mut frames, _) = solve(&prepared, true)?;
    if let Some((_, rect)) = frames
        .iter_mut()
        .find(|(id, _)| root.attr("id") == Some(id))
    {
        *rect = frame(root)?;
    }
    Some((frames, false))
}

#[allow(clippy::float_cmp)] // Exact rectangle equality prevents unnecessary mutations.
fn resize_children(view: &mut Element, old: [f64; 4], new: [f64; 4]) {
    let delta = [new[2] - old[2], new[3] - old[3]];
    for child in &mut view.children {
        if frame(child).is_none() {
            if child.name == "subviews" {
                resize_children(child, old, new);
            }
            continue;
        }
        let Some(mask) = child.child("autoresizingMask") else {
            continue;
        };
        if no(child, "translatesAutoresizingMaskIntoConstraints") {
            continue;
        }
        let mut rect = frame(child).unwrap();
        let old_child = rect;
        for (axis, key) in [(0, "widthSizable"), (1, "heightSizable")] {
            if yes(mask, key) {
                rect[axis + 2] += delta[axis];
            }
        }
        if rect != old_child {
            resize_children(child, old_child, rect);
            set_frame(child, rect);
        }
    }
}

fn set_frame(view: &mut Element, frame: [f64; 4]) {
    if let Some(rect) = view
        .children
        .iter_mut()
        .find(|r| r.name == "rect" && r.attr("key") == Some("frame"))
    {
        for (key, value) in ["x", "y", "width", "height"].into_iter().zip(frame) {
            if let Some((_, stored)) = rect.attributes.iter_mut().find(|(name, _)| name == key) {
                *stored = value.to_string();
            } else {
                rect.attributes.push((key.into(), value.to_string()));
            }
        }
    }
}

fn prepare_grid(grid: &mut Element) {
    let Some(metrics) = metrics::container(grid) else {
        return;
    };
    let Some(mut rect) = frame(grid) else {
        return;
    };
    let Some(rows) = grid.child("rows") else {
        return;
    };
    let rows = rows
        .children
        .iter()
        .filter_map(|r| r.attr("id"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let spacing = number(grid, "rowSpacing", 6.0).unwrap_or(6.0);
    let mut top = metrics.height;
    if let Some(cells) = grid.children.iter_mut().find(|g| g.name == "gridCells") {
        for row in rows {
            let height = cells
                .children
                .iter()
                .filter(|c| c.attr("row") == Some(&row))
                .filter_map(super::grids::content)
                .filter_map(metrics::control)
                .map(|m| m.height.ceil())
                .fold(0.0_f64, f64::max);
            top -= height;
            for cell in cells
                .children
                .iter_mut()
                .filter(|c| c.attr("row") == Some(&row))
            {
                if let Some(view) = cell
                    .children
                    .iter_mut()
                    .find(|v| v.attr("key") == Some("contentView"))
                    && let Some(m) = metrics::control(view)
                    && let Some(mut frame) = frame(view)
                {
                    frame[1] = top + ((height - m.height.ceil()) / 2.0).round();
                    frame[3] = m.height.ceil();
                    set_frame(view, frame);
                }
            }
            top -= spacing;
        }
    }
    rect[1] += rect[3] - metrics.height;
    rect[3] = metrics.height;
    set_frame(grid, rect);
}

fn apply(element: &mut Element, frames: &[(String, [f64; 4])]) {
    if let Some((_, rect)) = frames.iter().find(|(id, _)| element.attr("id") == Some(id)) {
        if let Some(old) = frame(element) {
            resize_children(element, old, *rect);
        }
        set_frame(element, *rect);
    }
    for child in &mut element.children {
        apply(child, frames);
    }
}

pub(super) fn prepare(document: &mut Element) {
    fn walk(element: &mut Element, fitting: bool, skip_layout: bool) -> bool {
        let mut skip_children = skip_layout;
        let mut updated = false;
        if let Some(subviews) = element.child("subviews") {
            let grids = subviews
                .children
                .iter()
                .filter(|v| v.name == "gridView")
                .filter_map(|v| v.attr("id"))
                .filter(|id| {
                    element.child("constraints").is_some_and(|g| {
                        g.children.iter().any(|c| {
                            c.attr("firstItem") == Some(*id) || c.attr("secondItem") == Some(*id)
                        })
                    })
                })
                .map(str::to_string)
                .collect::<Vec<_>>();
            if let Some(subviews) = element.children.iter_mut().find(|g| g.name == "subviews") {
                for grid in &mut subviews.children {
                    if grid
                        .attr("id")
                        .is_some_and(|id| grids.iter().any(|g| g == id))
                    {
                        prepare_grid(grid);
                    }
                }
            }
        }
        if !skip_layout
            && let Some((frames, deep)) = solve(element, fitting).or_else(|| partial(element))
        {
            skip_children |= deep;
            updated = true;
            apply(element, &frames);
        }

        let fitting = element.name == "window" || element.name == "subviews";
        let window = (element.name == "window")
            .then(|| {
                element
                    .children_named("rect")
                    .find(|r| r.attr("key") == Some("contentRect"))
                    .map(|r| {
                        [
                            number(r, "width", 0.0).unwrap_or(0.0),
                            number(r, "height", 0.0).unwrap_or(0.0),
                        ]
                    })
            })
            .flatten();
        for child in &mut element.children {
            if let Some([width, _]) = window
                && frame(child).is_some()
            {
                child
                    .attributes
                    .push(("ibtoolLayoutWidth".into(), width.to_string()));
            }
            let child_updated = walk(child, fitting, skip_children);
            if child_updated
                && let Some([width, height]) = window
                && let Some(mut rect) = frame(child)
            {
                if let Some(mask) = child.child("autoresizingMask") {
                    if yes(mask, "widthSizable") {
                        rect[2] = width;
                    }
                    if yes(mask, "heightSizable") {
                        rect[3] = height;
                    }
                }
                rect[3] = rect[3].max(height);
                rect[1] = height - rect[3];
                set_frame(child, rect);
            }
        }
        updated
    }
    if yes(document, "useAutolayout") {
        let _ = walk(document, false, false);
    }
}
