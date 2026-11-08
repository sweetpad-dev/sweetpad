//! Declaration shapes captured from Xcode 27's Swift intent generator.
use super::{Element, capitalized, field, identifier, list, text, yes};
use std::fmt::Write as _;
mod types;
const AVAIL: &str =
    "@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)";
const ASYNC: &str = "@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)";
const DEFAULT: &str = "@available(iOS 14.0, macOS 11.0, watchOS 7.0, *)";
pub(super) fn generate(model: &Element) -> Result<Vec<(String, String)>, String> {
    let modern = text(model, "INIntentDefinitionModelVersion").unwrap_or("1.1") != "1.1";
    let mut sources = list(model, "INIntents")
        .map(|m| intent(m, modern))
        .collect::<Result<Vec<_>, _>>()?;
    for value in list(model, "INEnums") {
        sources.push(types::enumeration(value)?);
    }
    for value in list(model, "INTypes") {
        sources.push(types::object(value)?);
    }
    Ok(sources)
}
fn intent(model: &Element, modern: bool) -> Result<(String, String), String> {
    let name = identifier(text(model, "INIntentName")?)?;
    let class = format!("{name}Intent");
    let mut out = format!(
        "#if canImport(Intents)\nimport Intents\n{AVAIL}\n@objc({class}) public class {class}: INIntent {{\n"
    );
    let parameters: Vec<_> = list(model, "INIntentParameters").collect();
    for p in &parameters {
        declaration(&mut out, p, "INIntentParameter")?;
    }
    writeln!(
        out,
        "}}\n{AVAIL}\n@objc({class}Handling) public protocol {class}Handling: NSObjectProtocol {{"
    )
    .unwrap();
    let widget = yes(model, "INIntentEligibleForWidgets");
    if !widget {
        handling(&mut out, name, false);
    }
    for p in parameters
        .iter()
        .filter(|p| yes(p, "INIntentParameterSupportsResolution"))
    {
        resolution(&mut out, name, p)?;
    }
    let mut dynamic: Vec<_> = parameters
        .iter()
        .copied()
        .filter(|p| yes(p, "INIntentParameterSupportsDynamicEnumeration"))
        .collect();
    dynamic.sort_by_key(|p| !yes(p, "INIntentParameterSupportsSearch"));
    for p in &dynamic {
        options(&mut out, name, p, modern)?;
    }
    writeln!(out,"@available(*, renamed: \"confirm(intent:)\")\n@objc(confirm{name}:completion:)\noptional func confirm(intent: {class}, completion: @escaping ({class}Response) -> Swift.Void)\n{ASYNC}\n@objc(confirm{name}:completion:)\noptional func confirm(intent: {class}) async -> {class}Response").unwrap();
    if widget {
        handling(&mut out, name, true);
    }
    for p in parameters
        .iter()
        .filter(|p| yes(p, "INIntentParameterSupportsDynamicEnumeration"))
    {
        let property = identifier(text(p, "INIntentParameterName")?)?;
        let title = capitalized(property);
        let kind = property_type(p, "INIntentParameter")?;
        writeln!(out,"{DEFAULT}\n@objc(default{title}For{name}:)\noptional func default{title}(for intent: {class}) -> {kind}").unwrap();
    }
    if modern {
        for p in parameters
            .iter()
            .filter(|p| yes(p, "INIntentParameterSupportsDynamicEnumeration"))
        {
            deprecated_options(&mut out, name, p)?;
        }
    }
    out.push_str("}\n");
    response(&mut out, name, model)?;
    for p in parameters
        .iter()
        .filter(|p| yes(p, "INIntentParameterSupportsResolution"))
    {
        unsupported(&mut out, name, p)?;
    }
    out.push_str("#endif\n");
    Ok((format!("{class}.swift"), out))
}
fn handling(out: &mut String, name: &str, optional: bool) {
    let optional = if optional { "optional " } else { "" };
    let class = format!("{name}Intent");
    writeln!(out,"@available(*, renamed: \"handle(intent:)\")\n@objc(handle{name}:completion:)\n{optional}func handle(intent: {class}, completion: @escaping ({class}Response) -> Swift.Void)\n{ASYNC}\n@objc(handle{name}:completion:)\n{optional}func handle(intent: {class}) async -> {class}Response").unwrap();
}
fn kind(value: &Element, prefix: &str) -> Result<String, String> {
    if let Some(enumeration) = field(value, &format!("{prefix}EnumType")) {
        return identifier(&enumeration.text).map(str::to_string);
    }
    Ok(match text(value, &format!("{prefix}Type"))? {
        "String" => "String".to_string(),
        "URL" => "URL".to_string(),
        "Boolean" => "NSNumber".to_string(),
        "Object" => field(value, &format!("{prefix}ObjectType"))
            .map(|v| identifier(&v.text))
            .transpose()?
            .unwrap_or("INObject")
            .to_string(),
        other => return Err(format!("unsupported intent parameter type {other}")),
    })
}
fn property_type(value: &Element, prefix: &str) -> Result<String, String> {
    let base = kind(value, prefix)?;
    Ok(if yes(value, &format!("{prefix}SupportsMultipleValues")) {
        format!("[{base}]?")
    } else if field(value, &format!("{prefix}EnumType")).is_some() {
        base
    } else {
        format!("{base}?")
    })
}
fn declaration(out: &mut String, value: &Element, prefix: &str) -> Result<(), String> {
    let name = identifier(text(value, &format!("{prefix}Name"))?)?;
    if text(value, &format!("{prefix}Type"))? == "Boolean" {
        out.push_str("@available(iOS 13.0, macOS 10.16, watchOS 6.0, *)\n");
    }
    writeln!(
        out,
        "@NSManaged public var {name}: {}",
        property_type(value, prefix)?
    )
    .unwrap();
    Ok(())
}
fn resolution(out: &mut String, name: &str, p: &Element) -> Result<(), String> {
    let property = identifier(text(p, "INIntentParameterName")?)?;
    let title = capitalized(property);
    let class = format!("{name}Intent");
    let result = if list(p, "INIntentParameterUnsupportedReasons")
        .next()
        .is_some()
    {
        format!("{name}{title}ResolutionResult")
    } else if field(p, "INIntentParameterEnumType").is_some()
        || field(p, "INIntentParameterObjectType").is_some()
    {
        format!("{}ResolutionResult", kind(p, "INIntentParameter")?)
    } else {
        format!("IN{}ResolutionResult", text(p, "INIntentParameterType")?)
    };
    let result = if yes(p, "INIntentParameterSupportsMultipleValues") {
        format!("[{result}]")
    } else {
        result
    };
    writeln!(out,"{ASYNC}\n@available(*, renamed: \"resolve{title}(for:)\")\n@objc(resolve{title}For{name}:withCompletion:)\nfunc resolve{title}(for intent: {class}, with completion: @escaping ({result}) -> Swift.Void)\n{ASYNC}\n@objc(resolve{title}For{name}:withCompletion:)\nfunc resolve{title}(for intent: {class}) async -> {result}").unwrap();
    Ok(())
}
fn options(out: &mut String, name: &str, p: &Element, modern: bool) -> Result<(), String> {
    let property = identifier(text(p, "INIntentParameterName")?)?;
    let title = capitalized(property);
    let class = format!("{name}Intent");
    let kind = kind(p, "INIntentParameter")?;
    if !modern {
        writeln!(out,"{ASYNC}\n@available(*, renamed: \"provide{title}Options(for:)\")\n@objc(provide{title}OptionsFor{name}:withCompletion:)\noptional func provide{title}Options(for intent: {class}, with completion: @escaping ([{kind}]?, Error?) -> Swift.Void)\n{ASYNC}\n@objc(provide{title}OptionsFor{name}:withCompletion:)\noptional func provide{title}Options(for intent: {class}) async throws -> [{kind}]").unwrap();
        return Ok(());
    }
    let kind = if kind == "String" { "NSString" } else { &kind };
    let search = yes(p, "INIntentParameterSupportsSearch");
    let suffix = if search { "searchTerm:" } else { "" };
    let argument = if search { ", searchTerm: String?" } else { "" };
    writeln!(out,"{DEFAULT}\n@available(*, renamed: \"provide{title}OptionsCollection(for:{suffix})\")\n@objc(provide{title}OptionsCollectionFor{name}:{suffix}withCompletion:)\nfunc provide{title}OptionsCollection(for intent: {class}{argument}, with completion: @escaping (INObjectCollection<{kind}>?, Error?) -> Swift.Void)\n{DEFAULT}\n@objc(provide{title}OptionsCollectionFor{name}:{suffix}withCompletion:)\nfunc provide{title}OptionsCollection(for intent: {class}{argument}) async throws -> INObjectCollection<{kind}>").unwrap();
    Ok(())
}
fn deprecated_options(out: &mut String, name: &str, p: &Element) -> Result<(), String> {
    let title = capitalized(identifier(text(p, "INIntentParameterName")?)?);
    let kind = kind(p, "INIntentParameter")?;
    writeln!(out,"@available(iOS, introduced: 13.0, deprecated: 14.0, message: \"\")\n@available(watchOS, introduced: 6.0, deprecated: 7.0, message: \"\")\n@objc(provide{title}OptionsFor{name}:withCompletion:)\noptional func provide{title}Options(for intent: {name}Intent, with completion: @escaping ([{kind}]?, Error?) -> Swift.Void)").unwrap();
    Ok(())
}
fn response(out: &mut String, name: &str, model: &Element) -> Result<(), String> {
    let class = format!("{name}Intent");
    let response = field(model, "INIntentResponse").ok_or("an intent has no response")?;
    let mut codes = vec![
        "unspecified".to_string(),
        "ready".into(),
        "continueInApp".into(),
        "inProgress".into(),
    ];
    for code in list(response, "INIntentResponseCodes") {
        let name = identifier(text(code, "INIntentResponseCodeName")?)?;
        if !codes.iter().any(|c| c == name) {
            codes.push(name.to_string());
        }
    }
    if !codes.iter().any(|c| c == "failureRequiringAppLaunch") {
        codes.push("failureRequiringAppLaunch".into());
    }
    writeln!(
        out,
        "{AVAIL}\n@objc public enum {class}ResponseCode: Int {{"
    )
    .unwrap();
    for (i, code) in codes.iter().enumerate() {
        writeln!(out, "case {code}{}", if i == 0 { " = 0" } else { "" }).unwrap();
    }
    writeln!(
        out,
        "}}\n{AVAIL}\n@objc({class}Response) public class {class}Response: INIntentResponse {{"
    )
    .unwrap();
    for p in list(response, "INIntentResponseParameters") {
        declaration(out, p, "INIntentResponseParameter")?;
    }
    writeln!(out,"@objc public fileprivate(set) var code: {class}ResponseCode = .unspecified\n@objc(initWithCode:userActivity:) public convenience init(code: {class}ResponseCode, userActivity: NSUserActivity?) {{\nself.init()\nself.code = code\nself.userActivity = userActivity\n}}").unwrap();
    for code in list(response, "INIntentResponseCodes") {
        response_helper(out, &class, response, code)?;
    }
    out.push_str("}\n");
    Ok(())
}
fn response_helper(
    out: &mut String,
    class: &str,
    response: &Element,
    code: &Element,
) -> Result<(), String> {
    let mut names = Vec::new();
    let mut format = text(code, "INIntentResponseCodeFormatString").unwrap_or_default();
    while let Some((_, rest)) = format.split_once("${") {
        let Some((name, rest)) = rest.split_once('}') else {
            break;
        };
        if !names.contains(&name) {
            names.push(identifier(name)?);
        }
        format = rest;
    }
    if names.is_empty() {
        return Ok(());
    }
    let name = identifier(text(code, "INIntentResponseCodeName")?)?;
    let mut parameters = Vec::new();
    let mut assignments = String::new();
    let mut selector = String::new();
    for (index, name) in names.iter().enumerate() {
        let p = list(response, "INIntentResponseParameters")
            .find(|p| text(p, "INIntentResponseParameterName") == Ok(*name))
            .ok_or_else(|| format!("unknown response parameter '{name}'"))?;
        let kind = property_type(p, "INIntentResponseParameter")?;
        parameters.push(format!("{name}: {}", kind.trim_end_matches('?')));
        writeln!(assignments, "intentResponse.{name} = {name}").unwrap();
        write!(
            selector,
            "{}:",
            if index == 0 {
                capitalized(name)
            } else {
                (*name).to_string()
            }
        )
        .unwrap();
    }
    writeln!(out,"@objc({name}IntentResponseWith{selector})\npublic static func {name}({}) -> {class}Response {{\nlet intentResponse = {class}Response(code: .{name}, userActivity: nil)\n{assignments}return intentResponse\n}}",parameters.join(", ")).unwrap();
    Ok(())
}
fn unsupported(out: &mut String, name: &str, p: &Element) -> Result<(), String> {
    let reasons: Vec<_> = list(p, "INIntentParameterUnsupportedReasons").collect();
    if reasons.is_empty() {
        return Ok(());
    }
    let title = capitalized(identifier(text(p, "INIntentParameterName")?)?);
    let class = format!("{name}{title}");
    let kind = kind(p, "INIntentParameter")?;
    writeln!(
        out,
        "{ASYNC}\n@objc public enum {class}UnsupportedReason: Int {{"
    )
    .unwrap();
    for (i, reason) in reasons.iter().enumerate() {
        let code = identifier(text(reason, "INIntentParameterUnsupportedReasonCode")?)?;
        writeln!(out, "case {code}{}", if i == 0 { " = 1" } else { "" }).unwrap();
    }
    writeln!(out,"}}\n{ASYNC}\n@objc({class}ResolutionResult) public class {class}ResolutionResult: IN{kind}ResolutionResult {{\n@objc(unsupportedForReason:) public class func unsupported(forReason reason: {class}UnsupportedReason) -> Self {{\nreturn __unsupported(withReason: reason.rawValue)\n}}\n}}").unwrap();
    Ok(())
}
