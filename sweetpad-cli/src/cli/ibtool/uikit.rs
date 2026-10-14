//! The UIKit objects an Interface Builder document describes, archived the
//! way their classes' `encodeWithCoder:` archives them: views, labels,
//! image views, Auto Layout constraints and the safe area, with their
//! colors and fonts.
//!
//! An element or attribute this doesn't know is an error naming it, never
//! a nib that leaves it out.

use std::collections::HashMap;

use sweetpad_lib::xcscheme::Element;

use super::nib::{Graph, Id, Value, doubles};

/// Encodes one nib's objects, from the document they're in.
pub(super) struct Encoder<'a> {
    pub graph: Graph,
    /// The fallback values of the system colors the document uses.
    system_colors: HashMap<&'a str, &'a Element>,
    /// The archived object of each element, by its Interface Builder id.
    ids: HashMap<String, Id>,
    /// The view each safe area layout guide belongs to, by the guide's id.
    guide_owners: HashMap<String, String>,
    /// The `UIViewSafeAreaLayoutGuide` of each view a constraint pins to
    /// its safe area, by the view's id.
    safe_areas: HashMap<String, Id>,
    /// What the nib's objects list holds besides its placeholders: each
    /// view, its layout guides and constraints, then its subviews.
    pub objects: Vec<Id>,
}

/// The value `key` of an element's `rect`, `color` or other child.
fn keyed<'e>(element: &'e Element, name: &'e str, key: &str) -> Option<&'e Element> {
    element
        .children_named(name)
        .find(|child| child.attr("key") == Some(key))
}

fn yes(element: &Element, attribute: &str) -> bool {
    element.attr(attribute) == Some("YES")
}

fn no(element: &Element, attribute: &str) -> bool {
    element.attr(attribute) == Some("NO")
}

fn number(element: &Element, attribute: &str) -> Result<Option<f64>, String> {
    element
        .attr(attribute)
        .map(|text| {
            text.parse::<f64>()
                .map_err(|_| format!("{attribute}=\"{text}\" isn't a number"))
        })
        .transpose()
}

fn integer(element: &Element, attribute: &str) -> Result<Option<i64>, String> {
    element
        .attr(attribute)
        .map(|text| {
            text.parse::<i64>()
                .map_err(|_| format!("{attribute}=\"{text}\" isn't a whole number"))
        })
        .transpose()
}

fn id_of(element: &Element) -> Result<&str, String> {
    element
        .attr("id")
        .ok_or_else(|| format!("a <{}> has no id", element.name))
}

/// The UIKit class of a view element.
fn view_class(element: &Element) -> Result<&'static str, String> {
    if let Some(custom) = element.attr("customClass") {
        return Err(format!(
            "<{}> {} has the custom class {custom}, which SweetPad's ibtool doesn't compile yet",
            element.name,
            element.attr("id").unwrap_or("")
        ));
    }
    match element.name.as_str() {
        "view" => Ok("UIView"),
        "label" => Ok("UILabel"),
        "imageView" => Ok("UIImageView"),
        other => Err(format!(
            "<{other}> isn't a view SweetPad's ibtool compiles yet"
        )),
    }
}

/// The view elements of `subviews`, in order.
fn subviews(element: &Element) -> impl Iterator<Item = &Element> {
    element
        .children_named("subviews")
        .flat_map(|subviews| subviews.children.iter())
}

/// The elements a view may have that encode nothing: Interface Builder's
/// own bookkeeping.
const IGNORED: &[&str] = &["point", "nil", "accessibility"];

impl<'a> Encoder<'a> {
    pub fn new(document: &'a Element) -> Self {
        let system_colors = document
            .children_named("resources")
            .flat_map(|resources| resources.children_named("systemColor"))
            .filter_map(|color| Some((color.attr("name")?, color.child("color")?)))
            .collect();
        Encoder {
            graph: Graph::default(),
            system_colors,
            ids: HashMap::new(),
            guide_owners: HashMap::new(),
            safe_areas: HashMap::new(),
            objects: Vec::new(),
        }
    }

