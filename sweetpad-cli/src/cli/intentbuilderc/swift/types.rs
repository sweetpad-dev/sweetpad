//! INObject and enum declarations generated from synthetic native captures.
use super::{ASYNC, AVAIL, Element, declaration, field, identifier, list, text, yes};
use std::fmt::Write as _;

pub(super) fn enumeration(value: &Element) -> Result<(String, String), String> {
    let name = identifier(text(value, "INEnumName")?)?;
    if text(value, "INEnumType")? != "Regular" {
        return Err("only regular intent enums are supported".into());
    }
    let mut out = format!(
        "#if canImport(Intents)\nimport Intents\n{AVAIL}\n@objc public enum {name}: Int {{\n"
    );
    for (index, case) in list(value, "INEnumValues").enumerate() {
        let case_name = identifier(text(case, "INEnumValueName")?)?;
        let index = field(case, "INEnumValueIndex")
            .map(|v| v.text.parse::<i64>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(i64::try_from(index).map_err(|e| e.to_string())?);
        out.push_str("case ");
        out.push('`');
        out.push_str(case_name);
        out.push('`');
        writeln!(out, " = {index}").unwrap();
    }
    writeln!(out,"}}\n{ASYNC} @available(tvOS, unavailable)\n@objc({name}ResolutionResult) public class {name}ResolutionResult: INEnumResolutionResult {{\n@objc(successWithResolved{name}:) public class func success(with resolvedValue: {name}) -> Self {{\nreturn __success(withResolvedValue: resolvedValue.rawValue)\n}}\n@objc(confirmationRequiredWith{name}ToConfirm:) public class func confirmationRequired(with valueToConfirm: {name}) -> Self {{\nreturn __confirmationRequiredWithValue(toConfirm: valueToConfirm.rawValue)\n}}\n}}\n#endif").unwrap();
    Ok((format!("{name}.swift"), out))
}
pub(super) fn object(value: &Element) -> Result<(String, String), String> {
    let name = identifier(text(value, "INTypeName")?)?;
    let mut out = format!(
        "#if canImport(Intents)\nimport Intents\n{AVAIL}\n@objc({name}) public class {name}: INObject {{\n"
    );
    for property in list(value, "INTypeProperties").filter(|p| !yes(p, "INTypePropertyDefault")) {
        writeln!(out, "{ASYNC}").unwrap();
        declaration(&mut out, property, "INTypeProperty")?;
    }
    writeln!(out,"override public class var supportsSecureCoding: Bool {{ true }}\n}}\n{ASYNC} @available(tvOS, unavailable)\n@objc({name}ResolutionResult) public class {name}ResolutionResult: INObjectResolutionResult {{\n@objc(successWithResolved{name}:) public class func success(with resolvedObject: {name}) -> Self {{\nreturn super.success(with: resolvedObject)\n}}\n@objc(disambiguationWith{name}sToDisambiguate:) public class func disambiguation(with objectsToDisambiguate: [{name}]) -> Self {{\nreturn super.disambiguation(with: objectsToDisambiguate)\n}}\n@objc(confirmationRequiredWith{name}ToConfirm:) public class func confirmationRequired(with objectToConfirm: {name}?) -> Self {{\nreturn super.confirmationRequired(with: objectToConfirm)\n}}\n@available(*, unavailable) override public class func success(with resolvedObject: INObject) -> Self {{ fatalError() }}\n@available(*, unavailable) override public class func disambiguation(with objectsToDisambiguate: [INObject]) -> Self {{ fatalError() }}\n@available(*, unavailable) override public class func confirmationRequired(with objectToConfirm: INObject?) -> Self {{ fatalError() }}\n}}\n#endif").unwrap();
    Ok((format!("{name}.swift"), out))
}
