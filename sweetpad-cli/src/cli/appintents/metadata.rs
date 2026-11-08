//! The registration schema, checked against Xcode's constant-value captures.

use serde_json::{Value, json};

mod parameters;
mod representations;
use representations::{entity, enumeration, query};

fn array(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}
fn properties(value: &Value) -> &[Value] {
    array(&value["properties"])
}
fn property<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    properties(value).iter().find(|p| p["label"] == name)
}
fn conforms(value: &Value, name: &str) -> bool {
    array(&value["conformances"])
        .iter()
        .any(|c| text(c).rsplit('.').next() == Some(name))
        || array(&value["allConformances"])
            .iter()
            .any(|c| text(&c["protocolName"]).rsplit('.').next() == Some(name))
}
fn identifier(value: &Value) -> &str {
    text(&value["typeName"])
        .split_once('.')
        .map_or(text(&value["typeName"]), |(_, n)| n)
}
fn availability() -> Value {
    json!({"LNPlatformNameWildcard":{"introducedVersion":"*"}})
}
fn visibility() -> Value {
    json!({"assistantOnly":false,"isDiscoverable":true})
}
fn localized(key: &str) -> Value {
    json!({"alternatives":[],"key":key})
}

/// A constant string, including a resource or display representation initializer.
fn literal(value: &Value) -> Result<String, String> {
    if value["valueKind"] == "RawLiteral" {
        return Ok(text(&value["value"]).to_string());
    }
    if value["valueKind"] == "InitCall" {
        let arguments = array(&value["value"]["arguments"]);
        let argument = arguments
            .iter()
            .find(|a| {
                matches!(
                    text(&a["label"]),
                    "title" | "name" | "key" | "stringLiteral" | "description"
                )
            })
            .or_else(|| arguments.first())
            .ok_or("a localized initializer has no literal")?;
        return literal(argument);
    }
    Err(format!(
        "expected a constant localized string, got {}",
        text(&value["valueKind"])
    ))
}

fn boolean(value: &Value, name: &str, default: bool) -> Result<bool, String> {
    let Some(property) = property(value, name) else {
        return Ok(default);
    };
    match text(&property["value"]) {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(format!("{name} must be a constant boolean")),
    }
}

fn argument<'a>(value: &'a Value, label: &str) -> Option<&'a Value> {
    array(&value["value"]["arguments"])
        .iter()
        .find(|a| a["label"] == label)
}

pub(super) fn extract(types: &[Value]) -> Result<Value, String> {
    if types
        .iter()
        .filter(|t| conforms(t, "AppShortcutsProvider"))
        .count()
        > 1
    {
        return Err("an app must have a single AppShortcutsProvider".into());
    }
    if types
        .iter()
        .any(|t| conforms(t, "_AssistantIntentsProvider"))
    {
        return Err("assistant schema metadata is not supported yet".into());
    }
    if types.iter().any(|t| conforms(t, "AppIntentsPackage")) {
        return Err("AppIntentsPackage registration is not supported yet".into());
    }
    let mut result = json!({"actions":{},"assistantEntities":[],"assistantIntentNegativePhrases":[],"assistantIntents":[],"autoShortcuts":[],"entities":{},"enums":[],"generator":{"name":"sweetpad","version":env!("CARGO_PKG_VERSION")},"negativePhrases":[],"queries":{},"shortcutTileColor":14,"version":1});
    for value in types {
        // Swift emits Never's imported conformances when a source extends it.
        // This uninhabited type is a protocol placeholder, not an app action.
        if value["typeName"] == "Swift.Never" {
            continue;
        }
        let id = identifier(value);
        if conforms(value, "AppEnum") {
            result["enums"]
                .as_array_mut()
                .unwrap()
                .push(enumeration(value)?);
        }
        if conforms(value, "AppEntity") {
            let mut entity = entity(value)?;
            entity["properties"] = json!(representations::entity_properties(
                &entity["properties"],
                types
            )?);
            result["entities"][id] = entity;
        }
        if conforms(value, "EntityQuery") {
            result["queries"][id] = query(value, types)?;
        }
        if conforms(value, "AppIntent") || conforms(value, "WidgetConfigurationIntent") {
            result["actions"][id] = action(value, types)?;
        }
        if conforms(value, "AppShortcutsProvider") {
            result["autoShortcutProviderMangledName"] = value["mangledTypeName"].clone();
            result["autoShortcuts"] = shortcuts(value)?;
        }
    }
    Ok(result)
}