    /// The archived view `element` and everything below it.
    pub fn root_view(&mut self, element: &Element) -> Result<Id, String> {
        self.declare(element)?;
        self.view(element)
    }

    /// Gives each view below `element` its object, so a constraint can
    /// refer to a view it comes before.
    fn declare(&mut self, element: &Element) -> Result<(), String> {
        let class = view_class(element)?;
        let id = id_of(element)?.to_string();
        for guide in element.children_named("viewLayoutGuide") {
            if guide.attr("key") == Some("safeArea") {
                self.guide_owners
                    .insert(id_of(guide)?.to_string(), id.clone());
            }
        }
        let object = self.graph.object(class);
        self.ids.insert(id, object);
        for subview in subviews(element) {
            self.declare(subview)?;
        }
        Ok(())
    }

    fn view(&mut self, element: &Element) -> Result<Id, String> {
        let id = id_of(element)?;
        let object = self.ids[id];
        self.objects.push(object);
        for child in &element.children {
            match child.name.as_str() {
                "rect" | "autoresizingMask" | "subviews" | "color" | "constraints"
                | "viewLayoutGuide" | "fontDescription" | "string" => {}
                name if IGNORED.contains(&name) => {}
                name => {
                    return Err(format!(
                        "<{name}> in <{}> {id} isn't something SweetPad's ibtool compiles yet",
                        element.name
                    ));
                }
            }
        }
        for guide in element.children_named("viewLayoutGuide") {
            if guide.attr("key") != Some("safeArea") {
                return Err(format!(
                    "the {} layout guide of {id} isn't something SweetPad's ibtool compiles yet",
                    guide.attr("key").unwrap_or("unnamed")
                ));
            }
            let placeholder = self.graph.object("UILayoutGuide");
            self.graph
                .set(placeholder, "UILayoutGuideOwningView", Value::Nil);
            let empty = self.graph.string("");
            self.graph
                .set(placeholder, "UILayoutGuideIdentifier", empty);
            self.graph
                .set(placeholder, "UILayoutGuideOwningViewIsLocked", false);
            self.graph
                .set(placeholder, "UILayoutGuideShouldBeArchived", false);
            self.graph
                .set(placeholder, "UILayoutGuideAllowsNegativeDimensions", false);
            self.objects.push(placeholder);
        }

        let frame = keyed(element, "rect", "frame")
            .ok_or_else(|| format!("<{}> {id} has no frame", element.name))?;
        let x = number(frame, "x")?.unwrap_or(0.0);
        let y = number(frame, "y")?.unwrap_or(0.0);
        let width = number(frame, "width")?.unwrap_or(0.0);
        let height = number(frame, "height")?.unwrap_or(0.0);
        self.graph
            .set(object, "UIBounds", doubles(&[0.0, 0.0, width, height]));
        self.graph.set(
            object,
            "UICenter",
            doubles(&[x + width / 2.0, y + height / 2.0]),
        );

        let constraints = self.constraint_objects(element)?;
        let children: Vec<&Element> = subviews(element).collect();
        if !children.is_empty() {
            let mut views = Vec::new();
            for child in &children {
                views.push(self.ids[id_of(child)?]);
            }
            let array = self.graph.array("NSMutableArray", &views);
            self.graph.set(object, "UISubviews", array);
        }
        if let Some(color) = keyed(element, "color", "backgroundColor") {
            let color = self.color(color)?;
            self.graph.set(object, "UIBackgroundColor", color);
        }
        self.appearance(element, object)?;
        if !constraints.is_empty() {
            let array = self.graph.array("NSMutableArray", &constraints);
            self.graph.set(object, "UIViewAutolayoutConstraints", array);
        }
        if let Some(&guide) = self.safe_areas.get(id) {
            let array = self.graph.array("NSMutableArray", &[guide]);
            self.graph.set(object, "UIViewLayoutGuides", array);
        }
        self.graph
            .set(object, "UIViewSemanticContentAttribute", 0_i64);
        self.graph
            .set(object, "UIViewLargeContentStoredProperties", Value::Nil);

        match element.name.as_str() {
            "label" => self.label(element, object)?,
            "imageView" => self.image_view(element, object)?,
            _ => {}
        }
        for child in children {
            self.view(child)?;
        }
        Ok(object)
    }

