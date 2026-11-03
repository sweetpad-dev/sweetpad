//! The native oracle profile supplies advances and pair adjustments, not fonts.
use std::{collections::HashMap, sync::OnceLock};

use serde::Deserialize;

use super::super::{Element, number};

#[derive(Deserialize)]
struct Font {
    height: f64,
    advances: HashMap<char, f64>,
    kerns: HashMap<String, f64>,
}

pub(super) struct Metrics {
    pub width: Option<f64>,
    pub height: f64,
    pub baseline: f64,
}

fn width(font: &Font, text: &str) -> Option<f64> {
    let mut width = 0.0;
    let mut previous = None;
    for character in text.chars() {
        width += font.advances.get(&character)?;
        if let Some(previous) = previous {
            width += font
                .kerns
                .get(&format!("{previous}{character}"))
                .copied()
                .unwrap_or(0.0);
        }
        previous = Some(character);
    }
    Some((width * 2.0).ceil() / 2.0)
}

fn font(cell: &Element) -> Option<&'static Font> {
    static FONTS: OnceLock<HashMap<String, Font>> = OnceLock::new();
    let fonts =
        FONTS.get_or_init(|| serde_json::from_str(include_str!("../layout-fonts.json")).unwrap());
    let font = cell.child("font")?;
    let meta = font.attr("metaFont").unwrap_or("system");
    let default_size = match font.attr("textStyle") {
        Some("subheadline") => 11.0,
        Some("callout") => 12.0,
        Some("title3") => 15.0,
        _ if meta.starts_with("small") => 11.0,
        _ => 13.0,
    };
    let size = number(font, "size", default_size).ok()?;
    let bold = font.attr("textStyle") == Some("headline")
        || meta.ends_with("Bold")
        || font.attr("name") == Some(".AppleSystemUIFontBold");
    let profile = match (size, bold, meta) {
        (9.0, false, _) => "mini",
        (11.0, false, _) => "small",
        (11.0, true, _) => "smallBold",
        (12.0, false, _) => "regular12",
        (13.0, false, _) => "regular",
        (15.0, false, _) => "regular15",
        (13.0, true, _) => "bold",
        (15.0, true, _) => "bold15",
        (16.0, false, _) => "regular16",
        (16.0, true, _) => "bold16",
        (36.0, false, "systemThin") => "thin36",
        _ => return None,
    };
    if let Some(name) = font.attr("name")
        && font.attr("textStyle").is_none()
        && !super::super::fonts::named_font(name)
            .0
            .starts_with(".AppleSystemUIFont")
    {
        return None;
    }
    fonts.get(profile)
}

fn progress(view: &Element) -> Metrics {
    let height = if view.attr("style") == Some("spinning") {
        if view.attr("controlSize") == Some("small") {
            16.0
        } else {
            32.0
        }
    } else if view.attr("controlSize") == Some("small") {
        12.0
    } else {
        20.0
    };
    Metrics {
        width: if view.attr("style") == Some("spinning") {
            Some(height)
        } else {
            None
        },
        height,
        baseline: 0.0,
    }
}

pub(super) fn control(view: &Element) -> Option<Metrics> {
    if matches!(view.name.as_str(), "stackView" | "gridView") {
        return container(view);
    }
    if view.name == "box" && view.attr("boxType") == Some("separator") {
        return Some(Metrics {
            width: None,
            height: 5.0,
            baseline: 0.0,
        });
    }
    if view.name == "progressIndicator" {
        return Some(progress(view));
    }

    let cell = view
        .children
        .iter()
        .find(|child| child.attr("key") == Some("cell"))?;
    let font = font(cell)?;
    let title = super::super::string_value(cell, "title")
        .ok()?
        .unwrap_or_default();
    let text_width = title
        .lines()
        .map(|line| width(font, line))
        .collect::<Option<Vec<_>>>()
        .map(|widths| widths.into_iter().fold(0.0_f64, f64::max));
    let size = cell.attr("controlSize").or(view.attr("controlSize"));
    if view.name == "button" && matches!(cell.attr("type"), Some("check" | "radio")) {
        let height = font.height.max(16.0);
        return Some(Metrics {
            width: text_width.map(|w| {
                w + match size {
                    Some("small") => 18.0,
                    Some("mini") => 16.0,
                    _ => 22.0,
                }
            }),
            height,
            baseline: height - 3.0,
        });
    }
    match view.name.as_str() {
        "button" if cell.attr("image").is_some() && cell.attr("borderStyle").is_none() => {
            let rect = view.child("rect")?;
            Some(Metrics {
                width: Some(number(rect, "width", 0.0).ok()?),
                height: number(rect, "height", 0.0).ok()?,
                baseline: 0.0,
            })
        }
        "button" if cell.attr("bezelStyle") == Some("rounded") => {
            let (padding, height, baseline) = match size {
                Some("small") => (20.0, 20.0, 14.0),
                Some("mini") => (16.0, 16.0, 12.0),
                Some("large") => (28.0, 28.0, 19.0),
                _ => (24.0, 24.0, 17.0),
            };
            Some(Metrics {
                width: text_width.map(|w| {
                    if title.is_empty() {
                        padding + 10.0
                    } else {
                        w + padding
                    }
                }),
                height,
                baseline,
            })
        }
        "popUpButton" if cell.attr("bezelStyle") == Some("recessed") => Some(Metrics {
            width: text_width.map(|w| w + 48.0),
            height: 18.0,
            baseline: 13.0,
        }),
        "popUpButton" => Some(Metrics {
            width: None,
            height: match size {
                Some("small") => 20.0,
                Some("mini") => 16.0,
                _ => 24.0,
            },
            baseline: match size {
                Some("small") => 14.0,
                Some("mini") => 12.0,
                _ => 16.75,
            },
        }),
        "textField" | "secureTextField" => text_field(view, cell, font, &title, text_width, size),
        _ => None,
    }
}

