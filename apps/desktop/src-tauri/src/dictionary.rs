use std::{collections::HashSet, fs, path::{Path, PathBuf}};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::trace::Span;

const DICTIONARY_VERSION: u32 = 1;
const DEFAULT_DICTIONARY_CONTEXT_CHARS: usize = 1800;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub id: String,
    pub source_term: String,
    pub preferred_term: String,
    pub note: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryFile {
    pub version: u32,
    pub entries: Vec<DictionaryEntry>,
    pub updated_at_ms: i64,
}

impl DictionaryFile {
    fn normalize(mut self) -> Self {
        self.version = self.version.max(DICTIONARY_VERSION);
        let mut merged: Vec<DictionaryEntry> = Vec::new();
        for e in self.entries {
            let n = normalize_entry(e);
            if let Some(n) = n {
                if let Some((i, _)) = merged
                    .iter()
                    .enumerate()
                    .find(|(_, old)| old.source_term.eq_ignore_ascii_case(&n.source_term))
                {
                    merged.remove(i);
                }
                merged.push(n);
            }
        }
        self.entries = merged;
        self.updated_at_ms = now_ms();
        self
    }
}

fn normalize_entry(mut e: DictionaryEntry) -> Option<DictionaryEntry> {
    let source = e.source_term.trim().to_string();
    let preferred = e.preferred_term.trim().to_string();
    if source.is_empty() || preferred.is_empty() {
        return None;
    }
    let id = e.id.trim().to_string();
    let id = if id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        id
    };
    let note = e.note.take().and_then(|v| {
        let t = v.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    });
    Some(DictionaryEntry {
        id,
        source_term: source,
        preferred_term: preferred,
        note,
        enabled: e.enabled,
    })
}

pub fn dictionary_path(data_dir: &Path) -> PathBuf {
    data_dir.join("dictionary.json")
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn default_dictionary() -> DictionaryFile {
    DictionaryFile {
        version: DICTIONARY_VERSION,
        entries: Vec::new(),
        updated_at_ms: now_ms(),
    }
}

pub fn dictionary_context_budget_chars() -> usize {
    DEFAULT_DICTIONARY_CONTEXT_CHARS
}

pub fn load_dictionary(data_dir: &Path) -> Result<DictionaryFile> {
    let p = dictionary_path(data_dir);
    if !p.exists() {
        return Ok(default_dictionary());
    }

    let s = fs::read_to_string(&p).context("read dictionary.json failed")?;
    let v = if s.trim_start().is_empty() {
        return Ok(default_dictionary());
    } else {
        serde_json::from_str::<Value>(&s).context("parse dictionary.json failed")?
    };

    let file: DictionaryFile = match v {
        Value::Array(_) => DictionaryFile {
            version: DICTIONARY_VERSION,
            entries: serde_json::from_value(v).context("parse dictionary array failed")?,
            updated_at_ms: now_ms(),
        },
        _ => serde_json::from_value(v).context("parse dictionary.json failed")?,
    };
    Ok(file.normalize())
}

pub fn save_dictionary(data_dir: &Path, mut file: DictionaryFile) -> Result<DictionaryFile> {
    let p = dictionary_path(data_dir);
    let span = Span::start(data_dir, None, "Dictionary", "DICT.save", None);
    std::fs::create_dir_all(data_dir).ok();
    let normalized = DictionaryFile {
        version: DICTIONARY_VERSION,
        entries: file.entries.drain(..).filter_map(normalize_entry).collect(),
        updated_at_ms: now_ms(),
    }
    .normalize();

    let r: Result<()> = (|| {
        let s = serde_json::to_string_pretty(&normalized).context("serialize dictionary failed")?;
        fs::write(&p, s).context("write dictionary failed")?;
        Ok(())
    })();
    match r {
        Ok(_) => {
            span.ok(Some(serde_json::json!({"count": normalized.entries.len(), "version": normalized.version})));
            Ok(normalized)
        }
        Err(e) => {
            span.err_anyhow("io", "E_DICTIONARY_SAVE", &e, None);
            Err(e)
        }
    }
}

enum ImportMode {
    Merge,
    Replace,
}

impl ImportMode {
    fn parse(v: &str) -> Option<Self> {
        match v {
            "merge" => Some(Self::Merge),
            "replace" => Some(Self::Replace),
            _ => None,
        }
    }
}

pub fn export_dictionary_json(data_dir: &Path) -> Result<String> {
    let file = load_dictionary(data_dir)?;
    serde_json::to_string_pretty(&file).context("serialize dictionary failed")
}

pub fn import_dictionary_json(data_dir: &Path, json: &str, mode: &str) -> Result<usize> {
    let span = Span::start(
        data_dir,
        None,
        "Dictionary",
        "DICT.import",
        Some(serde_json::json!({"mode": mode, "json_chars": json.len()})),
    );

    let mode = ImportMode::parse(mode).ok_or_else(|| {
        let e = anyhow!("E_DICTIONARY_IMPORT_MODE: mode must be merge or replace");
        e
    })?;

    if json.trim().is_empty() {
        return Ok(0);
    }

    let payload = serde_json::from_str::<Value>(json).context("dictionary import json invalid")?;
    let incoming: Vec<DictionaryEntry> = match payload {
        Value::Object(_) => {
            #[derive(Deserialize)]
            struct Wrapper {
                entries: Vec<DictionaryEntry>,
            }
            serde_json::from_value(payload).map(|w: Wrapper| w.entries)
                .context("import json needs entries array")?
        }
        _ => serde_json::from_value(payload).context("import json must be array or { entries }")?,
    };

    let mut normalized = Vec::<DictionaryEntry>::new();
    let mut seen = HashSet::new();
    for e in incoming {
        if let Some(ne) = normalize_entry(e) {
            // normalize_entry ensures non-empty source/preferred and id.
            // Deduplicate by source term, prefer later entry.
            if seen.contains(&ne.source_term) {
                let source = ne.source_term.clone();
                normalized.retain(|x| x.source_term != source);
                seen.remove(&source);
            }
            seen.insert(ne.source_term.clone());
            normalized.push(ne);
        }
    }

    let mut base = match mode {
        ImportMode::Replace => default_dictionary(),
        ImportMode::Merge => load_dictionary(data_dir).unwrap_or_else(|_| default_dictionary()),
    };
    match mode {
        ImportMode::Replace => {
            base.entries = normalized;
            base.version = DICTIONARY_VERSION;
        }
        ImportMode::Merge => {
            let mut entries = base.entries;
            for e in normalized {
                let pos = entries
                    .iter()
                    .position(|x| x.source_term.eq_ignore_ascii_case(&e.source_term));
                if let Some(i) = pos {
                    entries[i] = e;
                } else {
                    entries.push(e);
                }
            }
            base.entries = entries;
            base.version = DICTIONARY_VERSION;
        }
    }

    let saved = save_dictionary(data_dir, base)?;
    span.ok(Some(serde_json::json!({"count": saved.entries.len()})));
    Ok(saved.entries.len())
}

fn truncate_text(s: String, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s;
    }
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(ch);
    }
    out
}