    /// How a view draws and takes part in layout: the flags and priorities
    /// it has in the document.
    fn appearance(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.graph.set(object, "UIDeepDrawRect", true);
        if !no(element, "opaque") {
            self.graph.set(object, "UIOpaque", true);
        }
        if no(element, "userInteractionEnabled") {
            self.graph.set(object, "UIUserInteractionDisabled", true);
        }
        if !no(element, "autoresizesSubviews") {
            self.graph.set(object, "UIAutoresizeSubviews", true);
        }
        self.graph
            .set(object, "UIAutoresizingMask", autoresizing_mask(element));
        let mode = content_mode(element)?;
        if mode != 0 {
            self.graph.set(object, "UIContentMode", mode);
        }
        if yes(element, "clipsSubviews") {
            self.graph.set(object, "UIClipsToBounds", true);
        }
        if no(element, "translatesAutoresizingMaskIntoConstraints") {
            self.graph.set(
                object,
                "UIViewDoesNotTranslateAutoresizingMaskIntoConstraints",
                true,
            );
        }
        for (key, horizontal, vertical) in [
            (
                "UIViewContentHuggingPriority",
                "horizontalHuggingPriority",
                "verticalHuggingPriority",
            ),
            (
                "UIViewContentCompressionResistancePriority",
                "horizontalCompressionResistancePriority",
                "verticalCompressionResistancePriority",
            ),
        ] {
            if element.attr(horizontal).is_some() || element.attr(vertical).is_some() {
                let default = if key == "UIViewContentHuggingPriority" {
                    250.0
                } else {
                    750.0
                };
                let h = number(element, horizontal)?.unwrap_or(default);
                let v = number(element, vertical)?.unwrap_or(default);
                let text = self.graph.string(&format!("{{{h}, {v}}}"));
                self.graph.set(object, key, text);
            }
        }
        Ok(())
    }

    /// The constraints `element` holds, which are in the objects list right
    /// after it.
    fn constraint_objects(&mut self, element: &Element) -> Result<Vec<Id>, String> {
        let owner = id_of(element)?;
        let mut constraints = Vec::new();
        for constraint in element
            .children_named("constraints")
            .flat_map(|list| list.children.iter())
        {
            let object = self.constraint(constraint, owner)?;
            self.objects.push(object);
            constraints.push(object);
        }
        Ok(constraints)
    }

