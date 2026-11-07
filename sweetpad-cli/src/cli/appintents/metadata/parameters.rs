use super::{Value, argument, array, conforms, identifier, json, literal, localized, text};

pub(super) fn optional(name: &str) -> (&str, bool) {
    name.strip_prefix("Swift.Optional<")
        .and_then(|n| n.strip_suffix('>'))
        .map_or((name, false), |n| (n, true))
}

fn primitive(id: u64) -> Value {
    json!({"primitive":{"wrapper":{"typeIdentifier":id}}})
}
fn array_type(member: Value) -> Value {
    let mut value = json!({"array":{"wrapper":{"capabilities":3}}});
    value["array"]["wrapper"]["memberValueType"] = member;
    value
}

pub(super) fn value_type(name: &str, types: &[Value]) -> Result<Value, String> {
    let (name, _) = optional(name);
    if let Some(member) = name
        .strip_prefix("Swift.Array<")
        .and_then(|n| n.strip_suffix('>'))
    {
        return value_type(member, types).map(array_type);
    }
    let id = match name {
        "Swift.String" => Some(0),
        "Swift.Bool" => Some(1),
        "Swift.Int" | "Swift.Int8" | "Swift.Int16" | "Swift.Int32" | "Swift.Int64"
        | "Swift.UInt" | "Swift.UInt8" | "Swift.UInt16" | "Swift.UInt32" | "Swift.UInt64" => {
            Some(2)
        }
        "Swift.Double" | "Swift.Float" => Some(7),
        "Foundation.Date" | "FoundationEssentials.Date" => Some(8),
        "Foundation.URL" | "FoundationEssentials.URL" => Some(11),
        _ => None,
    };
    if let Some(id) = id {
        return Ok(primitive(id));
    }
    if name == "AppIntents.IntentFile" {
        return Ok(json!({"intents":{"wrapper":{"typeIdentifier":12}}}));
    }
    if let Some(value) = types.iter().find(|t| t["typeName"] == name) {
        if conforms(value, "AppEnum") {
            return Ok(json!({"linkEnumeration":{"wrapper":{"identifier":identifier(value)}}}));
        }
        if conforms(value, "AppEntity") {
            return Ok(json!({"entity":{"wrapper":{"typeName":identifier(value)}}}));
        }
    }
    Err(format!("unsupported App Intents value type '{name}'"))
}

