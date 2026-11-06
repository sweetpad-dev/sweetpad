//! `xcstringstool compile`: a String Catalog as the `.strings` and
//! `.stringsdict` tables of each language, which is what the app reads at
//! run time.
//!
//! A language's `.strings` holds its plain strings and `.stringsdict` the
//! ones that vary, by plural, device or width. In the source language a
//! string with no value of its own is left out, since the key is its text,
//! unless it was added by hand or every string is forced in. A string that
//! can't compile, like a plural that never mentions its number, fails its
//! language, and the languages without one are still written.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use super::format::{self, Token};
use super::{Catalog, Problem, symbols};

/// What to compile.
#[derive(Debug, Default)]
pub struct Options {
    /// The languages to compile, or every one when empty.
    pub languages: Vec<String>,
    /// Put every string in the source language's table, valued or not.
    pub force_all: bool,
}

/// One language's tables.
#[derive(Debug, Default)]
pub struct Tables {
    pub strings: Map<String, Value>,
    pub dict: Map<String, Value>,
}

/// The catalog compiled: the languages that compiled, and why the others
/// didn't.
#[derive(Debug, Default)]
pub struct Compiled {
    pub languages: BTreeMap<String, Tables>,
    pub problems: Vec<Problem>,
}

/// What one string compiles to in one language.
#[derive(Debug, Default)]
struct Entry {
    string: Option<String>,
    dict: Option<Value>,
}

/// Why a string doesn't compile: the variation it's in, if any, and the
/// message.
type Failure = (Option<String>, String);

pub fn compile(catalog: &Catalog, options: &Options) -> Compiled {
    let mut compiled = Compiled::default();
    for language in languages(catalog, options) {
        let mut tables = Tables::default();
        let mut failed = false;
        for (key, entry) in &catalog.strings {
            let result = match localization(entry, &language) {
                Some(localization) => compile_localization(localization),
                None if gets_key(catalog, key, entry, &language, options) => Ok(Entry {
                    string: Some(key.clone()),
                    dict: None,
                }),
                None => continue,
            };
            match result {
                Ok(compiled) => {
                    if let Some(string) = compiled.string {
                        tables.strings.insert(key.clone(), Value::String(string));
                    }
                    if let Some(dict) = compiled.dict {
                        tables.dict.insert(key.clone(), dict);
                    }
                }
                Err((path, message)) => {
                    failed = true;
                    compiled.problems.push(Problem {
                        scope: language.clone(),
                        key: key.clone(),
                        path,
                        message,
                    });
                }
            }
        }
        if !failed {
            compiled.languages.insert(language, tables);
        }
    }
    compiled
}

/// The tables a compile would write, by language: whether each has a
/// `.strings` and a `.stringsdict`. Like Xcode's `--dry-run`, it goes by the
/// catalog's shape and doesn't check that each string compiles.
pub fn planned(catalog: &Catalog, options: &Options) -> Vec<(String, bool, bool)> {
    languages(catalog, options)
        .into_iter()
        .filter_map(|language| {
            let (mut strings, mut dict) = (false, false);
            for (key, entry) in &catalog.strings {
                let (string, varied) = match localization(entry, &language) {
                    Some(localization) => shape(localization),
                    None => (gets_key(catalog, key, entry, &language, options), false),
                };
                strings |= string;
                dict |= varied;
            }
            (strings || dict).then_some((language, strings, dict))
        })
        .collect()
}

/// The source language and every language a string has a localization in,
/// narrowed to the ones asked for.
fn languages(catalog: &Catalog, options: &Options) -> Vec<String> {
    let mut all = vec![catalog.source_language.clone()];
    for entry in catalog.strings.values() {
        if let Some(localizations) = entry.get("localizations").and_then(Value::as_object) {
            all.extend(localizations.keys().cloned());
        }
    }
    all.sort();
    all.dedup();
    all.retain(|l| options.languages.is_empty() || options.languages.contains(l));
    all
}

fn localization<'a>(entry: &'a Value, language: &str) -> Option<&'a Value> {
    entry.get("localizations")?.get(language)
}

