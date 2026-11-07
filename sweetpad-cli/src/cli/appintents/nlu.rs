//! The SSU v1 FlatBuffer used for App Shortcut phrase registration. The
//! table layout and phrase expansion are checked against Xcode 27 captures.

use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};
use serde_json::Value;
use std::fmt::Write as _;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_ID: &str = "0591b843d1bfa8e989c3a884b4199791";
type Table = WIPOffset<TableFinishedWIPOffset>;

pub(super) fn write(
    directory: &Path,
    bundle: &str,
    app: &str,
    metadata: &Value,
) -> Result<(), String> {
    let model = model(bundle, app, metadata)?;
    let mut frame = b"bvx-".to_vec();
    frame.extend(
        u32::try_from(model.len())
            .map_err(|e| e.to_string())?
            .to_le_bytes(),
    );
    frame.extend(model);
    frame.extend(b"bvx$");
    let nlu = directory.join("nlu");
    std::fs::create_dir_all(&nlu).map_err(|e| e.to_string())?;
    std::fs::write(nlu.join("nlu.lzfse"), frame).map_err(|e| e.to_string())?;
    std::fs::write(nlu.join(format!("{SCHEMA_ID}.version")), [0]).map_err(|e| e.to_string())?;
    std::fs::write(directory.join("root.ssu.yaml"), ssu(bundle, app, metadata)?)
        .map_err(|e| e.to_string())
}