    fn constraint(&mut self, element: &Element, owner: &str) -> Result<Id, String> {
        let id = id_of(element)?;
        if element.name != "constraint" {
            return Err(format!("<{}> {id} isn't a constraint", element.name));
        }
        for (attribute, _) in &element.attributes {
            if !matches!(
                attribute.as_str(),
                "id" | "firstItem"
                    | "firstAttribute"
                    | "secondItem"
                    | "secondAttribute"
                    | "constant"
                    | "multiplier"
                    | "relation"
                    | "priority"
                    | "identifier"
                    | "symbolic"
            ) {
                return Err(format!(
                    "the constraint {id}'s {attribute} isn't something SweetPad's ibtool compiles yet"
                ));
            }
        }
        let first = self.item(element.attr("firstItem").unwrap_or(owner))?;
        let first_attribute = layout_attribute(element.attr("firstAttribute"), id)?;
        let object = self.graph.object("NSLayoutConstraint");
        self.graph.set(object, "NSFirstItem", first);
        self.graph
            .set(object, "NSFirstAttributeV2", first_attribute);
        self.graph.set(object, "NSFirstAttribute", first_attribute);
        let relation = match element.attr("relation") {
            None => 0,
            Some("lessThanOrEqual") => -1,
            Some("greaterThanOrEqual") => 1,
            Some(other) => {
                return Err(format!("the constraint {id}'s relation {other} is unknown"));
            }
        };
        if relation != 0 {
            self.graph.set(object, "NSRelation", relation);
        }
        if let Some(multiplier) = element.attr("multiplier") {
            let value = ratio(multiplier).ok_or_else(|| {
                format!("the constraint {id}'s multiplier {multiplier} isn't a number")
            })?;
            #[allow(clippy::cast_possible_truncation)]
            let value = f64::from(value as f32);
            if (value - 1.0).abs() > f64::EPSILON {
                self.graph.set(object, "NSMultiplier", value);
            }
        }
        if let Some(second) = element.attr("secondItem") {
            let second = self.item(second)?;
            let second_attribute = layout_attribute(element.attr("secondAttribute"), id)?;
            self.graph.set(object, "NSSecondItem", second);
            self.graph
                .set(object, "NSSecondAttributeV2", second_attribute);
            self.graph
                .set(object, "NSSecondAttribute", second_attribute);
        }
        // A symbolic constraint is Interface Builder's standard spacing,
        // which UIKit works out itself, so its constant is the system's.
        let symbolic = match element.attr("symbolic") {
            None | Some("NO") => false,
            Some("YES") => true,
            Some(other) => {
                return Err(format!("the constraint {id}'s symbolic {other} is unknown"));
            }
        };
        let constant = if symbolic {
            let space = self.graph.string("NSSpace");
            self.graph.set(object, "NSSymbolicConstant", space);
            0.0
        } else {
            number(element, "constant")?.unwrap_or(0.0)
        };
        if constant != 0.0 {
            self.graph.set(object, "NSConstantV2", constant);
        }
        if let Some(priority) = integer(element, "priority")?
            && priority != 1000
        {
            self.graph.set(object, "NSPriority", priority);
        }
        self.graph.set(object, "NSShouldBeArchived", true);
        if constant != 0.0 {
            self.graph.set(object, "NSConstant", constant);
        }
        if let Some(identifier) = element.attr("identifier") {
            let identifier = self.graph.string(identifier);
            self.graph.set(object, "NSLayoutIdentifier", identifier);
        }
        Ok(object)
    }

    /// The object a constraint names: a view, or a view's safe area.
    fn item(&mut self, ib_id: &str) -> Result<Id, String> {
        if let Some(&object) = self.ids.get(ib_id) {
            return Ok(object);
        }
        let Some(owner) = self.guide_owners.get(ib_id).cloned() else {
            return Err(format!(
                "a constraint refers to {ib_id}, which isn't in this view"
            ));
        };
        if let Some(&guide) = self.safe_areas.get(&owner) {
            return Ok(guide);
        }
        let view = self.ids[&owner];
        let guide = self.graph.object("UILayoutGuide");
        self.safe_areas.insert(owner, guide);
        self.graph.set(guide, "UILayoutGuideOwningView", view);
        let identifier = self.graph.string("UIViewSafeAreaLayoutGuide");
        self.graph.set(guide, "UILayoutGuideIdentifier", identifier);
        self.graph
            .set(guide, "UILayoutGuideOwningViewIsLocked", true);
        self.graph.set(guide, "UILayoutGuideShouldBeArchived", true);
        self.graph
            .set(guide, "UILayoutGuideAllowsNegativeDimensions", true);
        // What pins the guide to the view's edges until UIKit insets it.
        let mut system = Vec::new();
        for (edge, attribute, guide_first) in [
            ("top", 3, true),
            ("left", 1, true),
            ("bottom", 4, false),
            ("right", 2, false),
        ] {
            let (first, second) = if guide_first {
                (guide, view)
            } else {
                (view, guide)
            };
            let constraint = self.graph.object("NSLayoutConstraint");
            self.graph.set(constraint, "NSFirstItem", first);
            self.graph
                .set(constraint, "NSFirstAttributeV2", i64::from(attribute));
            self.graph
                .set(constraint, "NSFirstAttribute", i64::from(attribute));
            self.graph.set(constraint, "NSSecondItem", second);
            self.graph
                .set(constraint, "NSSecondAttributeV2", i64::from(attribute));
            self.graph
                .set(constraint, "NSSecondAttribute", i64::from(attribute));
            self.graph.set(constraint, "NSShouldBeArchived", false);
            let name = self
                .graph
                .string(&format!("UIViewSafeAreaLayoutGuide-{edge}"));
            self.graph.set(constraint, "NSLayoutIdentifier", name);
            system.push(constraint);
        }
        let system = self.graph.array("NSArray", &system);
        self.graph
            .set(guide, "UILayoutGuideSystemConstraints", system);
        Ok(guide)
    }

