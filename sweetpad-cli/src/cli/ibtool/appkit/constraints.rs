use super::{Element, Encoder, Id, number};

fn attribute(value: &str) -> Result<i64, String> {
    match value {
        "notAnAttribute" => Ok(0),
        "left" => Ok(1),
        "right" => Ok(2),
        "top" => Ok(3),
        "bottom" => Ok(4),
        "leading" => Ok(5),
        "trailing" => Ok(6),
        "width" => Ok(7),
        "height" => Ok(8),
        "centerX" => Ok(9),
        "centerY" => Ok(10),
        "baseline" | "lastBaseline" => Ok(11),
        "firstBaseline" => Ok(12),
        other => Err(format!("unsupported layout attribute {other}")),
    }
}

impl Encoder<'_> {
    #[allow(clippy::cast_possible_truncation, clippy::float_cmp)]
    pub(super) fn constraint(&mut self, element: &Element, parent: Id) -> Result<Id, String> {
        let object = self.object(element)?;
        self.owners.push((object, parent));
        self.oids.push(object);
        let first = element
            .attr("firstItem")
            .map_or(Ok(parent), |id| self.lookup(id))?;
        let first_attribute = attribute(
            element
                .attr("firstAttribute")
                .ok_or("a constraint has no first attribute")?,
        )?;
        self.graph.set(object, "NSFirstItem", first);
        let relation = match element.attr("relation") {
            None | Some("equal") => 0_i64,
            Some("lessThanOrEqual") => -1,
            Some("greaterThanOrEqual") => 1,
            Some(other) => return Err(format!("unsupported constraint relation {other}")),
        };
        if relation != 0 {
            self.graph.set(object, "NSRelation", relation);
        }
        self.graph
            .set(object, "NSFirstAttributeV2", first_attribute);
        self.graph.set(object, "NSFirstAttribute", first_attribute);
        if let Some(second) = element.attr("secondItem") {
            if let Some(multiplier) = element.attr("multiplier") {
                let multiplier = if let Some((numerator, denominator)) = multiplier.split_once(':')
                {
                    numerator
                        .parse::<f64>()
                        .map_err(|_| "invalid multiplier numerator")?
                        / denominator
                            .parse::<f64>()
                            .map_err(|_| "invalid multiplier denominator")?
                } else {
                    f64::from(
                        multiplier
                            .parse::<f32>()
                            .map_err(|_| "invalid constraint multiplier")?,
                    )
                };
                if !multiplier.is_finite() {
                    return Err("a constraint multiplier must be finite".into());
                }
                if multiplier != 1.0 {
                    self.graph.set(object, "NSMultiplier", multiplier);
                }
            }
            let second = self.lookup(second)?;
            let second_attribute =
                attribute(element.attr("secondAttribute").unwrap_or("notAnAttribute"))?;
            self.graph.set(object, "NSSecondItem", second);
            self.graph
                .set(object, "NSSecondAttributeV2", second_attribute);
            self.graph
                .set(object, "NSSecondAttribute", second_attribute);
        }
        if super::yes(element, "symbolic") {
            self.string(object, "NSSymbolicConstant", "NSSpace");
        } else if element.attr("constant").is_some() {
            let constant = number(element, "constant", 0.0)?;
            if constant != 0.0 {
                self.graph.set(object, "NSConstantV2", constant);
                self.graph.set(object, "NSConstant", constant);
            }
        }
        self.graph.set(object, "NSShouldBeArchived", true);
        let priority = number(element, "priority", 1000.0)? as i64;
        if priority != 1000 {
            self.graph.set(object, "NSPriority", priority);
        }
        if let Some(identifier) = element.attr("identifier") {
            self.string(object, "NSLayoutIdentifier", identifier);
        }
        Ok(object)
    }
}
