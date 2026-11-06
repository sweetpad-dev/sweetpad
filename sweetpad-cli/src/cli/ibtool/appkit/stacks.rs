use super::{Element, Encoder, Id, Value, number, yes};

// AppKit archives stack priorities and spacing as IEEE single precision.
#[allow(clippy::cast_possible_truncation)]
fn scalar(element: &Element, key: &str, default: f64) -> Result<f32, String> {
    Ok(number(element, key, default)? as f32)
}

impl Encoder<'_> {
    pub(super) fn stack(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let vertical = element.attr("orientation") != Some("horizontal");
        self.graph
            .set(object, "NSStackViewOrientation", i64::from(vertical));
        let alignment =
            match element
                .attr("alignment")
                .unwrap_or(if vertical { "centerX" } else { "centerY" })
            {
                "left" => 1_i64,
                "right" => 2,
                "top" => 3,
                "bottom" => 4,
                "leading" => 5,
                "trailing" => 6,
                "centerX" => 9,
                "centerY" => 10,
                "baseline" | "lastBaseline" => 11,
                "firstBaseline" => 12,
                other => return Err(format!("unsupported stack alignment {other}")),
            };
        let secondary = match alignment {
            1 | 3 | 5 => 1_i64,
            2 | 4 | 6 => 4,
            9 | 10 => 3,
            11 => 5,
            12 => 2,
            _ => unreachable!(),
        };
        self.graph
            .set(object, "NSStackViewSecondaryAlignment", secondary);
        self.graph.set(object, "NSStackViewAlignment", alignment);
        self.stack_metrics(element, object)?;
        let distribution = match element.attr("distribution") {
            None | Some("gravityAreas") => -1_i64,
            Some("fill") => 0,
            Some("fillEqually") => 1,
            Some("fillProportionally") => 2,
            Some("equalSpacing") => 3,
            Some("equalCentering") => 4,
            Some(other) => return Err(format!("unsupported stack distribution {other}")),
        };
        if distribution == -1 {
            self.graph.set(
                object,
                "NSStackViewHasEqualSpacing",
                yes(element, "hasEqualSpacing"),
            );
        } else {
            self.graph
                .set(object, "NSStackViewdistribution", distribution);
        }
        self.stack_insets(element, object)?;
        let views = element.child("subviews").map_or(Ok(Vec::new()), |s| {
            s.children
                .iter()
                .map(|e| self.object(e))
                .collect::<Result<Vec<_>, _>>()
        })?;
        if distribution == -1 || !views.is_empty() {
            let container = self.stack_container(element, object, &views)?;
            self.graph
                .set(object, "NSStackViewBeginningContainer", container);
        }
        if distribution == -1 {
            for key in ["NSStackViewMiddleContainer", "NSStackViewEndContainer"] {
                let container = self.stack_container(element, object, &[])?;
                self.graph.set(object, key, container);
            }
        }
        self.graph.set(
            object,
            "NSStackViewDetachesHiddenViews",
            yes(element, "detachesHiddenViews"),
        );
        self.graph
            .set(object, "NSStackViewHasFlatViewHierarchy", true);
        if !views.is_empty() {
            let connector = self.graph.object("NSNibConnector");
            self.graph.set(connector, "NSSource", object);
            self.string(connector,"NSLabel","Encoding NSStackView requires being decoded before other connections with an early decoding order priority of 999990.");
            self.early_connections.push(connector);
        }
        Ok(())
    }

    fn stack_metrics(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for (attribute, key, default) in [
            (
                "verticalClippingResistancePriority",
                "NSStackViewVerticalClippingResistance",
                1000.0,
            ),
            (
                "horizontalClippingResistancePriority",
                "NSStackViewHorizontalClippingResistance",
                1000.0,
            ),
            (
                "verticalStackHuggingPriority",
                "NSStackViewVerticalHugging",
                249.0,
            ),
            (
                "horizontalStackHuggingPriority",
                "NSStackViewHorizontalHugging",
                249.0,
            ),
            ("spacing", "NSStackViewSpacing", 8.0),
        ] {
            let value = scalar(element, attribute, default)?;
            self.graph.set(object, key, value);
        }
        Ok(())
    }

    fn stack_insets(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for (attribute, key) in [
            ("top", "NSStackViewEdgeInsets.top"),
            ("left", "NSStackViewEdgeInsets.left"),
            ("right", "NSStackViewEdgeInsets.right"),
            ("bottom", "NSStackViewEdgeInsets.bottom"),
        ] {
            let value = element
                .child("edgeInsets")
                .map_or(Ok(0.0_f32), |e| scalar(e, attribute, 0.0))?;
            self.graph.set(object, key, value);
        }
        Ok(())
    }

    fn stack_container(
        &mut self,
        element: &Element,
        stack: Id,
        views: &[Id],
    ) -> Result<Id, String> {
        let container = self.graph.object("NSStackViewContainer");
        for key in [
            "NSNextResponder",
            "NSNibTouchBar",
            "IBNSSafeAreaLayoutGuide",
            "IBNSLayoutMarginsGuide",
            "NSStackViewContainerViewToCustomAfterSpaceMap",
        ] {
            self.graph.set(container, key, Value::Nil);
        }
        self.graph.set(container, "NSvFlags", 256_i64);
        self.string(container, "NSFrameSize", "{0, 0}");
        self.graph
            .set(container, "NSViewWantsBestResolutionOpenGLSurface", true);
        self.graph
            .set(container, "NSDoNotTranslateAutoresizingMask", true);
        self.graph.set(container, "IBNSClipsToBounds", 0_i64);
        self.graph
            .set(container, "NSStackViewContainerStackView", stack);
        let mut priorities = Vec::new();
        if let Some(list) = element.child("visibilityPriorities") {
            for (&view, priority) in views.iter().zip(&list.children) {
                let value = number(priority, "value", 1000.0)?;
                if value.to_bits() != 1000.0_f64.to_bits() {
                    let value = self.graph.number(value);
                    priorities.push((view, value));
                }
            }
        }
        let map = if priorities.is_empty() {
            Value::Nil
        } else {
            let map = self.graph.object("NSMapTable");
            self.graph.set(map, "$0", 517_i64);
            self.graph.set(map, "$1", 0_i64);
            for (index, (view, priority)) in priorities.iter().enumerate() {
                self.graph
                    .set(map, &format!("{}{}", '$', 2 + index * 2), *view);
                self.graph
                    .set(map, &format!("{}{}", '$', 3 + index * 2), *priority);
            }
            self.graph.set(
                map,
                &format!("{}{}", '$', 2 + priorities.len() * 2),
                Value::Nil,
            );
            self.graph
                .set(map, "NS.count", i64::try_from(priorities.len()).unwrap());
            Value::Object(map)
        };
        self.graph
            .set(container, "NSStackViewContainerVisibilityPriorities", map);
        let views = self.graph.array("NSMutableArray", views);
        self.graph
            .set(container, "NSStackViewContainerNonDroppedViews", views);
        Ok(container)
    }
}