pub(super) fn parameter(value: &Value, types: &[Value]) -> Result<Value, String> {
    let wrapper = array(&value["propertyWrappers"])
        .iter()
        .find(|w| text(&w["type"]).ends_with(".IntentParameter"))
        .ok_or("no IntentParameter wrapper")?;
    let arguments = array(&wrapper["arguments"]);
    if let Some(argument) = arguments.iter().find(|a| {
        !matches!(
            text(&a["label"]),
            "title"
                | "description"
                | "default"
                | "requestValueDialog"
                | "supportedContentTypes"
                | "inputConnectionBehavior"
        )
    }) {
        return Err(format!(
            "unsupported App Intents parameter option '{}'",
            text(&argument["label"])
        ));
    }
    let title = arguments
        .iter()
        .find(|a| a["label"] == "title")
        .or_else(|| argument(value, "title"))
        .ok_or("parameter has no constant title")?;
    let (name, is_optional) = optional(text(&value["type"]));
    let kind = value_type(name, types)?;
    let connection = arguments
        .iter()
        .find(|a| a["label"] == "inputConnectionBehavior");
    let connection = match connection.map(|v| text(&v["value"]["name"])) {
        None => 0,
        Some("connectToPreviousIntentResult") => 2,
        Some(name) => return Err(format!("unsupported inputConnectionBehavior '{name}'")),
    };
    if let Some(dialog) = arguments
        .iter()
        .find(|a| a["label"] == "requestValueDialog" && a["valueKind"] != "NilLiteral")
    {
        // The dialog is compiled into the intent's runtime implementation.
        // Native registration contains no dialog field for this option.
        literal(dialog)?;
    }
    let default = argument(value, "default").filter(|v| v["valueKind"] != "NilLiteral");
    let mut parameter = json!({
        "capabilities":u8::from(default.is_some()),"dynamicOptionsSupport":u8::from(kind.get("entity").is_some()),
        "inputConnectionBehavior":connection,"isInput":connection != 0,"isOptional":is_optional,
        "name":value["label"],"resolvableInputTypes":resolvers(&kind),
        "title":localized(&literal(title)?),"typeSpecificMetadata":[],"valueType":kind,
    });
    if let Some(default) = default {
        parameter["typeSpecificMetadata"] = json!([
            "LNValueTypeSpecificMetadataKeyDefaultValue",
            default_value(default, &kind)?
        ]);
    }
    if let Some(description) = arguments
        .iter()
        .find(|a| a["label"] == "description" && a["valueKind"] != "NilLiteral")
    {
        parameter["parameterDescription"] = localized(&literal(description)?);
    }
    if let Some(content_types) = arguments
        .iter()
        .find(|a| a["label"] == "supportedContentTypes")
    {
        let file_type = kind
            .get("intents")
            .or_else(|| kind["array"]["wrapper"]["memberValueType"].get("intents"));
        if file_type.is_none() || content_types["valueKind"] != "Array" {
            return Err("supportedContentTypes requires a constant array for IntentFile".into());
        }
        let mut identifiers = Vec::new();
        for value in array(&content_types["value"]) {
            if value["valueKind"] != "MemberReference"
                || value["value"]["baseType"] != "UniformTypeIdentifiers.UTType"
            {
                return Err("unsupported content type expression".into());
            }
            let id = match text(&value["value"]["memberLabel"]) {
                "image" => "public.image",
                "jpeg" => "public.jpeg",
                "png" => "public.png",
                "gif" => "com.compuserve.gif",
                "heic" => "public.heic",
                name => return Err(format!("unsupported content type '{name}'")),
            };
            identifiers.push(json!({"string":{"wrapper":id}}));
        }
        parameter["typeSpecificMetadata"]
            .as_array_mut()
            .unwrap()
            .extend([
                json!("LNValueTypeMetadataKeyFileSupportedTypes"),
                json!({"array":{"elements":identifiers}}),
            ]);
    }
    Ok(parameter)
}

fn resolvers(kind: &Value) -> Vec<Value> {
    let ids: &[u64] = match kind["primitive"]["wrapper"]["typeIdentifier"].as_u64() {
        Some(0) => &[2, 0],
        Some(1) => &[1],
        Some(2 | 7) => &[2, 7, 0],
        Some(8) => &[8],
        Some(11) => &[0, 11],
        _ => &[],
    };
    let mut types: Vec<Value> = ids
        .iter()
        .map(|id| json!({"kindValue":0,"valueType":primitive(*id)}))
        .collect();
    if kind["primitive"]["wrapper"]["typeIdentifier"] == 0 {
        types.push(json!({"kindValue":0,"valueType":array_type(primitive(2))}));
    }
    types
}

fn default_value(value: &Value, kind: &Value) -> Result<Value, String> {
    if kind.get("linkEnumeration").is_some() && value["valueKind"] == "Enum" {
        return Ok(json!({"string":{"wrapper":value["value"]["name"]}}));
    }
    let value = literal(value)?;
    Ok(
        match kind["primitive"]["wrapper"]["typeIdentifier"].as_u64() {
            Some(0) => json!({"string":{"wrapper":value}}),
            Some(1) => {
                json!({"int":{"wrapper":u8::from(value.parse::<bool>().map_err(|e|e.to_string())?)}})
            }
            Some(2) => json!({"int":{"wrapper":value.parse::<i64>().map_err(|e|e.to_string())?}}),
            Some(7) => {
                json!({"double":{"wrapper":value.parse::<f64>().map_err(|e|e.to_string())?}})
            }
            _ => return Err("unsupported App Intents parameter default".into()),
        },
    )
}
