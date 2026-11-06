use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::cli::{
    keyed_archive::{self, Plist},
    xml_plist,
};

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub(super) enum Format {
    #[default]
    Xml,
    Binary,
    Human,
}

impl Format {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "xml1" => Ok(Self::Xml),
            "binary1" => Ok(Self::Binary),
            "human-readable-text" => Ok(Self::Human),
            _ => Err(format!("unknown output format {value}")),
        }
    }
}

pub(super) fn requested_format(args: &[String]) -> Format {
    args.iter()
        .enumerate()
        .find_map(|(index, arg)| {
            if arg == "--output-format" {
                args.get(index + 1)
                    .and_then(|value| Format::parse(value).ok())
            } else {
                arg.strip_prefix("--output-format=")
                    .and_then(|value| Format::parse(value).ok())
            }
        })
        .unwrap_or_default()
}

fn plist(value: &Value) -> Plist {
    match value {
        Value::Object(values) => Plist::Dict(
            values
                .iter()
                .map(|(key, value)| (key.clone(), plist(value)))
                .collect(),
        ),
        Value::Array(values) => Plist::Array(values.iter().map(plist).collect()),
        Value::Bool(value) => Plist::Bool(*value),
        Value::Number(value) => value
            .as_i64()
            .map_or_else(|| Plist::Real(value.as_f64().unwrap_or(0.0)), Plist::Int),
        Value::String(value) => Plist::String(value.clone()),
        Value::Null => Plist::String(String::new()),
    }
}

pub(super) fn encode(format: Format, data: &Value) -> Vec<u8> {
    match format {
        Format::Binary => keyed_archive::write(&plist(data)),
        Format::Xml => xml_plist::typed_of(data).into_bytes(),
        Format::Human => {
            let mut output = String::new();
            if let Some(errors) = data
                .get("com.apple.ibtool.errors")
                .and_then(Value::as_array)
            {
                output.push_str("/* com.apple.ibtool.errors */\n");
                for error in errors {
                    if let Some(file) = error.get("file").and_then(Value::as_str) {
                        output.push_str(file);
                        output.push_str(": ");
                    }
                    output.push_str("error: ");
                    output.push_str(error["description"].as_str().unwrap_or("unknown error"));
                    output.push('\n');
                }
            }
            for key in data.as_object().into_iter().flat_map(|map| map.keys()) {
                if key == "com.apple.ibtool.errors"
                    || data[key].as_object().is_some_and(Map::is_empty)
                {
                    continue;
                }
                let _ = write!(
                    output,
                    "/* {key} */\nibtool does not support human readable output for this option.\n"
                );
            }
            output.into_bytes()
        }
    }
}

pub(super) fn emit(format: Format, data: &Value) -> Result<(), String> {
    std::io::stdout()
        .lock()
        .write_all(&encode(format, data))
        .map_err(|error| error.to_string())
}

pub(super) fn error_value(input: Option<&Path>, message: &str) -> Value {
    let mut value = json!({"description":message});
    if let Some(input) = input {
        value["file"] = json!(super::absolute(input));
    }
    value
}

pub(super) fn error(format: Format, input: Option<&Path>, message: &str) {
    let _ = emit(
        format,
        &json!({"com.apple.ibtool.errors":[error_value(input,message)]}),
    );
}

pub(super) fn version_data() -> Value {
    json!({"com.apple.ibtool.version":{"bundle-version":"25098","short-bundle-version":"27.0"}})
}

pub(super) fn version(format: Format) -> Result<(), String> {
    emit(format, &version_data())
}

pub(super) fn success(
    format: Format,
    categories: &[String],
    mut metadata: Map<String, Value>,
) -> Result<(), String> {
    for category in categories {
        metadata.insert(format!("com.apple.ibtool.document.{category}"), json!({}));
    }
    if metadata.is_empty() {
        return Ok(());
    }
    emit(format, &Value::Object(metadata))
}
