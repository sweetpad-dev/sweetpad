use super::{
    Value, argument, array, availability, conforms, identifier, json, literal, localized,
    parameters, properties, property, text, visibility,
};

fn alias<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    array(&value["associatedTypeAliases"])
        .iter()
        .find(|a| a["typeAliasName"] == name)
        .map(|a| text(&a["substitutedTypeName"]))
}

fn display(value: &Value) -> Result<Value, String> {
    if value["valueKind"] == "RawLiteral" {
        return Ok(json!({"title":localized(&literal(value)?)}));
    }
    let title = literal(argument(value, "title").ok_or("display representation has no title")?)?;
    let mut result = json!({"title":localized(&title)});
    if let Some(subtitle) = argument(value, "subtitle").filter(|a| a["valueKind"] != "NilLiteral") {
        result["subtitle"] = localized(&literal(subtitle)?);
    }
    if let Some(image) = argument(value, "image").filter(|a| a["valueKind"] != "NilLiteral") {
        if let Some(name) = argument(image, "systemName") {
            result["image"] = json!({"systemImageName":{"_0":literal(name)?}});
            result["imageV2"] = result["image"].clone();
        } else {
            return Err(
                "only system images are supported in App Intents display representations".into(),
            );
        }
    }
    Ok(result)
}

pub(super) fn enumeration(value: &Value) -> Result<Value, String> {
    let cases = property(value, "caseDisplayRepresentations")
        .ok_or("AppEnum has no constant caseDisplayRepresentations")?;
    if cases["valueKind"] != "Dictionary" {
        return Err("AppEnum caseDisplayRepresentations must be a constant dictionary".into());
    }
    let mut entries = Vec::new();
    for entry in array(&cases["value"]) {
        let case = text(&entry["key"]["value"]["name"]);
        if case.is_empty() {
            return Err("AppEnum case has no identifier".into());
        }
        entries.push(json!({"displayRepresentation":display(&entry["value"])? ,"identifier":case}));
    }
    let declared = array(&value["cases"]);
    entries.sort_by_key(|entry| {
        declared
            .iter()
            .position(|case| case["name"] == entry["identifier"])
            .unwrap_or(usize::MAX)
    });
    let title = literal(
        property(value, "typeDisplayRepresentation")
            .ok_or("AppEnum has no typeDisplayRepresentation")?,
    )?;
    Ok(json!({
        "assistantDefinedSchemas":[],"availabilityAnnotations":availability(),"cases":entries,
        "displayTypeName":localized(&title),"effectiveBundleIdentifiers":[],
        "fullyQualifiedTypeName":value["typeName"],"identifier":identifier(value),"isSystem":false,
        "mangledTypeName":value["mangledTypeName"],"mangledTypeNameByBundleIdentifier":{},
        "systemProtocolMetadata":[],"visibilityMetadata":visibility(),
    }))
}

pub(super) fn entity(value: &Value) -> Result<Value, String> {
    let title = literal(
        property(value, "typeDisplayRepresentation")
            .ok_or("AppEntity has no typeDisplayRepresentation")?,
    )?;
    let mut result = json!({
        "assistantDefinedSchemas":[],"availabilityAnnotations":availability(),"displayTypeName":localized(&title),
        "effectiveBundleIdentifiers":[],"entitySyncMetadata":{"entitySyncType":0},
        "fullyQualifiedTypeName":value["typeName"],"mangledTypeName":value["mangledTypeName"],
        "mangledTypeNameByBundleIdentifier":{},"properties":[],"requiredCapabilities":[],
        "systemProtocolMetadata":[],"systemProtocolMetadataV2":[],"transient":false,
        "typeName":identifier(value),"typeSpecificMetadata":[],"visibilityMetadata":visibility(),
    });
    if let Some(query) = alias(value, "DefaultQuery") {
        result["defaultQueryIdentifier"] = json!(query);
    }
    result["properties"] = Value::Array(
        properties(value)
            .iter()
            .filter(|p| {
                array(&p["propertyWrappers"])
                    .iter()
                    .any(|w| text(&w["type"]).ends_with(".EntityProperty"))
            })
            .cloned()
            .collect(),
    );
    Ok(result)
}

pub(super) fn entity_properties(value: &Value, types: &[Value]) -> Result<Vec<Value>, String> {
    let mut result = Vec::new();
    for property in array(value) {
        let title = literal(argument(property, "title").ok_or("entity property has no title")?)?;
        let (name, optional) = parameters::optional(text(&property["type"]));
        result.push(json!({"capabilities":0,"identifier":property["label"],"isOptional":optional,"title":localized(&title),"valueType":parameters::value_type(name,types)?}));
    }
    Ok(result)
}

pub(super) fn query(value: &Value, types: &[Value]) -> Result<Value, String> {
    if conforms(value, "EntityStringQuery") || conforms(value, "EntityPropertyQuery") {
        return Err("App Intents string/property queries need additional metadata support".into());
    }
    let entity = alias(value, "Entity").ok_or("EntityQuery has no Entity associated type")?;
    let entity_name = entity.split_once('.').map_or(entity, |(_, n)| n);
    let default = types.iter().any(|t| {
        conforms(t, "AppEntity") && alias(t, "DefaultQuery") == Some(text(&value["typeName"]))
    });
    Ok(json!({
        "availabilityAnnotations":availability(),"capabilities":66,"defaultQueryForEntity":default,
        "effectiveBundleIdentifiers":[],"entityType":entity_name,"fullyQualifiedIdentifier":value["typeName"],
        "identifier":identifier(value),"mangledTypeName":value["mangledTypeName"],"mangledTypeNameByBundleIdentifier":{},
        "parameters":[],"queryType":identifier(value),"resultValueType":{"entity":{"wrapper":{"typeName":entity_name}}},
        "sortingOptions":[],"visibilityMetadata":visibility(),
    }))
}