fn text_field(
    view: &Element,
    cell: &Element,
    font: &Font,
    title: &str,
    text_width: Option<f64>,
    size: Option<&str>,
) -> Option<Metrics> {
    let bezel = cell.attr("borderStyle") == Some("bezel");
    let height = if bezel {
        font.height + if size == Some("mini") { 9.0 } else { 8.0 }
    } else {
        let available = view
            .child("rect")
            .and_then(|r| r.attr("width"))
            .and_then(|s| s.parse::<f64>().ok())?
            - 4.0;
        if cell
            .attr("lineBreakMode")
            .is_some_and(|mode| mode != "wordWrapping" && mode != "charWrapping")
            || super::super::yes(cell, "scrollable")
            || super::super::yes(cell, "usesSingleLineMode")
        {
            font.height
        } else {
            font.height * lines(font, title, available)?
        }
    };
    Some(Metrics {
        width: if bezel {
            None
        } else {
            text_width.map(|w| {
                w + if cell.attr("alignment") == Some("center") {
                    8.0
                } else {
                    4.0
                }
            })
        },
        height,
        baseline: height - if bezel { 7.0 } else { 3.0 },
    })
}

pub(super) fn inset(view: &Element) -> f64 {
    if matches!(view.name.as_str(), "textField" | "secureTextField")
        && view
            .children
            .iter()
            .find(|c| c.attr("key") == Some("cell"))
            .is_some_and(|c| c.attr("borderStyle") != Some("bezel"))
    {
        2.0
    } else {
        0.0
    }
}

fn lines(font: &Font, text: &str, available: f64) -> Option<f64> {
    if available <= 0.0 {
        return None;
    }
    let mut count = 0.0;
    for paragraph in text.split('\n') {
        count += 1.0;
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let joined = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && width(font, &joined)? > available {
                count += 1.0;
                line = word.into();
            } else {
                line = joined;
            }
        }
    }
    Some(count)
}

pub(super) fn container(view: &Element) -> Option<Metrics> {
    if view.name == "gridView" {
        let mut height = 0.0;
        let rows = &view.child("rows")?.children;
        let cells = &view.child("gridCells")?.children;
        for row in rows {
            let mut maximum = 0.0_f64;
            for cell in cells
                .iter()
                .filter(|cell| cell.attr("row") == row.attr("id"))
            {
                let child = super::super::grids::content(cell)?;
                maximum = maximum.max(control(child)?.height);
            }
            height += maximum;
        }
        height += number(view, "rowSpacing", 6.0).ok()?
            * f64::from(u32::try_from(rows.len().saturating_sub(1)).ok()?);
        return Some(Metrics {
            width: None,
            height,
            baseline: 0.0,
        });
    }
    let children = &view.child("subviews")?.children;
    let horizontal = view.attr("orientation") != Some("vertical");
    let mut sizes = Vec::new();
    for child in children {
        let metrics = control(child);
        let rect = child.child("rect")?;
        let mut size = [
            metrics
                .as_ref()
                .and_then(|m| m.width)
                .unwrap_or(number(rect, "width", 0.0).ok()?),
            metrics.map_or(number(rect, "height", 0.0).ok()?, |m| m.height),
        ];
        if let Some(constraints) = child.child("constraints") {
            for c in &constraints.children {
                if c.attr("secondItem").is_none() && c.attr("relation").is_none() {
                    match c.attr("firstAttribute") {
                        Some("width") => size[0] = number(c, "constant", size[0]).ok()?,
                        Some("height") => size[1] = number(c, "constant", size[1]).ok()?,
                        _ => {}
                    }
                }
            }
        }
        size[0] -= 2.0 * inset(child);
        sizes.push(size);
    }
    let mut size = [0.0_f64; 2];
    for s in sizes {
        if horizontal {
            size[0] += s[0];
            size[1] = size[1].max(s[1]);
        } else {
            size[0] = size[0].max(s[0]);
            size[1] += s[1];
        }
    }
    size[usize::from(!horizontal)] += number(view, "spacing", 8.0).ok()?
        * f64::from(u32::try_from(children.len().saturating_sub(1)).ok()?);
    Some(Metrics {
        width: Some(size[0]),
        height: size[1],
        baseline: 0.0,
    })
}

pub(super) fn vertical_inset(view: &Element) -> f64 {
    if view.name == "imageView"
        && view
            .child("imageCell")
            .is_some_and(|c| c.attr("image") == Some("NSActionTemplate"))
    {
        5.0
    } else if view.name == "box" && view.attr("boxType") == Some("separator") {
        2.0
    } else if view.name == "button"
        && view.child("buttonCell").is_some_and(|c| {
            c.attr("bezelStyle") == Some("smallSquare") && c.attr("borderStyle") == Some("border")
        })
    {
        1.0
    } else {
        0.0
    }
}
