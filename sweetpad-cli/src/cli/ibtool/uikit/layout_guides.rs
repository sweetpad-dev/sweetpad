//! Runtime guides used by legacy controllers and modern scroll views.
use super::{Element, Encoder, Id, Value};

impl Encoder<'_> {
    pub fn register_controller_guides(&mut self, controller: &Element, view: &Element) {
        let guides: Vec<_> = controller
            .children_named("layoutGuides")
            .flat_map(|l| l.children_named("viewControllerLayoutGuide"))
            .filter_map(|g| Some((g.attr("id")?.to_string(), g.attr("type")? == "bottom")))
            .collect();
        if guides.is_empty() {
            return;
        }
        self.legacy_guide_definitions
            .insert(view.attr("id").unwrap_or_default().to_string(), guides);
    }

    pub(super) fn install_legacy_guides(&mut self, owner_id: &str, owner: Id) {
        let Some(definitions) = self.legacy_guide_definitions.remove(owner_id) else {
            return;
        };
        let (mut guides, mut constraints) = (Vec::new(), Vec::new());
        for (name, bottom) in definitions {
            let guide = self.graph.object("_UILayoutGuide");
            self.bind(&name, guide);
            self.objects.push(guide);
            for key in [
                "UIAutoresizeSubviews",
                "UIDeepDrawRect",
                "UIHidden",
                "UIOpaque",
                "UIViewDoesNotTranslateAutoresizingMaskIntoConstraints",
            ] {
                self.graph.set(guide, key, true);
            }
            self.graph
                .set(guide, "UIViewSemanticContentAttribute", 0_i64);
            self.large_content_metadata(guide);
            let identifier = self.graph.string(if bottom {
                "_UIViewControllerBottom"
            } else {
                "_UIViewControllerTop"
            });
            self.graph
                .set(guide, "_UILayoutGuideIdentifier", identifier);
            let support =
                self.layout_constraint("_UILayoutSupportConstraint", guide, 7, None, true);
            self.graph.set(support, "NSPriority", 999_i64);
            let support = self.graph.array("NSMutableArray", &[support]);
            self.graph
                .set(guide, "UIViewAutolayoutConstraints", support);
            let leading =
                self.layout_constraint("NSLayoutConstraint", guide, 5, Some((owner, 5)), true);
            let height = self.layout_constraint("NSLayoutConstraint", guide, 8, None, true);
            let edge = if bottom {
                self.layout_constraint("NSLayoutConstraint", owner, 4, Some((guide, 4)), true)
            } else {
                self.layout_constraint("NSLayoutConstraint", guide, 3, Some((owner, 3)), true)
            };
            let removed = if bottom {
                vec![leading, height, edge]
            } else {
                vec![leading, edge, height]
            };
            constraints.extend(removed.iter().copied());
            let removed = self.graph.array("NSArray", &removed);
            self.graph
                .set(guide, "_UILayoutGuideConstraintsToRemove", removed);
            guides.push(guide);
        }
        self.legacy_guides
            .insert(owner_id.to_string(), (guides, constraints));
    }

    pub(super) fn scroll_layout_guide(&mut self, name: &str, owner: &str, content: bool) -> Id {
        let view = self.ids[owner];
        let guide = self.graph.object(if content {
            "_UIScrollViewLayoutGuide"
        } else {
            "UILayoutGuide"
        });
        self.bind(name, guide);
        let prefix = if content {
            "UIScrollView-contentLayoutGuide"
        } else {
            "UIScrollView-frameLayoutGuide"
        };
        let identifier = self.graph.string(prefix);
        self.graph.set(guide, "UILayoutGuideIdentifier", identifier);
        self.graph.set(guide, "UILayoutGuideOwningView", view);
        self.graph
            .set(guide, "UILayoutGuideOwningViewIsLocked", true);
        self.graph
            .set(guide, "UILayoutGuideShouldBeArchived", false);
        self.graph
            .set(guide, "UILayoutGuideAllowsNegativeDimensions", false);
        let mut constraints = Vec::new();
        for (suffix, attribute) in [("minX", 1), ("minY", 3), ("width", 7), ("height", 8)] {
            if content && attribute >= 7 {
                continue;
            }
            let constraint = self.layout_constraint(
                "NSLayoutConstraint",
                guide,
                attribute,
                Some((view, attribute)),
                false,
            );
            let identifier = self.graph.string(&format!("{prefix}-{suffix}"));
            self.graph.set(constraint, "NSLayoutIdentifier", identifier);
            constraints.push(constraint);
        }
        let constraints = self.graph.array("NSArray", &constraints);
        self.graph
            .set(guide, "UILayoutGuideSystemConstraints", constraints);
        guide
    }

    fn layout_constraint(
        &mut self,
        class: &'static str,
        first: Id,
        attribute: i64,
        second: Option<(Id, i64)>,
        archived: bool,
    ) -> Id {
        let object = self.graph.object(class);
        self.graph.set(object, "NSFirstItem", first);
        self.graph.set(object, "NSFirstAttribute", attribute);
        self.graph.set(object, "NSFirstAttributeV2", attribute);
        if let Some((second, attribute)) = second {
            self.graph.set(object, "NSSecondItem", second);
            self.graph.set(object, "NSSecondAttribute", attribute);
            self.graph.set(object, "NSSecondAttributeV2", attribute);
        }
        self.graph
            .set(object, "NSShouldBeArchived", Value::Bool(archived));
        object
    }
}