fn phrases(app: &str, shortcut: &Value) -> Vec<String> {
    shortcut["phraseTemplates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["key"].as_str())
        .map(|p| p.replace("${applicationName}", app))
        .collect()
}

fn utterance(builder: &mut FlatBufferBuilder<'_>, text: &str) -> Table {
    let text = builder.create_string(text);
    let start = builder.start_table();
    builder.push_slot_always(4, text);
    let payload = builder.end_table(start);
    let start = builder.start_table();
    builder.push_slot(4, true, false);
    builder.push_slot_always(6, payload);
    builder.end_table(start)
}

fn model(bundle: &str, app: &str, metadata: &Value) -> Result<Vec<u8>, String> {
    let mut builder = FlatBufferBuilder::new();
    let mut intents = Vec::new();
    for (index, shortcut) in metadata["autoShortcuts"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let mut utterances = Vec::new();
        for phrase in phrases(app, shortcut) {
            for prefix in ["", "hey Siri ", "Siri "] {
                utterances.push(utterance(&mut builder, &format!("{prefix}{phrase}")));
            }
        }
        let utterances = builder.create_vector(&utterances);
        let id = builder.create_string(&format!(
            "{}_{index}",
            shortcut["actionIdentifier"]
                .as_str()
                .ok_or("shortcut has no action identifier")?
        ));
        let empty = builder.create_string("");
        let start = builder.start_table();
        builder.push_slot_always(4, id);
        builder.push_slot_always(6, empty);
        builder.push_slot_always(8, utterances);
        intents.push(builder.end_table(start));
    }
    let templates: Vec<String> = serde_json::from_str(include_str!(
        "../../../fixtures/appintents/nlu-negatives.json"
    ))
    .map_err(|e| e.to_string())?;
    let negatives: Vec<Table> = templates
        .iter()
        .map(|t| utterance(&mut builder, &t.replace("${applicationName}", app)))
        .collect();
    let negatives = builder.create_vector(&negatives);
    let intents = builder.create_vector(&intents);
    let bundle = builder.create_string(bundle);
    let start = builder.start_table();
    builder.push_slot_always(4, bundle);
    builder.push_slot_always(6, intents);
    builder.push_slot_always(8, negatives);
    let application = builder.end_table(start);
    let applications = builder.create_vector(&[application]);
    let start = builder.start_table();
    builder.push_slot_always(6, applications);
    let group = builder.end_table(start);
    let groups = builder.create_vector(&[group]);
    let locale = builder.create_string("en");
    let schema = builder.create_string(SCHEMA_ID);
    let empty = builder.create_string("");
    let version = builder.create_string("1.0");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let start = builder.start_table();
    builder.push_slot_always(4, schema);
    builder.push_slot(6, now, 0);
    builder.push_slot_always(8, empty);
    builder.push_slot_always(10, version);
    let version = builder.end_table(start);
    let start = builder.start_table();
    builder.push_slot(4, 1_u16, 0);
    builder.push_slot_always(6, version);
    builder.push_slot_always(8, locale);
    builder.push_slot_always(10, groups);
    let root = builder.end_table(start);
    builder.finish(root, None);
    Ok(builder.finished_data().to_vec())
}

fn ssu(bundle: &str, app: &str, metadata: &Value) -> Result<String, String> {
    let quoted = |s: &str| serde_json::to_string(s).map_err(|e| e.to_string());
    let mut out = format!(
        "apiVersion: ssu/v1\nnlu:\n  shortcuts:\n    bundleIdentifier: {}\n    intents:\n",
        quoted(bundle)?
    );
    for (index, shortcut) in metadata["autoShortcuts"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let id = shortcut["actionIdentifier"]
            .as_str()
            .ok_or("shortcut has no action identifier")?;
        let _ = write!(
            out,
            "    - metadata:\n        name: {}\n        title: {}\n      corpuses:\n        training:\n        - locale: en\n          utterances:\n",
            quoted(&format!("{id}_{index}"))?,
            quoted(shortcut["shortTitle"]["key"].as_str().unwrap_or(id))?
        );
        for phrase in shortcut["phraseTemplates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["key"].as_str())
        {
            let _ = writeln!(
                out,
                "          - {}",
                quoted(&format!(
                    "{{${{+prefix}} }}{}",
                    phrase.replace("${applicationName}", "${+applicationName}")
                ))?
            );
        }
    }
    out.push_str("    - metadata:\n        name: +negative\n      corpuses:\n        training:\n        - locale: en\n          utterances:\n          - '{${+prefix} }${+standardAppShortcutNegatives}'\nvariables:\n- name: +applicationName\n  type: ssu/expansion\n  definitions:\n  - locale: en\n    synomyms:\n");
    let _ = writeln!(out, "    - {}", quoted(app)?);
    out.push_str("- name: +prefix\n  type: ssu/expansion\n  definitions:\n  - locale: en\n    synomyms:\n      $ref: ssu://Resources/prefix/en.yaml\n- name: +standardAppShortcutNegatives\n  type: ssu/expansion\n  definitions:\n  - locale: en\n    synomyms:\n      $ref: ssu://Resources/standardAppShortcutNegatives/en.yaml\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn u32_at(data: &[u8], at: usize) -> usize {
        u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
    }
    fn field(data: &[u8], at: usize, index: usize) -> usize {
        let back = i32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        let table = usize::try_from(i64::try_from(at).unwrap() - i64::from(back)).unwrap();
        let offset = usize::from(u16::from_le_bytes(
            data[table + 4 + index * 2..table + 6 + index * 2]
                .try_into()
                .unwrap(),
        ));
        let p = at + offset;
        p + u32_at(data, p)
    }
    fn vector(data: &[u8], at: usize) -> Vec<usize> {
        (0..u32_at(data, at))
            .map(|i| {
                let p = at + 4 + i * 4;
                p + u32_at(data, p)
            })
            .collect()
    }
    fn string(data: &[u8], at: usize) -> String {
        String::from_utf8(data[at + 4..at + 4 + u32_at(data, at)].to_vec()).unwrap()
    }
    fn content(data: &[u8]) -> (String, String, BTreeMap<String, Vec<String>>, Vec<String>) {
        let root = u32_at(data, 0);
        let group = vector(data, field(data, root, 3))[0];
        let app = vector(data, field(data, group, 1))[0];
        let phrases = |at| {
            let mut p: Vec<String> = vector(data, at)
                .into_iter()
                .map(|t| string(data, field(data, field(data, t, 1), 0)))
                .collect();
            p.sort();
            p
        };
        let intents = vector(data, field(data, app, 1))
            .into_iter()
            .map(|t| (string(data, field(data, t, 0)), phrases(field(data, t, 2))))
            .collect();
        (
            string(data, field(data, root, 2)),
            string(data, field(data, app, 0)),
            intents,
            phrases(field(data, app, 2)),
        )
    }
    #[test]
    fn shortcut_language_model_matches_xcodes_phrases_and_negative_examples() {
        let metadata: Value = serde_json::from_str(include_str!(
            "../../../fixtures/appintents/extract.actionsdata"
        ))
        .unwrap();
        let actual = model("dev.sweetpad.DataApp", "DataApp", &metadata).unwrap();
        let expected = include_bytes!("../../../fixtures/appintents/DataApp.nlu");
        assert_eq!(content(&actual), content(expected));
    }
}