    fn label(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let text = element
            .attr("text")
            .map(str::to_string)
            .or_else(|| keyed(element, "string", "text").map(|string| string.text.clone()));
        if let Some(text) = text {
            let text = self.graph.string(&text);
            self.graph.set(object, "UIText", text);
        }
        let description = keyed(element, "fontDescription", "fontDescription");
        let size = description
            .map(|d| number(d, "pointSize"))
            .transpose()?
            .flatten()
            .unwrap_or(17.0);
        let font = self.font(description)?;
        self.graph.set(object, "UIFont", font);
        let color = match keyed(element, "color", "textColor") {
            Some(color) => self.color(color)?,
            None => self.system_color("labelColor", Fallback::White(0.0, 1.0)),
        };
        self.graph.set(object, "UITextColor", color);
        self.graph
            .set(object, "UIShadowOffset", doubles(&[0.0, -1.0]));
        let lines = integer(element, "numberOfLines")?.unwrap_or(1);
        if lines != 1 {
            self.graph.set(object, "UINumberOfLines", lines);
        }
        let minimum_size = number(element, "minimumFontSize")?;
        let adjusts = !no(element, "adjustsFontSizeToFit")
            && (minimum_size.is_some() || element.attr("minimumScaleFactor").is_some());
        if adjusts {
            self.graph.set(object, "UIAdjustsFontSizeToFit", true);
        }
        if let Some(minimum) = minimum_size {
            #[allow(clippy::cast_possible_truncation)]
            self.graph.set(object, "UIMinimumFontSize", minimum as f32);
        }
        let line_break = match element.attr("lineBreakMode") {
            None | Some("tailTruncation") => 4,
            Some("wordWrap") => 0,
            Some("characterWrap") => 1,
            Some("clip") => 2,
            Some("headTruncation") => 3,
            Some("middleTruncation") => 5,
            Some(other) => return Err(format!("the line break mode {other} is unknown")),
        };
        if line_break != 4 {
            self.graph.set(object, "UILineBreakMode", line_break);
        }
        let alignment = match element.attr("textAlignment") {
            None | Some("natural") => 4,
            Some("left") => 0,
            Some("center") => 1,
            Some("right") => 2,
            Some("justified") => 3,
            Some(other) => return Err(format!("the text alignment {other} is unknown")),
        };
        if alignment != 4 {
            self.graph.set(object, "UITextAlignment", alignment);
        }
        let scale = match (number(element, "minimumScaleFactor")?, minimum_size) {
            (Some(factor), _) => Some(factor),
            (None, Some(minimum)) => Some(minimum / size),
            (None, None) => None,
        };
        if let Some(scale) = scale {
            #[allow(clippy::cast_possible_truncation)]
            self.graph.set(object, "UIMinimumScaleFactor", scale as f32);
        }
        self.graph.set(
            object,
            "UIDisableUpdateTextColorOnTraitCollectionChange",
            true,
        );
        Ok(())
    }

