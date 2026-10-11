//! XML property lists, written the way Foundation writes them: tab-indented,
//! each dictionary's keys in UTF-16 order, and only `&`, `<` and `>` escaped.
//! The stand-ins for Xcode's tools write theirs with it, since what they
//! write is compared with what Xcode's tools write.

use std::fmt::Write as _;

use serde_json::Value;

/// `body`, the top-level object's XML, as a whole document.
#[must_use]
pub fn document(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n{body}\n</plist>\n"
    )
}

/// A JSON value as a property list document: objects become dictionaries,
/// arrays arrays, and everything else a string.
#[must_use]
pub fn of(value: &Value) -> String {
    let mut body = String::new();
    render(value, 0, &mut body);
    document(body.trim_end())
}

fn render(value: &Value, indent: usize, out: &mut String) {
    let pad = "\t".repeat(indent);
    match value {
        Value::Object(map) if map.is_empty() => {
            let _ = writeln!(out, "{pad}<dict/>");
        }
        Value::Object(map) => {
            let _ = writeln!(out, "{pad}<dict>");
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            for key in keys {
                let _ = writeln!(out, "{pad}\t<key>{}</key>", escape(key));
                render(&map[key], indent + 1, out);
            }
            let _ = writeln!(out, "{pad}</dict>");
        }
        Value::Array(items) if items.is_empty() => {
            let _ = writeln!(out, "{pad}<array/>");
        }
        Value::Array(items) => {
            let _ = writeln!(out, "{pad}<array>");
            for item in items {
                render(item, indent + 1, out);
            }
            let _ = writeln!(out, "{pad}</array>");
        }
        other => {
            let text = other
                .as_str()
                .map_or_else(|| other.to_string(), str::to_string);
            let _ = writeln!(out, "{pad}<string>{}</string>", escape(&text));
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_sort_by_utf16_like_foundations() {
        // U+1F600 is a surrogate pair in UTF-16, which sorts below U+E000.
        let text = of(&json!({ "\u{e000}": "a", "😀": "b", "Z": "c", "a": "<&>" }));
        let keys: Vec<&str> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("<key>")?.strip_suffix("</key>"))
            .collect();
        assert_eq!(keys, ["Z", "a", "😀", "\u{e000}"]);
        assert!(text.contains("<string>&lt;&amp;&gt;</string>"));
    }
}
