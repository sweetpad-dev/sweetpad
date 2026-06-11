//! Dev tool: re-serialize every fixture pbxproj and show where the output
//! diverges from the raw bytes. `cargo run --example roundtrip_diff [filter]`

use std::path::{Path, PathBuf};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let mut exact = 0usize;
    let mut total = 0usize;
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap().display().to_string();
        if !filter.is_empty() && !rel.contains(&filter) {
            continue;
        }
        total += 1;
        let raw = std::fs::read_to_string(f).unwrap();
        let ser = if f.extension().and_then(|s| s.to_str()) == Some("pbxproj") {
            let parsed = match sweetpad::pbxproj::parse(&raw) {
                Ok(v) => v,
                Err(e) => {
                    println!("PARSE FAIL {rel}: {e}");
                    continue;
                }
            };
            sweetpad::pbxproj_writer::serialize(&parsed, &project_name(f))
        } else if f.extension().and_then(|s| s.to_str()) == Some("xcconfig") {
            let parsed = match sweetpad::xcconfig::parse(&raw) {
                Ok(v) => v,
                Err(e) => {
                    println!("PARSE FAIL {rel}: {e}");
                    continue;
                }
            };
            sweetpad::xcconfig::serialize(&parsed)
        } else {
            let parsed = match sweetpad::xcscheme::parse(&raw) {
                Ok(v) => v,
                Err(e) => {
                    println!("PARSE FAIL {rel}: {e}");
                    continue;
                }
            };
            sweetpad::xcscheme::serialize(&parsed)
        };
        if ser == raw {
            exact += 1;
            continue;
        }
        let (line_no, raw_line, ser_line) = first_diff(&raw, &ser);
        println!("DIFF {rel} at line {line_no}");
        println!("  raw: {raw_line:?}");
        println!("  ser: {ser_line:?}");
    }
    println!("\n{exact}/{total} byte-exact");
}

fn project_name(p: &Path) -> String {
    p.parent()
        .and_then(|d| d.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn first_diff(raw: &str, serialized: &str) -> (usize, String, String) {
    let mut raw_lines = raw.lines();
    let mut ser_lines = serialized.lines();
    let mut line_no = 0;
    loop {
        line_no += 1;
        match (raw_lines.next(), ser_lines.next()) {
            (Some(raw_line), Some(ser_line)) if raw_line == ser_line => {}
            (raw_line, ser_line) => {
                return (
                    line_no,
                    raw_line.unwrap_or("<eof>").to_string(),
                    ser_line.unwrap_or("<eof>").to_string(),
                );
            }
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("pbxproj" | "xcscheme" | "xcworkspacedata" | "xcconfig")
        ) {
            out.push(p);
        }
    }
}