    fn image_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(name) = element.attr("image") {
            if element.attr("catalog").is_some() {
                return Err(format!(
                    "the image {name} comes from a symbol catalog, which SweetPad's ibtool doesn't compile yet"
                ));
            }
            let image = self.graph.object("UIImageNibPlaceholder");
            self.graph.set(image, "UIImageWidth", 1.0_f32);
            self.graph.set(image, "UIImageHeight", 1.0_f32);
            let name = self.graph.string(name);
            self.graph.set(image, "UIResourceName", name);
            self.graph
                .set(image, "UISystemSymbolResourceName", Value::Nil);
            self.graph
                .set(image, "UISymbolImageConfiguration", Value::Nil);
            self.graph.set(image, "UIResourceCatalogName", Value::Nil);
            self.graph.set(object, "UIImage", image);
        }
        Ok(())
    }

    /// A system font, or an error for a font this doesn't know.
    fn font(&mut self, description: Option<&Element>) -> Result<Id, String> {
        let kind = description.and_then(|d| d.attr("type")).unwrap_or("system");
        let weight = description.and_then(|d| d.attr("weight"));
        let size = description
            .map(|d| number(d, "pointSize"))
            .transpose()?
            .flatten()
            .unwrap_or(17.0);
        let (name, usage, override_usage, traits) = match (kind, weight) {
            ("system", None | Some("regular")) => (".SFUI-Regular", "CTFontRegularUsage", None, 0),
            ("system", Some("light")) => (".SFUI-Light", "CTFontLightUsage", None, 8),
            ("boldSystem", None) => (
                ".SFUI-Bold",
                "CTFontEmphasizedUsage",
                Some("CTFontBoldUsage"),
                2,
            ),
            _ => {
                return Err(format!(
                    "the font {kind}{} isn't one SweetPad's ibtool compiles yet",
                    weight.map(|w| format!(" {w}")).unwrap_or_default()
                ));
            }
        };
        let usage_key = self.graph.string("NSCTFontUIUsageAttribute");
        let usage = self.graph.string(usage);
        let mut entries = vec![(usage_key, usage)];
        if let Some(bold) = override_usage {
            let key = self.graph.string("NSCTFontUIUsageOverrideAttribute");
            let value = self.graph.string(bold);
            entries.push((key, value));
        }
        let size_key = self.graph.string("NSFontSizeAttribute");
        let size_number = self.graph.number(size);
        entries.push((size_key, size_number));
        // UIKit derives a bold font's descriptor from the regular one's,
        // which leaves it a mutable copy.
        let class = if override_usage.is_some() {
            "NSMutableDictionary"
        } else {
            "NSDictionary"
        };
        let attributes = self.graph.dictionary(class, &entries);
        let descriptor = self.graph.shared(
            "UIFontDescriptor",
            vec![
                ("UIFontDescriptorAttributes", Value::Object(attributes)),
                ("UIFontDescriptorOptions", Value::Int(2_147_517_444)),
            ],
        );
        let name = self.graph.string(name);
        Ok(self.graph.shared(
            "UIFont",
            vec![
                ("UIFontName", Value::Object(name)),
                ("UIFontDescriptor", Value::Object(descriptor)),
                ("UIFontPointSize", Value::Double(size)),
                ("UIFontTextStyleForScaling", Value::Nil),
                ("UIFontPointSizeForScaling", Value::Double(0.0)),
                ("UIFontMaximumPointSizeAfterScaling", Value::Double(0.0)),
                ("UIFontTraits", Value::Int(traits)),
                ("UISystemFont", Value::Bool(true)),
                ("NSName", Value::Object(name)),
                ("NSSize", Value::Double(size)),
            ],
        ))
    }

    /// The color a `<color>` element describes.
    fn color(&mut self, element: &Element) -> Result<Id, String> {
        if let Some(name) = element.attr("systemColor") {
            let fallback = match self.system_colors.get(name) {
                Some(color) => components(color)?,
                None => {
                    return Err(format!(
                        "the system color {name} has no fallback in the document"
                    ));
                }
            };
            return Ok(self.system_color(name, fallback));
        }
        if let Some(name) = element.attr("xcode11CocoaTouchSystemColor") {
            let fallback = legacy_system_color(element.attr("cocoaTouchSystemColor"))?;
            return Ok(self.system_color(name, fallback));
        }
        if let Some(name) = element.attr("cocoaTouchSystemColor") {
            return Err(format!(
                "the system color {name} isn't one SweetPad's ibtool compiles yet"
            ));
        }
        if let Some(name) = element.attr("name") {
            return Err(format!(
                "the named color {name} isn't something SweetPad's ibtool compiles yet"
            ));
        }
        let fallback = components(element)?;
        let values = color_values(fallback);
        Ok(self.graph.shared("UIColor", values))
    }

    fn system_color(&mut self, name: &str, fallback: Fallback) -> Id {
        let name = self.graph.string(name);
        let mut values = vec![("UISystemColorName", Value::Object(name))];
        values.extend(color_values(fallback));
        self.graph.shared("UIColor", values)
    }
}