pub(super) fn strings_data(types: &[Value], metadata: &Value) -> Option<Value> {
    let provider = types.iter().find(|t| conforms(t, "AppShortcutsProvider"))?;
    let property = property(provider, "appShortcuts")?;
    let mut entries = Vec::new();
    for (index, shortcut) in array(&metadata["autoShortcuts"]).iter().enumerate() {
        for phrase in array(&shortcut["phraseTemplates"]) {
            entries.push(json!({
                "appShortcutMetadata":[{"actionIdentifier":shortcut["actionIdentifier"],"index":index,"parameterDisplayRepresentations":{},"parameterTypes":{},"title":shortcut["shortTitle"]["key"]}],
                "key":phrase["key"],"location":{"startingColumn":0,"startingLine":property["line"]},"values":[phrase["key"]],
            }));
        }
    }
    Some(json!({"source":property["file"],"tables":{"AppShortcuts":entries},"version":2}))
}

fn action(value: &Value, types: &[Value]) -> Result<Value, String> {
    let id = identifier(value);
    let widget = conforms(value, "WidgetConfigurationIntent");
    let open = boolean(value, "openAppWhenRun", false)?;
    let discoverable = boolean(value, "isDiscoverable", true)?;
    let authentication = if let Some(policy) = property(value, "authenticationPolicy") {
        match text(&policy["value"]["name"]) {
            "alwaysAllowed" => 0,
            "requiresAuthentication" => 1,
            "requiresLocalDeviceAuthentication" => 2,
            name => {
                return Err(format!(
                    "unsupported authenticationPolicy '{name}' in intent '{id}'"
                ));
            }
        }
    } else {
        0
    };
    if property(value, "supportedModes").is_some() {
        return Err(format!(
            "explicit supportedModes metadata is not supported yet in intent '{id}'"
        ));
    }
    let title = literal(
        property(value, "title").ok_or_else(|| format!("intent '{id}' has no constant title"))?,
    )?;
    let mut action = json!({
        "assistantDefinedSchemaTraits":[],"assistantDefinedSchemas":[],"authenticationPolicy":authentication,
        "availabilityAnnotations":availability(),"effectiveBundleIdentifiers":[],
        "fullyQualifiedTypeName":value["typeName"],"identifier":id,"isAuthPolExplicit":property(value,"authenticationPolicy").is_some(),
        "isDiscoverable":discoverable&&!widget,"mangledTypeName":value["mangledTypeName"],
        "mangledTypeNameByBundleIdentifier":{},"mangledTypeNameByBundleIdentifierV2":{},
        "mangledTypeNameV2":value["mangledTypeName"],"openAppWhenRun":open,
        "outputFlags":if widget {8}else{0},"parameters":[],"presentationStyle":0,
        "requiredCapabilities":[],"supportedModes":if open {2}else{1},
        "systemProtocolMetadata":[],"systemProtocolMetadataV2":[],"systemProtocols":[],
        "title":localized(&title),"typeSpecificMetadata":[],"visibilityMetadata":visibility(),
    });
    if !discoverable {
        action["visibilityMetadata"]["isDiscoverable"] = json!(false);
    }
    if widget {
        let protocol = "com.apple.link.systemProtocol.WidgetConfiguration";
        action["systemProtocolMetadata"] = json!([protocol,{"empty":{}}]);
        action["systemProtocolMetadataV2"] = action["systemProtocolMetadata"].clone();
        action["systemProtocols"] = json!([protocol]);
    }
    if let Some(description) = property(value, "description") {
        action["descriptionMetadata"] =
            json!({"descriptionText":localized(&literal(description)?),"searchKeywords":[]});
    }
    for parameter in properties(value).iter().filter(|p| {
        array(&p["propertyWrappers"])
            .iter()
            .any(|w| text(&w["type"]).ends_with(".IntentParameter"))
    }) {
        action["parameters"]
            .as_array_mut()
            .unwrap()
            .push(parameters::parameter(parameter, types)?);
    }
    for alias in array(&value["associatedTypeAliases"])
        .iter()
        .filter(|a| a["typeAliasName"] == "PerformResult")
    {
        let mut flags = action["outputFlags"].as_u64().unwrap();
        for requirement in array(&alias["opaqueTypeProtocolRequirements"]) {
            match text(requirement) {
                "AppIntents.ProvidesDialog" => flags |= 2,
                "AppIntents.ShowsSnippetView" => flags |= 4,
                _ => {}
            }
        }
        action["outputFlags"] = json!(flags);
        for requirement in array(&alias["opaqueTypeSameTypeRequirements"])
            .iter()
            .filter(|r| r["typeAliasName"] == "AppIntents.IntentResult.Value")
        {
            action["outputType"] =
                parameters::value_type(text(&requirement["substitutedTypeName"]), types)?;
        }
    }
    Ok(action)
}

fn shortcuts(provider: &Value) -> Result<Value, String> {
    let value =
        property(provider, "appShortcuts").ok_or("an AppShortcutsProvider has no appShortcuts")?;
    let mut items = Vec::new();
    shortcut_items(value, &mut items)?;
    Ok(Value::Array(items))
}