pub fn dictionary_context_section(file: &DictionaryFile, max_chars: usize) -> String {
    let mut enabled: Vec<&DictionaryEntry> = file
        .entries
        .iter()
        .filter(|e| e.enabled)
        .collect();
    if enabled.is_empty() || max_chars == 0 {
        return String::new();
    }

    // Keep order and apply simple dedupe for output stability.
    let mut seen = HashSet::new();
    let mut ordered = Vec::with_capacity(enabled.len());
    for e in enabled.drain(..) {
        let k = e.source_term.to_lowercase();
        if seen.insert(k) {
            ordered.push(e);
        }
    }

    let mut out = String::new();
    out.push_str("### DICTIONARY\n");

    for e in ordered {
        let mut line = format!("{} -> {}", e.source_term, e.preferred_term);
        if let Some(note) = e.note.as_deref() {
            let note = note.trim();
            if !note.is_empty() {
                line.push_str(&format!(" # {}", note));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }

    truncate_text(out.trim_end().to_string(), max_chars)
}

#[cfg(test)]
mod tests {
    use super::{default_dictionary, dictionary_context_section, load_dictionary, DictionaryEntry, DictionaryFile};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn dictionary_default_and_context_section() {
        let file = default_dictionary();
        assert_eq!(file.version, 1);
        assert_eq!(file.entries.len(), 0);

        let txt = dictionary_context_section(&file, 100);
        assert!(txt.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = PathBuf::from("/tmp/typevoice-dict-test");
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("dictionary.json");
        let _ = fs::remove_file(&p);

        let file = DictionaryFile {
            version: 1,
            entries: vec![DictionaryEntry {
                id: String::new(),
                source_term: "GPU".to_string(),
                preferred_term: "图形处理器".to_string(),
                note: Some("硬件".to_string()),
                enabled: true,
            }],
            updated_at_ms: 0,
        };

        let saved = super::save_dictionary(&dir, file).expect("save");
        let load = load_dictionary(&dir).expect("load");
        assert_eq!(load.entries.len(), 1);
        assert!(!load.entries[0].id.is_empty());
        assert_eq!(load.entries[0].source_term, "GPU");
        assert_eq!(load.entries[0].preferred_term, saved.entries[0].preferred_term);
    }
}