/// What a color archives of its components.
fn color_values(fallback: Fallback) -> Vec<(&'static str, Value)> {
    match fallback {
        Fallback::White(white, alpha) => {
            let text = color_text(&[white], alpha);
            vec![
                ("UIColorComponentCount", Value::Int(2)),
                ("UIWhite", Value::Float(single(white))),
                ("UIAlpha", Value::Float(single(alpha))),
                ("NSWhite", Value::Data(text.into_bytes())),
                ("NSColorSpace", Value::Int(4)),
            ]
        }
        Fallback::Rgb(red, green, blue, alpha) => {
            let mut values = vec![("UIColorComponentCount", Value::Int(4))];
            for (key, double_key, value) in [
                ("UIRed", "UIRed-Double", red),
                ("UIGreen", "UIGreen-Double", green),
                ("UIBlue", "UIBlue-Double", blue),
                ("UIAlpha", "UIAlpha-Double", alpha),
            ] {
                let value = rounded(value);
                values.push((key, Value::Float(single(value))));
                if f64::from(single(value)).to_bits() != value.to_bits() {
                    values.push((double_key, Value::Double(value)));
                }
            }
            let text = color_text(&[red, green, blue], alpha);
            values.push(("NSRGB", Value::Data(text.into_bytes())));
            values.push(("NSColorSpace", Value::Int(2)));
            values
        }
    }
}

/// A color's components where it has no system color to resolve to.
#[derive(Debug, Clone, Copy)]
enum Fallback {
    White(f64, f64),
    Rgb(f64, f64, f64, f64),
}

fn components(element: &Element) -> Result<Fallback, String> {
    let space = element
        .attr("customColorSpace")
        .or_else(|| element.attr("colorSpace"))
        .unwrap_or("");
    let alpha = number(element, "alpha")?.unwrap_or(1.0);
    match space {
        "sRGB" => Ok(Fallback::Rgb(
            number(element, "red")?.unwrap_or(0.0),
            number(element, "green")?.unwrap_or(0.0),
            number(element, "blue")?.unwrap_or(0.0),
            alpha,
        )),
        "genericGamma22GrayColorSpace" => Ok(Fallback::White(
            number(element, "white")?.unwrap_or(0.0),
            alpha,
        )),
        other => Err(format!(
            "the color space {other} isn't one SweetPad's ibtool compiles yet"
        )),
    }
}