fn shortcut_items(value: &Value, items: &mut Vec<Value>) -> Result<(), String> {
    match text(&value["valueKind"]) {
        "Builder" => {
            for member in array(&value["value"]["members"]) {
                shortcut_items(&member["element"], items)?;
            }
        }
        "Array" => {
            for item in array(&value["value"]) {
                shortcut_items(item, items)?;
            }
        }
        "InitCall" if value["value"]["type"] == "AppIntents.AppShortcut" => {
            let intent = argument(value, "intent").ok_or("an AppShortcut has no intent")?;
            let intent = text(&intent["type"])
                .split_once('.')
                .map_or(text(&intent["type"]), |(_, n)| n);
            let phrases = argument(value, "phrases").ok_or("an AppShortcut has no phrases")?;
            let templates = array(&phrases["value"])
                .iter()
                .map(|p| phrase(p).map(|s| localized(&s)))
                .collect::<Result<Vec<_>, _>>()?;
            let title =
                literal(argument(value, "shortTitle").ok_or("an AppShortcut has no shortTitle")?)?;
            let image = literal(
                argument(value, "systemImageName")
                    .ok_or("an AppShortcut has no systemImageName")?,
            )?;
            items.push(json!({"actionIdentifier":intent,"availabilityAnnotations":availability(),"phraseTemplates":templates,"shortTitle":localized(&title),"systemImageName":image}));
        }
        kind => return Err(format!("unsupported AppShortcuts builder value '{kind}'")),
    }
    Ok(())
}

fn phrase(value: &Value) -> Result<String, String> {
    if value["valueKind"] == "RawLiteral" {
        return literal(value);
    }
    if value["valueKind"] != "InterpolatedStringLiteral" {
        return Err("AppShortcut phrases must be constant strings".into());
    }
    let mut phrase = String::new();
    for segment in array(&value["value"]["segments"]) {
        match text(&segment["valueKind"]) {
            "RawLiteral" => phrase.push_str(text(&segment["value"])),
            "Enum" if segment["value"]["name"] == "applicationName" => {
                phrase.push_str("${applicationName}");
            }
            _ => {
                return Err(
                    "AppShortcut phrase interpolation is only supported for applicationName".into(),
                );
            }
        }
    }
    Ok(phrase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn unknown_authentication_policies_never_fall_back_to_always_allowed() {
        let mut types: Vec<Value> = serde_json::from_str(include_str!(
            "../../../fixtures/appintents/Types.swiftconstvalues"
        ))
        .unwrap();
        let intent = types
            .iter_mut()
            .find(|t| t["typeName"] == "MetadataProbe.AuthenticatedIntent")
            .unwrap();
        let policy = intent["properties"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["label"] == "authenticationPolicy")
            .unwrap();
        policy["value"]["name"] = json!("futureAuthenticationPolicy");
        assert!(
            extract(&types)
                .unwrap_err()
                .contains("unsupported authenticationPolicy")
        );
    }
    #[test]
    fn shortcut_localization_metadata_matches_xcode() {
        let types: Vec<Value> = serde_json::from_str(include_str!(
            "../../../fixtures/appintents/DataApp.swiftconstvalues"
        ))
        .unwrap();
        let actual = strings_data(&types, &extract(&types).unwrap()).unwrap();
        let expected: Value = serde_json::from_str(include_str!(
            "../../../fixtures/appintents/Shortcuts.stringsdata"
        ))
        .unwrap();
        assert_eq!(actual, expected);
    }
    fn normalize(value: &mut Value) {
        match value {
            Value::Object(object) => {
                for (key, v) in object {
                    if key == "generator" {
                        *v = json!({"name":"normalized"});
                    } else if key == "resolvableInputTypes" {
                        v.as_array_mut().unwrap().sort_by_key(Value::to_string);
                    } else {
                        normalize(v);
                    }
                }
            }
            Value::Array(values) => values.iter_mut().for_each(normalize),
            _ => {}
        }
    }
    #[test]
    fn intent_enum_entity_query_and_shortcut_metadata_match_xcode() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/appintents");
        for (input, expected) in [
            ("DataApp.swiftconstvalues", "extract.actionsdata"),
            ("Types.swiftconstvalues", "Types.actionsdata"),
            ("Files.swiftconstvalues", "Files.actionsdata"),
        ] {
            let types: Vec<Value> =
                serde_json::from_slice(&std::fs::read(dir.join(input)).unwrap()).unwrap();
            let mut actual = extract(&types).unwrap();
            let mut expected: Value =
                serde_json::from_slice(&std::fs::read(dir.join(expected)).unwrap()).unwrap();
            normalize(&mut actual);
            normalize(&mut expected);
            assert_eq!(actual, expected, "{input}");
        }
    }
}
