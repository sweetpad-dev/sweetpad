use super::{
    RegistrationFormat, Value, argument, array, conforms, identifier, json, literal, localized,
    text,
};

pub(super) fn optional(name: &str) -> (&str, bool) {
    name.strip_prefix("Swift.Optional<")
        .and_then(|n| n.strip_suffix('>'))
        .map_or((name, false), |n| (n, true))
}

fn primitive(id: u64) -> Value {
    json!({"primitive":{"wrapper":{"typeIdentifier":id}}})
}
fn array_type(member: Value, capabilities: u8) -> Value {
    let mut value = json!({"array":{"wrapper":{"capabilities":capabilities}}});
    value["array"]["wrapper"]["memberValueType"] = member;
    value
}

pub(super) fn value_type(name: &str, types: &[Value]) -> Result<Value, String> {
    let (name, _) = optional(name);
    // Both use the array representation, with distinct native capability masks.
    for (prefix, capabilities) in [("Swift.Array<", 3), ("Swift.Set<", 0)] {
        if let Some(member) = name.strip_prefix(prefix).and_then(|n| n.strip_suffix('>')) {
            return Ok(array_type(value_type(member, types)?, capabilities));
        }
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

pub(super) fn parameter(
    value: &Value,
    types: &[Value],
    format: RegistrationFormat,
) -> Result<Value, String> {
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
                | "inputOptions"
        )
    }) {
        return Err(format!(
            "unsupported App Intents parameter option '{}'",
            text(&argument["label"])
        ));
    }
    let (arguments, title, default) =
        if format == RegistrationFormat::Xcode26 && value["label"] == "parameterSummary" {
            // Xcode 26 infers this instance parameter from its name and type. Its
            // wrapper options are lost when the name matches the AppIntent requirement.
            (&[][..], "Parameter Summary".to_owned(), None)
        } else {
            let title = arguments
                .iter()
                .find(|a| a["label"] == "title")
                .or_else(|| argument(value, "title"))
                .ok_or("parameter has no constant title")?;
            let default = argument(value, "default").filter(|v| v["valueKind"] != "NilLiteral");
            (arguments, literal(title)?, default)
        };
    let (name, is_optional) = optional(text(&value["type"]));
    let kind = value_type(name, types)?;
    let connection = input_connection_behavior(arguments)?;
    if let Some(dialog) = arguments
        .iter()
        .find(|a| a["label"] == "requestValueDialog" && a["valueKind"] != "NilLiteral")
    {
        // The dialog is compiled into the intent's runtime implementation.
        // Native registration contains no dialog field for this option.
        literal(dialog)?;
    }
    let mut parameter = json!({
        "capabilities":u8::from(default.is_some()),"dynamicOptionsSupport":u8::from(kind.get("entity").is_some()),
        "inputConnectionBehavior":connection,"isInput":connection == 2,"isOptional":is_optional,
        "name":value["label"],"resolvableInputTypes":resolvers(&kind),
        "title":localized(&title),"typeSpecificMetadata":[],"valueType":kind,
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
    input_options(arguments, &mut parameter)?;
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

fn input_connection_behavior(arguments: &[Value]) -> Result<u8, String> {
    let behavior = arguments
        .iter()
        .find(|a| a["label"] == "inputConnectionBehavior");
    match behavior.map(|v| text(&v["value"]["name"])) {
        None | Some("default") => Ok(0),
        Some("never") => Ok(1),
        Some("connectToPreviousIntentResult") => Ok(2),
        Some(name) => Err(format!("unsupported inputConnectionBehavior '{name}'")),
    }
}

/// `String.IntentInputOptions`, in the order of each option's metadata value.
const KEYBOARD_TYPES: [&str; 5] = [
    "asciiCapable",
    "numbersAndPunctuation",
    "URL",
    "numberPad",
    "default",
];
const CAPITALIZATION_TYPES: [&str; 4] = ["none", "words", "sentences", "allCharacters"];

/// Xcode writes one key per `String.IntentInputOptions` argument the source passes.
/// Arguments left at their defaults are captured as `Runtime` and omitted.
fn input_options(arguments: &[Value], parameter: &mut Value) -> Result<(), String> {
    let Some(options) = arguments
        .iter()
        .find(|a| a["label"] == "inputOptions" && a["valueKind"] != "NilLiteral")
    else {
        return Ok(());
    };
    let kind = &parameter["valueType"];
    let member = kind
        .pointer("/array/wrapper/memberValueType")
        .unwrap_or(kind);
    if member["primitive"]["wrapper"]["typeIdentifier"] != 0 {
        return Err("inputOptions requires a String parameter".into());
    }
    if options["valueKind"] != "InitCall"
        || options["value"]["type"] != "Swift.String.IntentInputOptions"
    {
        return Err("inputOptions must be a constant String.IntentInputOptions initializer".into());
    }
    let mut metadata = Vec::new();
    for option in array(&options["value"]["arguments"]) {
        let label = text(&option["label"]);
        let (key, cases): (&str, &[&str]) = match label {
            "keyboardType" => ("KeyboardType", &KEYBOARD_TYPES),
            "capitalizationType" => ("CapitalizationType", &CAPITALIZATION_TYPES),
            "multiline" => ("Multiline", &["false", "true"]),
            "autocorrect" => ("Autocorrect", &["false", "true"]),
            "smartQuotes" => ("SmartQuotes", &["false", "true"]),
            "smartDashes" => ("SmartDashes", &["false", "true"]),
            _ => return Err(format!("unsupported inputOptions argument '{label}'")),
        };
        if option["valueKind"] == "Runtime" {
            continue;
        }
        let (expected, name) = if label.ends_with("Type") {
            ("Enum", text(&option["value"]["name"]))
        } else {
            ("RawLiteral", text(&option["value"]))
        };
        let value = cases
            .iter()
            .position(|c| *c == name)
            .filter(|_| option["valueKind"] == expected)
            .ok_or_else(|| format!("unsupported inputOptions {label} '{name}'"))?;
        metadata.extend([
            json!(format!("LNValueTypeMetadataKeyString{key}")),
            json!({"int":{"wrapper":value}}),
        ]);
    }
    parameter["typeSpecificMetadata"]
        .as_array_mut()
        .unwrap()
        .extend(metadata);
    Ok(())
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
        types.push(json!({"kindValue":0,"valueType":array_type(primitive(2), 3)}));
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
