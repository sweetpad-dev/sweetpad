//! Declaration shapes captured from Xcode 27's Swift intent generator.
use super::{Element, capitalized, identifier, list, text, yes};
use std::fmt::Write as _;
const AVAIL: &str =
    "@available(iOS 12.0, macOS 11.0, watchOS 5.0, *) @available(tvOS, unavailable)";
const ASYNC: &str = "@available(iOS 13.0, macOS 11.0, watchOS 6.0, *)";
const DEFAULT: &str = "@available(iOS 14.0, macOS 11.0, watchOS 7.0, *)";
pub(super) fn generate(model: &Element) -> Result<Vec<(String, String)>, String> {
    if list(model, "INEnums").next().is_some() || list(model, "INTypes").next().is_some() {
        return Err("custom intent types and enums aren't supported".into());
    }
    list(model, "INIntents").map(intent).collect()
}
fn intent(model: &Element) -> Result<(String, String), String> {
    let name = identifier(text(model, "INIntentName")?)?;
    let class = format!("{name}Intent");
    let mut out = format!(
        "#if canImport(Intents)\nimport Intents\n{AVAIL}\n@objc({class}) public class {class}: INIntent {{\n"
    );
    let parameters: Vec<_> = list(model, "INIntentParameters").collect();
    for p in &parameters {
        let property = identifier(text(p, "INIntentParameterName")?)?;
        let kind = kind(p)?;
        writeln!(out, "@NSManaged public var {property}: {kind}?").unwrap();
    }
    writeln!(
        out,
        "}}\n{AVAIL}\n@objc({class}Handling) public protocol {class}Handling: NSObjectProtocol {{"
    )
    .unwrap();
    writeln!(out,"@available(*, renamed: \"handle(intent:)\")\n@objc(handle{name}:completion:)\nfunc handle(intent: {class}, completion: @escaping ({class}Response) -> Swift.Void)\n{ASYNC}\n@objc(handle{name}:completion:)\nfunc handle(intent: {class}) async -> {class}Response").unwrap();
    for p in parameters
        .iter()
        .filter(|p| yes(p, "INIntentParameterSupportsResolution"))
    {
        resolution(&mut out, name, p)?;
    }
    for p in parameters
        .iter()
        .filter(|p| yes(p, "INIntentParameterSupportsDynamicEnumeration"))
    {
        options(&mut out, name, p)?;
    }
    writeln!(out,"@available(*, renamed: \"confirm(intent:)\")\n@objc(confirm{name}:completion:)\noptional func confirm(intent: {class}, completion: @escaping ({class}Response) -> Swift.Void)\n{ASYNC}\n@objc(confirm{name}:completion:)\noptional func confirm(intent: {class}) async -> {class}Response").unwrap();
    for p in parameters
        .iter()
        .filter(|p| yes(p, "INIntentParameterSupportsDynamicEnumeration"))
    {
        let property = identifier(text(p, "INIntentParameterName")?)?;
        let title = capitalized(property);
        let kind = kind(p)?;
        writeln!(out,"{DEFAULT}\n@objc(default{title}For{name}:)\noptional func default{title}(for intent: {class}) -> {kind}?").unwrap();
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
fn kind(parameter: &Element) -> Result<&'static str, String> {
    match text(parameter, "INIntentParameterType")? {
        "String" => Ok("String"),
        "URL" => Ok("URL"),
        other => Err(format!("unsupported intent parameter type {other}")),
    }
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
    } else {
        format!("IN{}ResolutionResult", kind(p)?)
    };
    writeln!(out,"{ASYNC}\n@available(*, renamed: \"resolve{title}(for:)\")\n@objc(resolve{title}For{name}:withCompletion:)\nfunc resolve{title}(for intent: {class}, with completion: @escaping ({result}) -> Swift.Void)\n{ASYNC}\n@objc(resolve{title}For{name}:withCompletion:)\nfunc resolve{title}(for intent: {class}) async -> {result}").unwrap();
    Ok(())
}
fn options(out: &mut String, name: &str, p: &Element) -> Result<(), String> {
    let property = identifier(text(p, "INIntentParameterName")?)?;
    let title = capitalized(property);
    let class = format!("{name}Intent");
    let kind = kind(p)?;
    writeln!(out,"{ASYNC}\n@available(*, renamed: \"provide{title}Options(for:)\")\n@objc(provide{title}OptionsFor{name}:withCompletion:)\noptional func provide{title}Options(for intent: {class}, with completion: @escaping ([{kind}]?, Error?) -> Swift.Void)\n{ASYNC}\n@objc(provide{title}OptionsFor{name}:withCompletion:)\noptional func provide{title}Options(for intent: {class}) async throws -> [{kind}]").unwrap();
    Ok(())
}
fn response(out: &mut String, name: &str, model: &Element) -> Result<(), String> {
    let class = format!("{name}Intent");
    let response = super::field(model, "INIntentResponse").ok_or("an intent has no response")?;
    if list(response, "INIntentResponseParameters")
        .next()
        .is_some()
    {
        return Err("intent response parameters aren't supported".into());
    }
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
    for (index, code) in codes.iter().enumerate() {
        if index == 0 {
            writeln!(out, "case {code} = 0").unwrap();
        } else {
            writeln!(out, "case {code}").unwrap();
        }
    }
    writeln!(out,"}}\n{AVAIL}\n@objc({class}Response) public class {class}Response: INIntentResponse {{\n@objc public fileprivate(set) var code: {class}ResponseCode = .unspecified\n@objc(initWithCode:userActivity:) public convenience init(code: {class}ResponseCode, userActivity: NSUserActivity?) {{\nself.init()\nself.code = code\nself.userActivity = userActivity\n}}\n}}").unwrap();
    Ok(())
}
fn unsupported(out: &mut String, name: &str, p: &Element) -> Result<(), String> {
    let reasons: Vec<_> = list(p, "INIntentParameterUnsupportedReasons").collect();
    if reasons.is_empty() {
        return Ok(());
    }
    let property = identifier(text(p, "INIntentParameterName")?)?;
    let title = capitalized(property);
    let class = format!("{name}{title}");
    let kind = kind(p)?;
    writeln!(
        out,
        "{ASYNC}\n@objc public enum {class}UnsupportedReason: Int {{"
    )
    .unwrap();
    for (index, reason) in reasons.iter().enumerate() {
        let code = identifier(text(reason, "INIntentParameterUnsupportedReasonCode")?)?;
        if index == 0 {
            writeln!(out, "case {code} = 1").unwrap();
        } else {
            writeln!(out, "case {code}").unwrap();
        }
    }
    writeln!(out,"}}\n{ASYNC}\n@objc({class}ResolutionResult) public class {class}ResolutionResult: IN{kind}ResolutionResult {{\n@objc(unsupportedForReason:) public class func unsupported(forReason reason: {class}UnsupportedReason) -> Self {{\nreturn __unsupported(withReason: reason.rawValue)\n}}\n}}").unwrap();
    Ok(())
}
