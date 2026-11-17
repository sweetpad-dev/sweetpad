//! SSUv1 shortcut models. Native captures verify the decoded phrase content.

use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::path::Path;

type Table = WIPOffset<TableFinishedWIPOffset>;

#[derive(Clone, Deserialize)]
pub(super) struct Expansions {
    pub prefixes: Vec<String>,
    pub negatives: Vec<String>,
}

impl Expansions {
    pub fn english() -> Result<Self, String> {
        Ok(Self {
            prefixes: vec![String::new(), "hey Siri ".into(), "Siri ".into()],
            negatives: serde_json::from_str(include_str!(
                "../../../fixtures/appintents/nlu-negatives.json"
            ))
            .map_err(|e| e.to_string())?,
        })
    }
}

#[derive(Serialize)]
struct Intent {
    identifier: String,
    title: String,
    utterances: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct LanguageModel {
    pub locale: String,
    intents: Vec<Intent>,
    negatives: Vec<String>,
}

impl LanguageModel {
    pub fn new(
        locale: &str,
        app: &str,
        metadata: &Value,
        expansions: &Expansions,
    ) -> Result<Self, String> {
        let mut intents = Vec::new();
        for (index, shortcut) in metadata["autoShortcuts"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let action = shortcut["actionIdentifier"]
                .as_str()
                .ok_or("shortcut has no action identifier")?;
            let mut utterances = Vec::new();
            for phrase in shortcut["phraseTemplates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p["key"].as_str())
            {
                for prefix in &expansions.prefixes {
                    utterances.push(format!(
                        "{prefix}{}",
                        phrase.replace("${applicationName}", app)
                    ));
                }
            }
            intents.push(Intent {
                identifier: format!("{action}_{index}"),
                title: metadata["actions"][action]["title"]["key"]
                    .as_str()
                    .or_else(|| shortcut["shortTitle"]["key"].as_str())
                    .unwrap_or(action)
                    .into(),
                utterances,
            });
        }
        Ok(Self {
            locale: locale.into(),
            intents,
            negatives: expansions
                .negatives
                .iter()
                .map(|p| p.replace("${applicationName}", app))
                .collect(),
        })
    }
}

pub(super) fn write(
    directory: &Path,
    bundle: &str,
    language: &LanguageModel,
) -> Result<(), String> {
    let (model, fingerprint) = model(bundle, language)?;
    let mut frame = b"bvx-".to_vec();
    frame.extend(
        u32::try_from(model.len())
            .map_err(|e| e.to_string())?
            .to_le_bytes(),
    );
    frame.extend(model);
    frame.extend(b"bvx$");
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "version")
            && path.file_stem().is_none_or(|s| s != fingerprint.as_str())
        {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    write_if_changed(&directory.join("nlu.lzfse"), &frame)?;
    write_if_changed(&directory.join(format!("{fingerprint}.version")), &[0])
}

pub(super) fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|previous| previous == bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
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

fn model(bundle: &str, language: &LanguageModel) -> Result<(Vec<u8>, String), String> {
    // This is a producer fingerprint, not a schema identifier. It must change
    // when phrases change, and must agree with the adjacent .version filename.
    let content = serde_json::to_vec(&(bundle, language)).map_err(|e| e.to_string())?;
    let mut fingerprint = String::with_capacity(32);
    for byte in &sweetpad_lib::xcode_hash::sha256(&content)[..16] {
        let _ = write!(fingerprint, "{byte:02x}");
    }
    let mut builder = FlatBufferBuilder::new();
    let mut intents = Vec::new();
    for intent in &language.intents {
        let utterances = intent
            .utterances
            .iter()
            .map(|p| utterance(&mut builder, p))
            .collect::<Vec<_>>();
        let utterances = builder.create_vector(&utterances);
        let id = builder.create_string(&intent.identifier);
        let empty = builder.create_string("");
        let start = builder.start_table();
        builder.push_slot_always(4, id);
        builder.push_slot_always(6, empty);
        builder.push_slot_always(8, utterances);
        intents.push(builder.end_table(start));
    }
    let negatives = language
        .negatives
        .iter()
        .map(|p| utterance(&mut builder, p))
        .collect::<Vec<_>>();
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
    let locale = builder.create_string(&language.locale);
    let fingerprint_string = builder.create_string(&fingerprint);
    let empty = builder.create_string("");
    let version = builder.create_string("1.0");
    let start = builder.start_table();
    builder.push_slot_always(4, fingerprint_string);
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
    Ok((builder.finished_data().to_vec(), fingerprint))
}

pub(super) fn ssu(bundle: &str, languages: &[LanguageModel]) -> Result<String, String> {
    let mut intents = Vec::new();
    for (index, intent) in languages
        .first()
        .into_iter()
        .flat_map(|l| &l.intents)
        .enumerate()
    {
        let training: Vec<_> = languages
            .iter()
            .map(|l| json!({"locale": l.locale, "utterances": l.intents[index].utterances}))
            .collect();
        intents.push(json!({"metadata": {"name": intent.identifier, "title": intent.title}, "corpuses": {"training": training}}));
    }
    let training: Vec<_> = languages
        .iter()
        .map(|l| json!({"locale": l.locale, "utterances": l.negatives}))
        .collect();
    intents.push(json!({"metadata": {"name": "+negative"}, "corpuses": {"training": training}}));
    // JSON is valid YAML and preserves literal localized strings without
    // interpreting shortcut parameter syntax as YAML substitutions.
    serde_json::to_string_pretty(&json!({"apiVersion": "ssu/v1", "nlu": {"shortcuts": {"bundleIdentifier": bundle, "intents": intents}}})).map_err(|e| e.to_string())
}

/// What a shortcut model holds: its locale and app, each intent's phrases
/// and the negative examples, phrases sorted.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Content {
    pub locale: String,
    pub bundle: String,
    pub intents: std::collections::BTreeMap<String, Vec<String>>,
    pub negatives: Vec<String>,
}

/// Read a model as Xcode's training processor writes it (`nlu.lzfse`):
/// LZFSE around an SSUv1 FlatBuffer.
pub(crate) fn read(packed: &[u8]) -> Result<Content, String> {
    let mut data = Vec::new();
    lzfse_rust::decode_bytes(packed, &mut data)
        .map_err(|e| format!("invalid shortcut model: {e}"))?;
    content(&data)
}

/// The content of an SSUv1 FlatBuffer, checking every offset.
pub(crate) fn content(data: &[u8]) -> Result<Content, String> {
    let bad = || "invalid shortcut model offset".to_string();
    let u32_at = |at: usize| -> Result<usize, String> {
        let bytes = data.get(at..at + 4).ok_or_else(bad)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
    };
    let u16_at = |at: usize| -> Result<usize, String> {
        let bytes = data.get(at..at + 2).ok_or_else(bad)?;
        Ok(usize::from(u16::from_le_bytes(bytes.try_into().unwrap())))
    };
    // Where field `index` of the table at `at` is, if it's set.
    let field = |at: usize, index: usize| -> Result<Option<usize>, String> {
        // A table starts with a signed offset back to its vtable.
        let bytes = data.get(at..at + 4).ok_or_else(bad)?;
        let back = i64::from(i32::from_le_bytes(bytes.try_into().unwrap()));
        let at_signed = i64::try_from(at).map_err(|_| bad())?;
        let table = usize::try_from(at_signed - back).map_err(|_| bad())?;
        if 4 + index * 2 >= u16_at(table)? {
            return Ok(None);
        }
        let offset = u16_at(table + 4 + index * 2)?;
        Ok((offset != 0).then_some(at + offset))
    };
    let reference = |at: usize, index: usize| -> Result<usize, String> {
        let p = field(at, index)?.ok_or_else(bad)?;
        Ok(p + u32_at(p)?)
    };
    let vector = |at: usize| -> Result<Vec<usize>, String> {
        let count = u32_at(at)?;
        if at + 4 + 4 * count > data.len() {
            return Err("invalid shortcut model vector".into());
        }
        (0..count)
            .map(|i| {
                let p = at + 4 + 4 * i;
                Ok(p + u32_at(p)?)
            })
            .collect()
    };
    let string = |at: usize| -> Result<String, String> {
        let count = u32_at(at)?;
        let bytes = data
            .get(at + 4..at + 4 + count)
            .ok_or("invalid shortcut model string")?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "invalid shortcut model string".into())
    };
    let phrases = |at: usize| -> Result<Vec<String>, String> {
        let mut phrases = vector(at)?
            .into_iter()
            .map(|t| string(reference(reference(t, 1)?, 0)?))
            .collect::<Result<Vec<_>, _>>()?;
        phrases.sort();
        Ok(phrases)
    };
    let root = u32_at(0)?;
    if u16_at(field(root, 0)?.ok_or_else(bad)?)? != 1 {
        return Err("unsupported shortcut model schema".into());
    }
    let [group] = vector(reference(root, 3)?)?[..] else {
        return Err("a shortcut model must have one group".into());
    };
    let [app] = vector(reference(group, 1)?)?[..] else {
        return Err("a shortcut model must have one app".into());
    };
    let mut intents = std::collections::BTreeMap::new();
    for intent in vector(reference(app, 1)?)? {
        let name = string(reference(intent, 0)?)?;
        if intents
            .insert(name, phrases(reference(intent, 2)?)?)
            .is_some()
        {
            return Err("duplicate shortcut model intent".into());
        }
    }
    Ok(Content {
        locale: string(reference(root, 2)?)?,
        bundle: string(reference(app, 0)?)?,
        intents,
        negatives: phrases(reference(app, 2)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_language_model_matches_xcodes_phrases_and_negative_examples() {
        let metadata: Value = serde_json::from_str(include_str!(
            "../../../fixtures/appintents/extract.actionsdata"
        ))
        .unwrap();
        let language =
            LanguageModel::new("en", "DataApp", &metadata, &Expansions::english().unwrap())
                .unwrap();
        let (actual, _) = model("dev.sweetpad.DataApp", &language).unwrap();
        let expected = include_bytes!("../../../fixtures/appintents/DataApp.nlu");
        assert_eq!(content(&actual).unwrap(), content(expected).unwrap());
    }
}
