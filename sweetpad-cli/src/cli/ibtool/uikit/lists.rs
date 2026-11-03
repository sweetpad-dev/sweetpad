//! Table and collection views, including their layout metadata.

use super::{Element, Encoder, Id, Options, Value, doubles, id_of, keyed, no, number};

impl Encoder<'_> {
    pub(super) fn table_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.scroll_view(element, object);
        let style = match element.attr("style") {
            None | Some("plain") => 0_i64,
            Some("grouped") => 1,
            Some("insetGrouped") => 2,
            Some(other) => return Err(format!("unsupported table style {other}")),
        };
        self.graph.set(object, "UITableViewStyle", style);
        self.graph.set(object, "UIStyle", i64::from(style != 0));
        if style == 0
            && keyed(element, "color", "backgroundColor")
                .is_some_and(|c| c.attr("systemColor") == Some("systemBackgroundColor"))
        {
            let color = self.system_color("tableBackgroundColor", super::Fallback::White(1.0, 1.0));
            self.graph.replace(object, "UIBackgroundColor", color);
        }
        self.graph.set(object, "UISeparatorStyle", 1_i64);
        self.graph
            .set(object, "UISeparatorStyleIOS5AndLater", 1_i64);
        self.graph.set(
            object,
            "UIPrefetchingEnabled",
            element.attr("prefetchingEnabled").map_or(
                self.document.attr("targetRuntime") != Some("AppleTV"),
                |value| value == "YES",
            ),
        );
        for (attribute, key, default) in [
            ("rowHeight", "UIRowHeight", 44.0),
            ("sectionHeaderHeight", "UISectionHeaderHeight", 22.0),
            ("sectionFooterHeight", "UISectionFooterHeight", 22.0),
            ("estimatedRowHeight", "UIEstimatedRowHeight", 0.0),
            (
                "estimatedSectionHeaderHeight",
                "UIEstimatedSectionHeaderHeight",
                0.0,
            ),
            (
                "estimatedSectionFooterHeight",
                "UIEstimatedSectionFooterHeight",
                0.0,
            ),
        ] {
            self.graph.set(
                object,
                key,
                super::single(number(element, attribute)?.unwrap_or(default)),
            );
        }
        self.graph.set(object, "UIFillerRowHeight", 0.0);
        self.graph.set(object, "UISectionHeaderTopPadding", -1.0);
        self.graph
            .set(object, "UIInsetsContentViewsToSafeArea", true);
        self.graph.set(object, "UISeparatorInsetReference", 0_i64);
        if no(element, "allowsSelection") {
            self.graph.set(object, "UIAllowsSelection", false);
        }
        if style != 0 && keyed(element, "color", "backgroundColor").is_none() {
            let color = self.system_color(
                "systemGroupedBackgroundColor",
                super::Fallback::Rgb(
                    0.949_019_607_843_137_2,
                    0.949_019_607_843_137_2,
                    0.968_627_450_980_392_2,
                    1.0,
                ),
            );
            self.graph.set(object, "UIBackgroundColor", color);
        }
        for (key, archive) in [
            ("tableHeaderView", "UITableHeaderView"),
            ("tableFooterView", "UITableFooterView"),
        ] {
            if let Some(view) = element.children.iter().find(|v| v.attr("key") == Some(key)) {
                let view = self.view(view)?;
                self.graph.set(object, archive, view);
            }
        }
        if element
            .child("sections")
            .is_some_and(|s| !s.children.is_empty())
        {
            self.static_table(element)?;
        }
        let prototypes: Vec<&Element> = element
            .children_named("prototypes")
            .flat_map(|p| p.children.iter())
            .collect();
        if !prototypes.is_empty() {
            self.prototypes(
                object,
                &prototypes,
                "UITableViewCellPrototypeNibExternalObjects",
                "UITableViewCellPrototypeNibs",
            )?;
        }
        Ok(())
    }

    pub(super) fn collection_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        self.scroll_view(element, object);
        self.graph.set(object, "UIDelegate", Value::Nil);
        let layout = keyed(element, "collectionViewFlowLayout", "collectionViewLayout")
            .ok_or("a collection view has no flow layout")?;
        let flow = self.graph.object("UICollectionViewFlowLayout");
        self.bind(id_of(layout)?, flow);
        self.objects.push(flow);
        self.graph.set(object, "UICollectionLayout", flow);
        self.flow_layout(layout, flow)?;
        self.graph
            .set(object, "UIAllowsUserInitiatedMultipleSelection", true);
        self.graph.set(
            object,
            "UICollectionViewPrefetchingEnabled",
            element.attr("prefetchingEnabled").map_or(
                self.document.attr("targetRuntime") != Some("AppleTV"),
                |value| value == "YES",
            ),
        );
        let cells: Vec<&Element> = element
            .children_named("cells")
            .flat_map(|p| p.children.iter())
            .collect();
        if !cells.is_empty() {
            self.prototypes(
                object,
                &cells,
                "UICollectionViewCellPrototypeNibExternalObjects",
                "UICollectionViewCellNibDict",
            )?;
        }
        let supplementary: Vec<_> = element.children_named("collectionReusableView").collect();
        if !supplementary.is_empty() {
            self.prototypes(
                object,
                &supplementary,
                "UICollectionViewSupplementaryViewPrototypeNibExternalObjects",
                "UICollectionViewSupplementaryViewNibDict",
            )?;
        }
        Ok(())
    }

    fn flow_layout(&mut self, element: &Element, object: Id) -> Result<(), String> {
        for (attribute, key, default) in [
            ("minimumInteritemSpacing", "UIInteritemSpacing", 10.0_f64),
            ("minimumLineSpacing", "UILineSpacing", 10.0_f64),
        ] {
            let value = number(element, attribute)?.unwrap_or(default);
            if value.to_bits() != default.to_bits() {
                self.graph.set(object, key, super::single(value));
            }
        }
        for (key, archive, default) in [
            ("itemSize", "UIItemSize", [50.0_f64; 2]),
            ("headerReferenceSize", "UIHeaderReferenceSize", [0.0_f64; 2]),
            ("footerReferenceSize", "UIFooterReferenceSize", [0.0_f64; 2]),
        ] {
            if let Some(size) = keyed(element, "size", key) {
                let size = [
                    number(size, "width")?.unwrap_or(0.0),
                    number(size, "height")?.unwrap_or(0.0),
                ];
                if size
                    .iter()
                    .zip(default)
                    .any(|(value, default)| value.to_bits() != default.to_bits())
                {
                    self.graph.set(object, archive, doubles(&size));
                }
            }
        }
        if let Some(inset) = keyed(element, "inset", "sectionInset") {
            let mut values = Vec::new();
            for key in ["minY", "minX", "maxY", "maxX"] {
                values.push(number(inset, key)?.unwrap_or(0.0));
            }
            if values.iter().any(|&v| v != 0.0) {
                self.graph.set(object, "UISectionInset", doubles(&values));
            }
        }
        if element.attr("automaticEstimatedItemSize") == Some("YES") {
            let value = self.graph.object("NSValue");
            self.graph.set(value, "NS.special", 2_i64);
            let size = self
                .graph
                .string("{1.7976931348623157e+308, 1.7976931348623157e+308}");
            self.graph.set(value, "NS.sizeval", size);
            let pair = self.graph.object("UINibKeyValuePair");
            self.graph.set(pair, "UIObject", object);
            self.graph.set(pair, "UIValue", value);
            let key = self.graph.string("estimatedItemSize");
            self.graph.set(pair, "UIKeyPath", key);
            self.key_value_pairs.push(pair);
        }
        if element.attr("sectionInsetReference") == Some("safeArea") {
            self.graph.set(object, "UISectionInsetReference", 1_i64);
        }
        if element.attr("scrollDirection") == Some("horizontal") {
            self.graph.set(object, "UIScrollDirection", 1_i64);
        }
        Ok(())
    }

    fn static_table(&mut self, element: &Element) -> Result<(), String> {
        let source = self.graph.object("UITableViewDataSource");
        self.static_sources.push(source);
        self.objects.push(source);
        let mut sections = Vec::new();
        for element in element
            .children_named("sections")
            .flat_map(|s| s.children_named("tableViewSection"))
        {
            let section = self.graph.object("UITableViewSection");
            self.objects.push(section);
            self.bind(id_of(element)?, section);
            for (attribute, key) in [
                ("headerTitle", "UITableSectionHeaderTitle"),
                ("footerTitle", "UITableSectionFooterTitle"),
            ] {
                if let Some(text) = element.attr(attribute) {
                    let text = self.text(element, attribute, text);
                    self.graph.set(section, key, text);
                }
            }
            let mut rows = Vec::new();
            for cell in element
                .children_named("cells")
                .flat_map(|c| c.children.iter())
            {
                let object = self.view(cell)?;
                let row = self.graph.object("UITableViewRow");
                self.graph.set(row, "UITableRowCell", object);
                self.graph.set(
                    row,
                    "UITableRowHeight",
                    super::single(number(cell, "rowHeight")?.unwrap_or(-1.0)),
                );
                rows.push(row);
            }
            let rows = self.graph.array("NSArray", &rows);
            self.graph.set(section, "UITableSectionRows", rows);
            sections.push(section);
        }
        let sections = self.graph.array("NSArray", &sections);
        self.graph.set(source, "UITableSections", sections);
        Ok(())
    }

    pub(super) fn collection_cell(&mut self, element: &Element, object: Id) -> Result<(), String> {
        if let Some(identifier) = element.attr("reuseIdentifier") {
            let identifier = self.graph.string(identifier);
            self.graph.set(object, "UIReuseIdentifier", identifier);
        }
        if element.name == "collectionViewCell" {
            self.graph.set(object, "UISystemBackgroundView", Value::Nil);
            self.graph
                .set(object, "UIContentConfigurationView", Value::Nil);
        }
        if let Some(content) = element
            .children
            .iter()
            .find(|e| e.attr("key") == Some("contentView"))
        {
            self.graph
                .set(object, "UIContentView", self.object_for(content)?);
        }
        Ok(())
    }

    fn prototypes(
        &mut self,
        object: Id,
        cells: &[&Element],
        external_key: &'static str,
        nib_key: &'static str,
    ) -> Result<(), String> {
        let mut external = Vec::new();
        let mut nibs = Vec::new();
        for source in cells {
            let prepared = super::prototype::prepare(source);
            let cell = &prepared;
            let identifier = cell
                .attr("reuseIdentifier")
                .ok_or("a prototype has no reuse identifier")?;
            let identifier = if cell.name == "collectionReusableView" {
                format!(
                    "{}/{}",
                    if cell.attr("key") == Some("sectionFooterView") {
                        "UICollectionElementKindSectionFooter"
                    } else {
                        "UICollectionElementKindSectionHeader"
                    },
                    identifier
                )
            } else {
                identifier.to_string()
            };
            let key = self.graph.string(&identifier);
            let options = Options {
                module: &self.module,
                localized: self.localized,
            };
            let mut encoder = Encoder::new(self.document, &options);
            let owner = super::super::storyboard::proxy(&mut encoder.graph, "IBFilesOwner");
            let responder = super::super::storyboard::proxy(&mut encoder.graph, "IBFirstResponder");
            let mut inside = std::collections::HashSet::new();
            collect(cell, &mut inside);
            let mut required = std::collections::BTreeSet::new();
            references(cell, &inside, &mut required);
            let mut top = vec![owner, responder];
            let mut registration = Vec::new();
            for (index, target) in required.iter().enumerate() {
                let name = format!("UpstreamPlaceholder-{}", index + 1);
                let proxy = super::super::storyboard::proxy(&mut encoder.graph, &name);
                encoder.bind(target, proxy);
                top.push(proxy);
                let name = self.graph.string(&name);
                registration.push((
                    name,
                    self.lookup(target)
                        .ok_or_else(|| format!("a prototype refers to missing {target}"))?,
                ));
            }
            let table = self
                .graph
                .fresh_dictionary("NSMutableDictionary", &registration);
            external.push((key, table));
            encoder.declare(cell)?;
            let view = encoder.view(cell)?;
            let mut objects = top.clone();
            top.push(view);
            objects.extend(encoder.objects.iter().copied());
            let connections = encoder.connections(cell)?;
            let root = super::super::storyboard::nib_root(
                &mut encoder.graph,
                &top,
                &objects,
                &connections,
                &encoder.key_value_pairs,
            );
            let bytes = encoder.graph.write(root);
            let data = self.graph.object("NSData");
            self.graph.set(data, "NS.bytes", Value::Data(bytes));
            let nib = self.graph.object("UINib");
            self.graph
                .set(nib, "captureEnclosingNIBBundleOnDecode", true);
            self.graph.set(nib, "archiveData", data);
            nibs.push((key, nib));
        }
        let external = self.graph.dictionary("NSMutableDictionary", &external);
        self.graph.set(object, external_key, external);
        let nibs = self.graph.dictionary("NSMutableDictionary", &nibs);
        self.graph.set(object, nib_key, nibs);
        Ok(())
    }
}

fn collect(element: &Element, inside: &mut std::collections::HashSet<String>) {
    if let Some(id) = element.attr("id") {
        inside.insert(id.to_string());
    }
    for child in &element.children {
        collect(child, inside);
    }
}
fn references(
    element: &Element,
    inside: &std::collections::HashSet<String>,
    required: &mut std::collections::BTreeSet<String>,
) {
    if let Some(list) = element.child("connections") {
        for connection in &list.children {
            let target = if connection.name == "segue" {
                connection.attr("id")
            } else {
                connection
                    .attr("destination")
                    .filter(|id| !inside.contains(*id))
            };
            if let Some(target) = target {
                required.insert(target.to_string());
            }
        }
    }
    for child in &element.children {
        references(child, inside, required);
    }
}
