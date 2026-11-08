//! UIGestureRecognizer archives, captured from native Interface Builder.

use super::{Element, Encoder, Id, integer, no, number, yes};

pub(in crate::cli::ibtool) fn class(name: &str) -> Option<&'static str> {
    Some(match name {
        "tapGestureRecognizer" => "UITapGestureRecognizer",
        "swipeGestureRecognizer" => "UISwipeGestureRecognizer",
        "panGestureRecognizer" => "UIPanGestureRecognizer",
        // Interface Builder's XML spelling has always used "pong".
        "pongPressGestureRecognizer" => "UILongPressGestureRecognizer",
        "pinchGestureRecognizer" => "UIPinchGestureRecognizer",
        "rotationGestureRecognizer" => "UIRotationGestureRecognizer",
        _ => return None,
    })
}

impl Encoder<'_> {
    fn gesture_common(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for child in &element.children {
            if !matches!(child.name.as_str(), "connections" | "point") {
                return Err(format!("unsupported gesture child <{}>", child.name));
            }
        }
        for (attribute, key, default) in [
            ("enabled", "disabled", true),
            ("cancelsTouchesInView", "cancelsTouchesInView", true),
            ("delaysTouchesBegan", "delaysTouchesBegan", false),
            ("delaysTouchesEnded", "delaysTouchesEnded", true),
        ] {
            let value = element
                .attr(attribute)
                .map_or(default, |_| yes(element, attribute));
            if value != default {
                self.graph.set(
                    object,
                    &format!("UIGestureRecognizer.{key}"),
                    if attribute == "enabled" {
                        !value
                    } else {
                        value
                    },
                );
            }
        }
        self.graph
            .set(object, "UIGestureRecognizer.allowedTouchTypes", 15_i64);
        self.graph.set(
            object,
            "UIGestureRecognizer.requiresExclusiveTouchType",
            !no(element, "requiresExclusiveTouchType"),
        );
        Ok(())
    }

    pub(super) fn gesture(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.gesture_common(element, object)?;
        let class = class(&element.name).ok_or("unknown gesture recognizer")?;
        match class {
            "UITapGestureRecognizer" | "UILongPressGestureRecognizer" => {
                let implementation = self.graph.object("UITapRecognizer");
                let taps = integer(element, "numberOfTapsRequired")?
                    .unwrap_or(i64::from(class == "UITapGestureRecognizer"));
                let touches = integer(element, "numberOfTouchesRequired")?.unwrap_or(1);
                if taps != 1 {
                    self.graph
                        .set(implementation, "UITapRecognizer.numberOfTapsRequired", taps);
                }
                if touches != 1 {
                    self.graph.set(
                        implementation,
                        "UITapRecognizer.numberOfTouchesRequired",
                        touches,
                    );
                }
                self.graph
                    .set(object, &format!("{class}._imp"), implementation);
                if class == "UILongPressGestureRecognizer" {
                    if touches != 1 {
                        self.graph.set(
                            object,
                            "UILongPressGestureRecognizer.numberOfTouchesRequired",
                            touches,
                        );
                    }
                    for (attribute, default) in [
                        ("minimumPressDuration", 0.5_f64),
                        ("allowableMovement", 10.0_f64),
                    ] {
                        if let Some(value) =
                            number(element, attribute)?.filter(|n| n.to_bits() != default.to_bits())
                        {
                            self.graph.set(
                                object,
                                &format!("{class}.{attribute}"),
                                super::single(value),
                            );
                        }
                    }
                }
            }
            "UISwipeGestureRecognizer" => {
                let touches = integer(element, "numberOfTouchesRequired")?.unwrap_or(1);
                if touches != 1 {
                    self.graph.set(
                        object,
                        "UISwipeGestureRecognizer.numberOfTouchesRequired",
                        touches,
                    );
                }
                let direction = match element.attr("direction").unwrap_or("right") {
                    "right" => 1_i64,
                    "left" => 2,
                    "up" => 4,
                    "down" => 8,
                    other => return Err(format!("unsupported swipe direction '{other}'")),
                };
                if direction != 1 {
                    self.graph
                        .set(object, "UISwipeGestureRecognizer.direction", direction);
                }
            }
            "UIPanGestureRecognizer" => {
                for (attribute, default) in [
                    ("minimumNumberOfTouches", 1),
                    ("maximumNumberOfTouches", i64::MAX),
                ] {
                    if let Some(value) = integer(element, attribute)?.filter(|n| *n != default) {
                        self.graph
                            .set(object, &format!("{class}.{attribute}"), value);
                    }
                }
                self.graph.set(
                    object,
                    "UIPanGestureRecognizer.allowedScrollTypesMask",
                    0_i64,
                );
            }
            _ => {}
        }
        self.view_custom_class(element, object)?;
        Ok(())
    }
}
