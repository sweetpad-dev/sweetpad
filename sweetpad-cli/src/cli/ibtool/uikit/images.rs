//! Image placeholders resolve assets and SF Symbols from the nib's bundle.

use super::{Element, Encoder, Id, Value, number, single};

impl Encoder<'_> {
    pub(super) fn image_view(&mut self, element: &Element, object: Id) -> Result<(), String> {
        let reference = element.child("imageReference");
        let source = reference.unwrap_or(element);
        if let Some(name) = source.attr("image") {
            let image = self.image(name, source.attr("catalog"), reference)?;
            self.graph.set(object, "UIImage", image);
        }
        if let Some(configuration) = element.child("preferredSymbolConfiguration") {
            let configuration = self.symbol_configuration(configuration)?;
            self.graph
                .set(object, "UIPreferredSymbolConfiguration", configuration);
        }
        Ok(())
    }

    pub(super) fn image(
        &mut self,
        name: &str,
        catalog: Option<&str>,
        configuration: Option<&Element>,
    ) -> Result<Id, String> {
        let system = catalog == Some("system");
        let width = if system { 0.0 } else { 1.0 };
        let height = width;
        let image = self.graph.object("UIImageNibPlaceholder");
        self.graph.set(image, "UIImageWidth", single(width));
        self.graph.set(image, "UIImageHeight", single(height));
        let name = self.graph.string(name);
        self.graph.set(image, "UIResourceName", name);
        self.graph.set(
            image,
            "UISystemSymbolResourceName",
            if system { Some(name) } else { None },
        );
        let configuration = configuration
            .map(|c| self.symbol_configuration(c))
            .transpose()?;
        self.graph
            .set(image, "UISymbolImageConfiguration", configuration);
        self.graph.set(image, "UIResourceCatalogName", Value::Nil);
        Ok(self.graph.intern(image))
    }

    pub(super) fn symbol_configuration(&mut self, element: &Element) -> Result<Id, String> {
        let object = self.graph.object("UIImageSymbolConfiguration");
        if let Some(scale) = element
            .attr("scale")
            .or_else(|| element.attr("symbolScale"))
        {
            let value = match scale {
                "small" => 1_i64,
                "medium" => 2,
                "large" => 3,
                other => return Err(format!("unknown symbol scale {other}")),
            };
            self.graph.set(object, "UISymbolScale", value);
        }
        if let Some(weight) = element.attr("weight") {
            let value = match weight {
                "ultraLight" => 1_i64,
                "thin" => 2,
                "light" => 3,
                "regular" => 4,
                "medium" => 5,
                "semibold" => 6,
                "bold" => 7,
                "heavy" => 8,
                "black" => 9,
                other => return Err(format!("unknown symbol weight {other}")),
            };
            self.graph.set(object, "UISymbolWeight", value);
        }
        if let Some(size) = number(element, "pointSize")? {
            self.graph.set(object, "UISymbolPointSize", size);
        }
        Ok(self.graph.intern(object))
    }
}