/// Whether a string with no localization in `language` goes in with its key
/// as its text: in the source language when it was added by hand or every
/// string is forced in, and in any other when it was added by hand and its
/// symbol takes arguments. That symbol's default value is just the
/// arguments, which a language without the string would show.
fn gets_key(
    catalog: &Catalog,
    key: &str,
    entry: &Value,
    language: &str,
    options: &Options,
) -> bool {
    let manual = entry.get("extractionState").and_then(Value::as_str) == Some("manual");
    if language == catalog.source_language {
        return options.force_all || manual;
    }
    manual && symbols::takes_arguments(entry, key, &catalog.source_language)
}

/// Whether a localization goes in the `.strings` table, the `.stringsdict`
/// one, or both.
fn shape(localization: &Value) -> (bool, bool) {
    if let Some((kind, cases)) = variations(localization) {
        return match kind {
            "device" => (
                cases
                    .get("other")
                    .is_some_and(|other| variations(other).is_none() && unit(other).is_some()),
                cases.keys().any(|device| device != "other"),
            ),
            _ => (false, true),
        };
    }
    match unit(localization) {
        Some(value) if substitutions(localization).is_some() && refers(value) => (false, true),
        Some(_) => (true, false),
        None => (false, false),
    }
}

fn compile_localization(localization: &Value) -> Result<Entry, Failure> {
    let subs = substitutions(localization);
    if let Some((kind, cases)) = variations(localization) {
        return match kind {
            "plural" => Ok(Entry {
                string: None,
                dict: Some(plural(cases, None)?),
            }),
            "device" => device(cases, subs),
            "width" => Ok(Entry {
                string: None,
                dict: Some(json!({ "NSStringVariableWidthRuleType": units(cases) })),
            }),
            _ => Ok(Entry::default()),
        };
    }
    let Some(value) = unit(localization) else {
        return Ok(Entry::default());
    };
    match subs {
        Some(subs) if refers(value) => Ok(Entry {
            string: None,
            dict: Some(substituted(value, subs).map_err(|m| (None, m))?),
        }),
        _ => Ok(Entry {
            string: Some(value.to_string()),
            dict: None,
        }),
    }
}

/// A localization's variations, by the one kind it varies by.
fn variations(value: &Value) -> Option<(&str, &Map<String, Value>)> {
    let (kind, cases) = value.get("variations")?.as_object()?.iter().next()?;
    Some((kind.as_str(), cases.as_object()?))
}

fn substitutions(value: &Value) -> Option<&Map<String, Value>> {
    value
        .get("substitutions")?
        .as_object()
        .filter(|s| !s.is_empty())
}

fn unit(value: &Value) -> Option<&str> {
    value.get("stringUnit")?.get("value")?.as_str()
}

/// Each case's plain value.
fn units(cases: &Map<String, Value>) -> Map<String, Value> {
    cases
        .iter()
        .filter_map(|(case, v)| Some((case.clone(), json!(unit(v)?))))
        .collect()
}

/// Whether `value` names a substitution.
fn refers(value: &str) -> bool {
    format::tokens(value)
        .iter()
        .any(|t| matches!(t, Token::Substitution { .. }))
}

/// A plural variation as a `.stringsdict` entry. The number it's chosen by
/// is the one numeric specifier its cases share; when that isn't the first
/// argument, every specifier is given its position so the format key can
/// name it.
fn plural(cases: &Map<String, Value>, at: Option<&str>) -> Result<Value, Failure> {
    let mut values = Vec::new();
    for (case, variant) in cases {
        if variations(variant).is_some() {
            let path = at.map_or_else(|| format!("plural.{case}"), |at| format!("{at}.{case}"));
            return Err((Some(path), "Plural case cannot be further varied".into()));
        }
        if let Some(value) = unit(variant) {
            values.push((case, value));
        }
    }
    let fail = |message: &str| Err((at.map(str::to_string), message.to_string()));
    let mut number = None;
    for (_, value) in &values {
        let tokens = format::tokens(value);
        let numeric: Vec<(usize, &Token)> = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| t.is_numeric())
            .collect();
        match numeric.as_slice() {
            [] => {}
            [(index, token)] => {
                number.get_or_insert((
                    token.position().unwrap_or(index + 1),
                    token.position().is_some(),
                    token.value_type(),
                ));
            }
            _ => {
                return fail(
                    "Cannot infer format specifier for string group because more than one \
                     numerical specifier was found",
                );
            }
        }
    }
    let Some((position, explicit, value_type)) = number else {
        return fail(
            "Plural variation requires referencing the number in the string. To maintain \
             grammatical correctness for strings that do not reference the number of items, use \
             separate top-level strings for one and greater than one.",
        );
    };
    let rewrite = !explicit && position > 1;
    let mut rule = Map::new();
    rule.insert(
        "NSStringFormatSpecTypeKey".into(),
        json!("NSStringPluralRuleType"),
    );
    rule.insert("NSStringFormatValueTypeKey".into(), json!(value_type));
    for (case, value) in values {
        let value = if rewrite {
            format::positioned(value)
        } else {
            value.to_string()
        };
        rule.insert(case.clone(), json!(value));
    }
    let format_key = if explicit || position > 1 {
        format!("%{position}$#@value@")
    } else {
        "%#@value@".to_string()
    };
    Ok(json!({ "NSStringLocalizedFormatKey": format_key, "value": rule }))
}