/// The color an Xcode 10 document gives a system color for older iOS.
fn legacy_system_color(name: Option<&str>) -> Result<Fallback, String> {
    match name {
        Some("whiteColor") => Ok(Fallback::White(1.0, 1.0)),
        Some("blackColor") => Ok(Fallback::White(0.0, 1.0)),
        Some("clearColor") => Ok(Fallback::White(0.0, 0.0)),
        Some(other) => Err(format!(
            "the system color {other} isn't one SweetPad's ibtool compiles yet"
        )),
        None => Err("a system color has no fallback".to_string()),
    }
}

/// A component as Interface Builder keeps it: to ten decimal places.
fn rounded(value: f64) -> f64 {
    (value * 1e10).round() / 1e10
}

#[allow(clippy::cast_possible_truncation)]
fn single(value: f64) -> f32 {
    value as f32
}

/// AppKit's text for a color's components: three significant digits each,
/// the alpha only when it isn't 1.
fn color_text(components: &[f64], alpha: f64) -> String {
    let mut parts: Vec<String> = components.iter().map(|&c| short(c)).collect();
    if (alpha - 1.0).abs() > f64::EPSILON {
        parts.push(short(alpha));
    }
    parts.join(" ")
}

fn short(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    #[allow(clippy::cast_possible_truncation)]
    let digits = (2 - value.abs().log10().floor() as i32).max(0);
    let text = format!("{value:.*}", usize::try_from(digits).unwrap_or(0));
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// `1/3`, `16:9` or a plain number.
fn ratio(text: &str) -> Option<f64> {
    if let Some((a, b)) = text.split_once(['/', ':']) {
        return Some(a.trim().parse::<f64>().ok()? / b.trim().parse::<f64>().ok()?);
    }
    text.parse().ok()
}

fn layout_attribute(name: Option<&str>, constraint: &str) -> Result<i64, String> {
    Ok(match name {
        Some("left") => 1,
        Some("right") => 2,
        Some("top") => 3,
        Some("bottom") => 4,
        Some("leading") => 5,
        Some("trailing") => 6,
        Some("width") => 7,
        Some("height") => 8,
        Some("centerX") => 9,
        Some("centerY") => 10,
        Some("baseline" | "lastBaseline") => 11,
        Some("firstBaseline") => 12,
        Some("leftMargin") => 13,
        Some("rightMargin") => 14,
        Some("topMargin") => 15,
        Some("bottomMargin") => 16,
        Some("leadingMargin") => 17,
        Some("trailingMargin") => 18,
        Some("centerXWithinMargins") => 19,
        Some("centerYWithinMargins") => 20,
        Some(other) => {
            return Err(format!(
                "the constraint {constraint}'s attribute {other} is unknown"
            ));
        }
        None => return Err(format!("the constraint {constraint} has no attribute")),
    })
}

fn autoresizing_mask(element: &Element) -> i64 {
    let Some(mask) = keyed(element, "autoresizingMask", "autoresizingMask") else {
        return 36;
    };
    [
        ("flexibleMinX", 1),
        ("widthSizable", 2),
        ("flexibleMaxX", 4),
        ("flexibleMinY", 8),
        ("heightSizable", 16),
        ("flexibleMaxY", 32),
    ]
    .iter()
    .filter(|(attribute, _)| yes(mask, attribute))
    .map(|(_, bit)| bit)
    .sum()
}

fn content_mode(element: &Element) -> Result<i64, String> {
    Ok(match element.attr("contentMode") {
        None | Some("scaleToFill") => 0,
        Some("scaleAspectFit") => 1,
        Some("scaleAspectFill") => 2,
        Some("redraw") => 3,
        Some("center") => 4,
        Some("top") => 5,
        Some("bottom") => 6,
        Some("left") => 7,
        Some("right") => 8,
        Some("topLeft") => 9,
        Some("topRight") => 10,
        Some("bottomLeft") => 11,
        Some("bottomRight") => 12,
        Some(other) => return Err(format!("the content mode {other} is unknown")),
    })
}
