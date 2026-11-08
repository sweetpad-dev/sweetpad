//! Portable property-list reading and XML writing, laid out the
//! way Core Foundation writes them, which matters where a signature hashes
//! the file: `CodeResources` and the CDHashes list in the CMS signature.
//! Dates read as their text.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sweetpad_lib::xcscheme::{self, Element};

pub(super) use crate::cli::keyed_archive::Plist;

/// A property list in any of Apple's formats: XML here, binary through the
/// keyed archive reader's sibling in `sweetpad_lib`.
pub(super) fn parse(data: &[u8]) -> Result<Plist, String> {
    if data.starts_with(b"bplist00") {
        return Err("a binary property list where XML was expected".to_string());
    }
    let text = std::str::from_utf8(data).map_err(|_| "a property list that isn't UTF-8")?;
    let root = xcscheme::parse(text).map_err(|e| format!("couldn't parse a property list: {e}"))?;
    if root.name != "plist" {
        return Err(format!("expected <plist>, found <{}>", root.name));
    }
    let value = root.children.first().ok_or("an empty property list")?;
    from_element(value)
}

fn from_element(e: &Element) -> Result<Plist, String> {
    Ok(match e.name.as_str() {
        "dict" => {
            let mut entries = Vec::new();
            let mut children = e.children.iter();
            while let Some(key) = children.next() {
                if key.name != "key" {
                    return Err(format!("expected <key> in a <dict>, found <{}>", key.name));
                }
                let value = children.next().ok_or("a <key> without its value")?;
                entries.push((key.text.clone(), from_element(value)?));
            }
            Plist::Dict(entries)
        }
        "array" => Plist::Array(
            e.children
                .iter()
                .map(from_element)
                .collect::<Result<_, _>>()?,
        ),
        "string" | "date" => Plist::String(e.text.clone()),
        "true" => Plist::Bool(true),
        "false" => Plist::Bool(false),
        "integer" => Plist::Int(
            e.text
                .trim()
                .parse()
                .map_err(|_| format!("a bad <integer>: {}", e.text))?,
        ),
        "real" => Plist::Real(
            e.text
                .trim()
                .parse()
                .map_err(|_| format!("a bad <real>: {}", e.text))?,
        ),
        "data" => {
            let text: String = e.text.chars().filter(|c| !c.is_whitespace()).collect();
            Plist::Data(
                STANDARD
                    .decode(text)
                    .map_err(|_| "bad base64 in a <data>")?,
            )
        }
        other => return Err(format!("unsupported property list element <{other}>")),
    })
}

/// `value` as an XML property list, as Core Foundation writes one.
pub(super) fn to_xml(value: &Plist) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n",
    );
    write_value(value, 0, &mut out);
    out.push_str("</plist>\n");
    out
}

fn write_value(value: &Plist, depth: usize, out: &mut String) {
    let pad = "\t".repeat(depth);
    match value {
        Plist::Dict(entries) => {
            if entries.is_empty() {
                let _ = writeln!(out, "{pad}<dict/>");
                return;
            }
            let _ = writeln!(out, "{pad}<dict>");
            let mut sorted: Vec<&(String, Plist)> = entries.iter().collect();
            sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            for (key, value) in sorted {
                let _ = writeln!(out, "{pad}\t<key>{}</key>", escape(key));
                write_value(value, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}</dict>");
        }
        Plist::Array(items) => {
            if items.is_empty() {
                let _ = writeln!(out, "{pad}<array/>");
                return;
            }
            let _ = writeln!(out, "{pad}<array>");
            for item in items {
                write_value(item, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}</array>");
        }
        Plist::String(text) => {
            let _ = writeln!(out, "{pad}<string>{}</string>", escape(text));
        }
        Plist::Bool(true) => {
            let _ = writeln!(out, "{pad}<true/>");
        }
        Plist::Bool(false) => {
            let _ = writeln!(out, "{pad}<false/>");
        }
        Plist::Int(n) => {
            let _ = writeln!(out, "{pad}<integer>{n}</integer>");
        }
        Plist::Uid(n) => {
            let _ = writeln!(out, "{pad}<integer>{n}</integer>");
        }
        Plist::Real(x) => {
            let _ = writeln!(out, "{pad}<real>{x}</real>");
        }
        Plist::Real32(x) => {
            let _ = writeln!(out, "{pad}<real>{x}</real>");
        }
        Plist::Data(bytes) => {
            let _ = writeln!(out, "{pad}<data>");
            let encoded = STANDARD.encode(bytes);
            for line in encoded.as_bytes().chunks(76) {
                let _ = writeln!(out, "{pad}{}", String::from_utf8_lossy(line));
            }
            let _ = writeln!(out, "{pad}</data>");
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The value at `key` in a dictionary.
pub(super) fn get<'a>(value: &'a Plist, key: &str) -> Option<&'a Plist> {
    match value {
        Plist::Dict(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

pub(super) fn string<'a>(value: &'a Plist, key: &str) -> Option<&'a str> {
    match get(value, key) {
        Some(Plist::String(text)) => Some(text),
        _ => None,
    }
}

/// `value` with `key` set to `new`, replacing what was there.
pub(super) fn set(value: &mut Plist, key: &str, new: Plist) {
    if let Plist::Dict(entries) = value {
        if let Some(entry) = entries.iter_mut().find(|(k, _)| k == key) {
            entry.1 = new;
        } else {
            entries.push((key.to_string(), new));
        }
    }
}

/// An `Info.plist`: XML, or the binary form Swift Build writes for iOS.
pub(super) fn read_info(data: &[u8]) -> Result<Plist, String> {
    if data.starts_with(b"bplist00") {
        let value = sweetpad_lib::bplist::parse(data).map_err(|e| e.to_string())?;
        return Ok(from_pbxproj(&value));
    }
    parse(data)
}

/// A binary property list's strings, dictionaries and arrays, which is what
/// a signature reads of an `Info.plist`.
fn from_pbxproj(value: &sweetpad_lib::pbxproj::Value) -> Plist {
    use sweetpad_lib::pbxproj::Value;
    match value {
        Value::String(text) => Plist::String(text.clone()),
        Value::Array(items) => Plist::Array(items.iter().map(from_pbxproj).collect()),
        Value::Dict(dict) => Plist::Dict(
            dict.iter()
                .map(|(k, v)| (k.clone(), from_pbxproj(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_like_core_foundation_and_reads_back() {
        let value = Plist::Dict(vec![
            ("b".to_string(), Plist::Bool(true)),
            (
                "a".to_string(),
                Plist::Dict(vec![
                    ("hash2".to_string(), Plist::Data(vec![1, 2, 3])),
                    ("weight".to_string(), Plist::Real(1000.0)),
                ]),
            ),
        ]);
        let xml = to_xml(&value);
        assert!(xml.contains(
            "<dict>\n\t<key>a</key>\n\t<dict>\n\t\t<key>hash2</key>\n\t\t<data>\n\t\tAQID\n\t\t</data>\n\t\t<key>weight</key>\n\t\t<real>1000</real>\n\t</dict>\n\t<key>b</key>\n\t<true/>\n</dict>\n"
        ));
        let back = parse(xml.as_bytes()).unwrap();
        assert_eq!(get(&back, "b"), Some(&Plist::Bool(true)));
    }
}
