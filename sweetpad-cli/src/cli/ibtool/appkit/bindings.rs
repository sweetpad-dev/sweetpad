use super::{Element, Encoder, Id, Value};

impl Encoder<'_> {
    pub(super) fn binding(
        &mut self,
        element: &Element,
        connection: &Element,
    ) -> Result<Id, String> {
        let object = self.graph.object("NSNibBindingConnector");
        let source = self.object(element)?;
        let destination = self.lookup(
            connection
                .attr("destination")
                .ok_or("a binding has no destination")?,
        )?;
        let binding = connection.attr("name").ok_or("a binding has no name")?;
        let path = connection
            .attr("keyPath")
            .ok_or("a binding has no key path")?;
        self.graph.set(object, "NSSource", source);
        self.graph.set(object, "NSDestination", destination);
        self.string(object, "NSLabel", &format!("{binding}: {path}"));
        self.string(object, "NSBinding", binding);
        self.string(object, "NSKeyPath", path);
        if let Some(options) = connection.child("dictionary") {
            let mut entries = Vec::new();
            for option in &options.children {
                let key = self
                    .graph
                    .string(option.attr("key").ok_or("a binding option has no key")?);
                let value = match option.name.as_str() {
                    "bool" => self.graph.shared(
                        "NSNumber",
                        vec![("NS.boolval", Value::Bool(super::yes(option, "value")))],
                    ),
                    "integer" => self.graph.shared(
                        "NSNumber",
                        vec![(
                            "NS.intval",
                            Value::Int(
                                option
                                    .attr("value")
                                    .ok_or("an integer option has no value")?
                                    .parse()
                                    .map_err(|_| "invalid integer binding option")?,
                            ),
                        )],
                    ),
                    "string" => self.graph.string(option.text.as_str()),
                    other => return Err(format!("unsupported binding option <{other}>")),
                };
                entries.push((key, value));
            }
            let options = self.graph.dictionary("NSDictionary", &entries);
            self.graph.set(object, "NSOptions", options);
        }
        self.graph
            .set(object, "NSNibBindingConnectorVersion", 2_i64);
        Ok(object)
    }
}
