//! Outline views add hierarchy and indentation state to table-view archives.
use super::{Element, Encoder, Id, no, number, yes};

impl Encoder<'_> {
    #[allow(clippy::cast_possible_truncation)]
    pub(super) fn outline_view(
        &mut self,
        element: &Element,
        object: Id,
        enclosed: bool,
    ) -> Result<(), String> {
        self.table_view(element, object, enclosed)?;
        if !yes(element, "autoresizesOutlineColumn") {
            self.graph
                .set(object, "NSOutlineViewAutoresizesOutlineColumnKey", false);
        }
        let indentation = number(element, "indentationPerLevel", 0.0)? as f32;
        if indentation.to_bits() != 13.0_f32.to_bits() {
            self.graph
                .set(object, "NSOutlineViewIndentationPerLevelKey", indentation);
        }
        if no(element, "indentationMarkerFollowsCell") {
            self.graph.set(
                object,
                "NSOutlineViewIndentationMarkerFollowsCellKey",
                false,
            );
        }
        if yes(element, "autosaveExpandedItems") {
            self.graph
                .set(object, "NSOutlineViewAutosaveExpandedItemsKey", true);
        }
        self.graph.set(
            object,
            "NSOutineViewStronglyReferencesItems",
            !no(element, "stronglyReferencesItems"),
        );
        if let Some(column) = element.attr("outlineTableColumn") {
            let first = element
                .child("tableColumns")
                .and_then(|c| c.children.first())
                .and_then(|c| c.attr("id"));
            if first != Some(column) {
                let column = self.lookup(column)?;
                self.graph
                    .set(object, "NSOutlineViewOutlineTableColumnKey", column);
            }
        }
        Ok(())
    }
}