/// A device variation: its `other` case is the plain string, and the rest
/// go in the `.stringsdict`.
fn device(cases: &Map<String, Value>, subs: Option<&Map<String, Value>>) -> Result<Entry, Failure> {
    let mut entry = Entry::default();
    let mut rules = Map::new();
    for (device, variant) in cases {
        let at = format!("device.{device}");
        if device == "other" {
            if variations(variant).is_some() {
                return Err((Some(at), "Fallback value cannot be further varied".into()));
            }
            entry.string = unit(variant).map(str::to_string);
            continue;
        }
        let rule = match (variations(variant), unit(variant)) {
            (Some(("plural", cases)), _) => plural(cases, Some(&at))?,
            (Some(_), _) | (None, None) => continue,
            (None, Some(value)) => match subs {
                Some(subs) if refers(value) => {
                    substituted(value, subs).map_err(|m| (Some(at.clone()), m))?
                }
                _ => json!(value),
            },
        };
        rules.insert(device.clone(), rule);
    }
    if !rules.is_empty() {
        entry.dict = Some(json!({ "NSStringDeviceSpecificRuleType": rules }));
    }
    Ok(entry)
}

/// A string that names substitutions as a `.stringsdict` entry: the string
/// is the format key, and each substitution it names is a plural rule whose
/// `%arg` is the substitution's argument.
fn substituted(value: &str, subs: &Map<String, Value>) -> Result<Value, String> {
    let mut entry = Map::new();
    let mut format_key = String::new();
    let mut last = 0;
    for token in format::tokens(value) {
        let Token::Substitution {
            range,
            position,
            name,
        } = token
        else {
            continue;
        };
        let sub = subs
            .get(name)
            .ok_or_else(|| format!("Referencing undefined substitution '{name}'"))?;
        let arg = sub.get("argNum").and_then(Value::as_u64);
        let spec = sub
            .get("formatSpecifier")
            .and_then(Value::as_str)
            .unwrap_or("@");
        format_key.push_str(&value[last..range.start]);
        match position.or(arg.and_then(|a| usize::try_from(a).ok())) {
            Some(p) => {
                let _ = write!(format_key, "%{p}$#@{name}@");
            }
            None => {
                let _ = write!(format_key, "%#@{name}@");
            }
        }
        last = range.end;
        let Some(("plural", cases)) = variations(sub) else {
            return Err(format!(
                "Cannot reference substitution '{name}' from here because it is not a plain \
                 string or plural variation"
            ));
        };
        let placeholder = match arg {
            Some(arg) => format!("%{arg}${spec}"),
            None => format!("%{spec}"),
        };
        let mut rule = Map::new();
        rule.insert(
            "NSStringFormatSpecTypeKey".into(),
            json!("NSStringPluralRuleType"),
        );
        rule.insert("NSStringFormatValueTypeKey".into(), json!(spec));
        for (case, text) in units(cases) {
            let text = text
                .as_str()
                .unwrap_or_default()
                .replace("%arg", &placeholder);
            rule.insert(case, json!(text));
        }
        entry.insert(name.to_string(), Value::Object(rule));
    }
    format_key.push_str(&value[last..]);
    entry.insert("NSStringLocalizedFormatKey".into(), json!(format_key));
    Ok(Value::Object(entry))
}
