//! Cross-checks for the `project.xcproj` parser.
//!
//! On the JSON subset of the format the parser must agree with `serde_json`,
//! which is what makes the extensions it adds on top legible: the same
//! documents, plus trailing commas and comments.

use serde_json::Value as Json;
use sweetpad_lib::xcproj::{self, Value};

/// A document in Xcode's canonical printing, trailing commas and all.
const CANONICAL: &str = r#"{
  "default-configuration": "Release",
  "configurations": [
    "Debug",
    { "name": "Release", "file": { "anchor": "group", "relative-path": "Config/Release.xcconfig" } },
  ],
  "localizations": {
    "development": "en",
    "supported": [
      "en",
      "fr",
    ],
  },
  "files": [
    { "kind": "folder", "path": "Shared", "target-membership": [ "MyApp" ] },
  ],
}
"#;

/// The documents below are strict JSON, so both parsers must accept them and
/// see the same thing.
const STRICT: &[&str] = &[
    r"{}",
    r"[]",
    r#"{"a": 1, "b": [true, false, null], "c": {"d": "e"}}"#,
    r#"{"nested": [[[{"deep": [1, 2, 3]}]]]}"#,
    r#"{"escapes": "a\"b\\c\/d\b\f\n\r\teé😀"}"#,
    r#"{"numbers": [0, -1, 1.5, -1.5e3, 1E+2, 0.0]}"#,
    r#"{"unicode": "héllo 😀 日本語"}"#,
    r#"{"empty": {"a": {}, "b": []}}"#,
    r#"{"SWIFT_OPTIMIZATION_LEVEL[config=Debug][sdk=iphoneos*]": "-Onone"}"#,
    r#""just a string""#,
    r"42",
    r"true",
    r"null",
];

/// Equality across the two representations. Numbers compare by their numeric
/// value, since one side keeps the lexeme and the other does not.
fn same(ours: &Value, theirs: &Json) -> bool {
    match (ours, theirs) {
        (Value::Null, Json::Null) => true,
        (Value::Bool(a), Json::Bool(b)) => a == b,
        // Both sides parsed the same lexeme, so they either produced the same
        // double or the parser has a bug; an epsilon would hide exactly that.
        #[allow(clippy::float_cmp)]
        (Value::Number(_), Json::Number(b)) => ours
            .as_f64()
            .is_some_and(|a| b.as_f64().is_some_and(|b| a == b)),
        (Value::String(a), Json::String(b)) => a == b,
        (Value::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
        }
        (Value::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|other| same(v, other)))
        }
        _ => false,
    }
}

#[test]
fn agrees_with_serde_json_on_strict_documents() {
    for src in STRICT {
        let ours = xcproj::parse(src).unwrap_or_else(|e| panic!("xcproj rejected {src}: {e}"));
        let theirs: Json =
            serde_json::from_str(src).unwrap_or_else(|e| panic!("serde_json rejected {src}: {e}"));
        assert!(same(&ours, &theirs), "disagreed on {src}");
    }
}

#[test]
fn rejects_what_serde_json_rejects() {
    for src in [
        r#"{"a": }"#,
        r#"{"a" 1}"#,
        r"{a: 1}",
        r#"{"a": 1"#,
        r"[",
        r#"{"a": 'b'}"#,
        r#"{"a": 1} trailing"#,
        r"",
    ] {
        assert!(xcproj::parse(src).is_err(), "xcproj accepted {src:?}");
        assert!(
            serde_json::from_str::<Json>(src).is_err(),
            "serde_json accepted {src:?}"
        );
    }
}

/// The reason this parser exists: Xcode's own output is not JSON.
#[test]
fn the_canonical_printing_is_not_json() {
    assert!(
        serde_json::from_str::<Json>(CANONICAL).is_err(),
        "serde_json read the canonical printing, so the trailing commas are gone"
    );

    let doc = xcproj::parse(CANONICAL).expect("xcproj parse");
    let configs = doc.get("configurations").and_then(Value::as_array).unwrap();
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].as_str(), Some("Debug"));
    assert_eq!(
        doc.get("localizations")
            .and_then(|l| l.get("supported"))
            .and_then(Value::as_array)
            .unwrap()
            .len(),
        2
    );
}

/// Comments are trivia to this parser and are absent from anything Xcode
/// writes, so a document keeps its meaning when they are stripped.
#[test]
fn comments_do_not_change_what_a_document_says() {
    let commented = r#"// a project
        {
          "a": 1, /* between */
          "b": [ 2, 3 ] // after
        }"#;
    let plain = r#"{"a": 1, "b": [2, 3]}"#;

    let with = xcproj::parse(commented).unwrap();
    let without: Json = serde_json::from_str(plain).unwrap();
    assert!(same(&with, &without));
}
