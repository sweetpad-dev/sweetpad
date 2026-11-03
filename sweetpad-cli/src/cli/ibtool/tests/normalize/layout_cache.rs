//! AppKit can archive an Auto Layout child immediately before or after its
//! parent resizes. Repeated native compilations differ by that resize delta.
//! Normalize only this exact vertical autoresizing displacement, using the
//! unchanged constraint graph to find the final position. Sizes, other frame
//! differences, fixed views, and the constraints themselves remain compared.
use std::collections::HashMap;

use cassowary::{
    Expression, Solver, Variable,
    WeightedRelation::{EQ, GE, LE},
    strength::REQUIRED,
};

use super::{Objects, Read, array_references, new_string, reference, string};

fn number(objects: &Objects, object: usize, key: &str) -> Option<f64> {
    objects[object].values.iter().find_map(|(k, v)| {
        if k != key {
            return None;
        }
        match v {
            Read::Double(n) => Some(*n),
            Read::Float(n) => Some(f64::from(*n)),
            Read::Int(n) => n.to_string().parse().ok(),
            _ => None,
        }
    })
}
fn rect(objects: &Objects, object: usize) -> Option<[f64; 4]> {
    let text = string(objects, reference(objects, object, "NSFrame")?)?;
    let values = text
        .split(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-')))
        .filter(|v| !v.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    values.try_into().ok()
}
#[allow(clippy::cast_possible_truncation)] // Archived integer bitfields.
fn baseline(objects: &Objects, object: usize) -> Option<f64> {
    let cell = reference(objects, object, "NSCell")?;
    Some(match objects[cell].class.as_str() {
        "NSTextFieldCell" | "NSSecureTextFieldCell" => {
            if number(objects, cell, "NSCellFlags")
                .is_some_and(|flags| (flags as i64) & (1 << 22) != 0)
            {
                7.0
            } else {
                3.0
            }
        }
        "NSButtonCell" => 7.0,
        "NSPopUpButtonCell" => 7.25,
        _ => return None,
    })
}
fn attribute(
    objects: &Objects,
    object: usize,
    key: i64,
    variables: &HashMap<usize, (Variable, f64)>,
) -> Option<Expression> {
    let (top, height) = variables.get(&object)?;
    Some(match key {
        3 => (*top).into(),
        4 => *top + *height,
        10 => *top + *height * 0.5,
        11 | 12 => *top + *height - baseline(objects, object)?,
        _ => return None,
    })
}
#[allow(clippy::cast_possible_truncation)] // Archived integer attribute and relation codes.
fn positions(
    objects: &Objects,
    root: usize,
    children: &[usize],
    height: f64,
) -> Option<HashMap<usize, f64>> {
    let variables = children
        .iter()
        .filter_map(|id| Some((*id, (Variable::new(), rect(objects, *id)?[3]))))
        .chain([(root, (Variable::new(), height))])
        .collect::<HashMap<_, _>>();
    if variables.len() != children.len() + 1 {
        return None;
    }
    let mut solver = Solver::new();
    solver
        .add_constraint(variables[&root].0 | EQ(REQUIRED) | 0.0)
        .ok()?;
    let constraints = array_references(objects, reference(objects, root, "NSViewConstraints")?)?;
    let mut edges: HashMap<usize, Vec<usize>> = HashMap::new();
    for constraint in constraints {
        let first = reference(objects, constraint, "NSFirstItem")?;
        let first_attribute = number(objects, constraint, "NSFirstAttribute")? as i64;
        if !matches!(first_attribute, 3 | 4 | 10 | 11 | 12) {
            continue;
        }
        let first_expression = attribute(objects, first, first_attribute, &variables)?;
        let second = reference(objects, constraint, "NSSecondItem")?;
        let second_attribute = number(objects, constraint, "NSSecondAttribute")? as i64;
        let mut second_expression = attribute(objects, second, second_attribute, &variables)?;
        second_expression =
            second_expression * number(objects, constraint, "NSMultiplier").unwrap_or(1.0);
        let constant = if reference(objects, constraint, "NSSymbolicConstant").is_some() {
            if first == root || second == root {
                20.0
            } else {
                return None;
            }
        } else {
            number(objects, constraint, "NSConstant").unwrap_or(0.0)
        };
        let priority = number(objects, constraint, "NSPriority").unwrap_or(1000.0);
        let strength = if priority >= 1000.0 {
            REQUIRED
        } else {
            2.0_f64.powf(priority / 40.0)
        };
        second_expression = second_expression + constant;
        let relation = number(objects, constraint, "NSRelation").unwrap_or(0.0) as i64;
        let constraint = match relation {
            -1 => first_expression | LE(strength) | second_expression,
            1 => first_expression | GE(strength) | second_expression,
            _ => first_expression | EQ(strength) | second_expression,
        };
        solver.add_constraint(constraint).ok()?;
        if relation == 0 && priority >= 1000.0 {
            edges.entry(first).or_default().push(second);
            edges.entry(second).or_default().push(first);
        }
    }
    let mut connected = std::collections::HashSet::from([root]);
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        for neighbor in edges.get(&id).into_iter().flatten() {
            if connected.insert(*neighbor) {
                pending.push(*neighbor);
            }
        }
    }
    Some(
        variables
            .into_iter()
            .filter(|(id, _)| connected.contains(id))
            .map(|(id, (top, h))| (id, (height - solver.get_value(top) - h + 0.5).floor()))
            .collect(),
    )
}
#[allow(clippy::cast_possible_truncation)] // Integer fields are small archived flags/attributes.
pub(super) fn prepare(objects: &mut Objects) {
    let roots = (0..objects.len())
        .filter_map(|root| {
            let frame = rect(objects, root)?;
            (frame[1] < 0.0
                && objects[root]
                    .values
                    .iter()
                    .any(|(k, v)| k == "NSNextResponder" && *v == Read::Nil))
            .then_some((root, frame))
        })
        .collect::<Vec<_>>();
    for (root, frame) in roots {
        let Some(children) =
            reference(objects, root, "NSSubviews").and_then(|id| array_references(objects, id))
        else {
            continue;
        };
        let Some(positions) = positions(objects, root, &children, frame[3]) else {
            continue;
        };
        for child in children {
            if !objects[child]
                .values
                .iter()
                .any(|(k, v)| k == "NSDoNotTranslateAutoresizingMask" && *v == Read::Bool(true))
                || number(objects, child, "NSvFlags").is_none_or(|flags| (flags as i64) & 8 == 0)
            {
                continue;
            }
            let (Some(mut child_frame), Some(y)) =
                (rect(objects, child), positions.get(&child).copied())
            else {
                continue;
            };
            if (child_frame[1] - y + frame[1]).abs() > 1e-6 || (child_frame[1] - y).abs() < 1e-6 {
                continue;
            }
            child_frame[1] = y;
            let [x, y, w, h] = child_frame;
            let text = new_string(objects, format!("{{{{{x}, {y}}}, {{{w}, {h}}}}}"));
            for (key, value) in &mut objects[child].values {
                if key == "NSFrame" {
                    *value = Read::Object(text);
                }
            }
        }
    }
}

#[test]
fn only_the_exact_autoresizing_cache_offset_is_normalized() {
    use crate::cli::ibtool::nib::{Graph, Value};
    fn archive(y: i64, width: i64, constant: i64, managed: bool, relation: i64) -> Vec<u8> {
        let mut graph = Graph::default();
        let root = graph.object("NSView");
        let frame = graph.string("{{0, -11}, {272, 176}}");
        graph.set(root, "NSFrame", frame);
        graph.set(root, "NSNextResponder", Value::Nil);
        let field = graph.object("NSTextField");
        let frame = graph.string(&format!("{{{{62, {y}}}, {{{width}, 24}}}}"));
        graph.set(field, "NSFrame", frame);
        graph.set(field, "NSDoNotTranslateAutoresizingMask", managed);
        graph.set(field, "NSvFlags", 268_i64);
        let children = graph.array("NSMutableArray", &[field]);
        graph.set(root, "NSSubviews", children);
        let constraint = graph.object("NSLayoutConstraint");
        graph.set(constraint, "NSFirstItem", field);
        graph.set(constraint, "NSFirstAttribute", 3_i64);
        graph.set(constraint, "NSSecondItem", root);
        graph.set(constraint, "NSSecondAttribute", 3_i64);
        graph.set(constraint, "NSConstant", constant);
        if relation != 0 {
            graph.set(constraint, "NSRelation", relation);
        }
        let constraints = graph.array("NSArray", &[constraint]);
        graph.set(root, "NSViewConstraints", constraints);
        graph.write(root)
    }
    let expected = super::super::canonical(&archive(88, 200, 64, true, 0));
    assert_eq!(
        expected,
        super::super::canonical(&archive(99, 200, 64, true, 0))
    );
    for invalid in [
        archive(100, 200, 64, true, 0),
        archive(99, 201, 64, true, 0),
        archive(99, 200, 63, true, 0),
    ] {
        assert_ne!(expected, super::super::canonical(&invalid));
    }
    assert_ne!(
        super::super::canonical(&archive(88, 200, 64, false, 0)),
        super::super::canonical(&archive(99, 200, 64, false, 0))
    );
    assert_ne!(
        super::super::canonical(&archive(88, 200, 64, true, -1)),
        super::super::canonical(&archive(99, 200, 64, true, -1))
    );
}
