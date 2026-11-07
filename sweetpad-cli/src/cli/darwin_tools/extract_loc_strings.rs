//! Literal NSLocalizedString-family extraction used by localization build scripts.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Default)]
struct Entry {
    value: String,
    comments: BTreeSet<String>,
}
type Tables = BTreeMap<String, BTreeMap<String, Entry>>;

pub(super) fn main(args: &[String]) -> Result<(), String> {
    let mut output = PathBuf::from(".");
    let mut name = "NSLocalizedString".to_string();
    let mut files = Vec::new();
    let mut positional = true;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => output = super::value(arg, &mut args)?.into(),
            "-s" => name.clone_from(super::value(arg, &mut args)?),
            "-u" | "-q" | "-littleEndian" => {}
            "-noPositionalParameters" => positional = false,
            "-encoding" => {
                let encoding = super::value(arg, &mut args)?;
                if !encoding.eq_ignore_ascii_case("utf-8") {
                    return Err(format!("unsupported input encoding '{encoding}'"));
                }
            }
            flag if flag.starts_with('-') => return Err(format!("unsupported option '{flag}'")),
            _ => files.push(PathBuf::from(arg)),
        }
    }
    if files.is_empty() {
        return Err("name at least one source file".into());
    }
    let mut tables = Tables::new();
    for file in files {
        let source =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        extract(&source, &name, &mut tables, positional);
    }
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    for (table, entries) in tables {
        if table.contains(['/', '\\']) || table == "." || table == ".." {
            return Err(format!("invalid strings table '{table}'"));
        }
        let text = render(entries);
        let bytes: Vec<u8> = std::iter::once(0xfeff_u16)
            .chain(text.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(output.join(format!("{table}.strings")), bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn extract(source: &str, name: &str, tables: &mut Tables, positional: bool) {
    for (offset, _) in source.match_indices(name) {
        if offset > 0
            && source[..offset]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let rest = &source[offset + name.len()..];
        let suffix = ["FromTableInBundle", "FromTable", "WithDefaultValue", ""]
            .into_iter()
            .find(|s| rest.starts_with(s))
            .unwrap();
        let Some(rest) = rest[suffix.len()..].trim_start().strip_prefix('(') else {
            continue;
        };
        let Some(args) = arguments(rest) else {
            continue;
        };
        let Some((key, table, mut value, comment)) = entry(&args, suffix) else {
            continue;
        };
        if positional && super::super::xcstringstool::format::tokens(&value).len() > 1 {
            value = super::super::xcstringstool::format::positioned(&value);
        }
        let item = tables.entry(table).or_default().entry(key).or_default();
        item.value = value;
        if !comment.is_empty() {
            item.comments.insert(comment);
        }
    }
}

fn entry(args: &[&str], suffix: &str) -> Option<(String, String, String, String)> {
    let key = literal(args.first()?)?;
    let mut table = "Localizable".to_string();
    let mut value = key.clone();
    let mut comment = String::new();
    if args
        .iter()
        .skip(1)
        .any(|a| a.trim_start().starts_with("comment:"))
    {
        for arg in &args[1..] {
            let (label, expression) = arg.split_once(':')?;
            match label.trim() {
                "tableName" if expression.trim() != "nil" => table = literal(expression)?,
                "value" => value = literal(expression)?,
                "comment" => comment = literal(expression)?,
                _ => {}
            }
        }
    } else {
        let (table_index, value_index, comment_index, count) = match suffix {
            "FromTable" => (Some(1), None, 2, 3),
            "FromTableInBundle" => (Some(1), None, 3, 4),
            "WithDefaultValue" => (Some(1), Some(3), 4, 5),
            "" => (None, None, 1, 2),
            _ => return None,
        };
        if args.len() != count {
            return None;
        }
        if let Some(i) = table_index
            && args[i].trim() != "nil"
        {
            table = literal(args[i])?;
        }
        if let Some(i) = value_index {
            value = literal(args[i])?;
        }
        comment = literal(args[comment_index]).unwrap_or_default();
    }
    Some((key, table, value, comment))
}

/// Split call arguments without treating commas in strings or nested calls as separators.
fn arguments(source: &str) -> Option<Vec<&str>> {
    let bytes = source.as_bytes();
    let (mut i, mut start, mut depth) = (0, 0, 0);
    let mut result = Vec::new();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i = string_end(source, i)?.1;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' if depth == 0 => {
                result.push(&source[start..i]);
                return Some(result);
            }
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                result.push(&source[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn string_end(source: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = source.as_bytes();
    let triple = source[start..].starts_with("\"\"\"");
    let width = if triple { 3 } else { 1 };
    let mut i = start + width;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' && (!triple || source[i..].starts_with("\"\"\"")) {
            return Some((&source[start + width..i], i + width));
        }
        i += 1;
    }
    None
}

fn literal(expression: &str) -> Option<String> {
    let expression = expression
        .trim()
        .strip_prefix('@')
        .unwrap_or(expression.trim());
    if !expression.starts_with('"') {
        return None;
    }
    let (content, end) = string_end(expression, 0)?;
    if !expression[end..].trim().is_empty() {
        return None;
    }
    let mut chars = content.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '"' => '"',
            '\\' => '\\',
            '\'' => '\'',
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            _ => return None,
        });
    }
    Some(out)
}

fn render(entries: BTreeMap<String, Entry>) -> String {
    let mut out = String::new();
    for (key, entry) in entries {
        let comment = if entry.comments.is_empty() {
            "No comment provided by engineer.".into()
        } else {
            entry.comments.into_iter().collect::<Vec<_>>().join("\n   ")
        };
        let _ = writeln!(
            out,
            "/* {comment} */\n{} = {};\n",
            quote(&key),
            quote(&entry.value)
        );
    }
    out
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_native_literal_extraction() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/extractlocstrings");
        let mut tables = Tables::new();
        for file in ["Strings.swift", "Strings.m"] {
            extract(
                &std::fs::read_to_string(root.join(file)).unwrap(),
                "WMFLocalizedString",
                &mut tables,
                true,
            );
        }
        for (name, entries) in tables {
            let got: Vec<u8> = std::iter::once(0xfeff_u16)
                .chain(render(entries).encode_utf16())
                .flat_map(u16::to_le_bytes)
                .collect();
            assert_eq!(
                got,
                std::fs::read(root.join("expected").join(format!("{name}.strings"))).unwrap(),
                "{name}"
            );
        }
    }
}
