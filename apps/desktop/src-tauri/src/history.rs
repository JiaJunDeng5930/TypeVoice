use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::trace::Span;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub task_id: String,
    pub created_at_ms: i64,
    pub asr_text: String,
    pub final_text: String,
    pub template_id: Option<String>,
    pub rtf: f64,
    pub device_used: String,
    pub preprocess_ms: i64,
    pub asr_ms: i64,
}

fn conn(db_path: &Path) -> Result<Connection> {
    let c = Connection::open(db_path).context("open sqlite failed")?;
    c.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS history (
          task_id TEXT PRIMARY KEY,
          created_at_ms INTEGER NOT NULL,
          asr_text TEXT NOT NULL,
          final_text TEXT NOT NULL,
          template_id TEXT NULL,
          rtf REAL NOT NULL,
          device_used TEXT NOT NULL,
          preprocess_ms INTEGER NOT NULL,
          asr_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_history_created_at ON history(created_at_ms DESC);
        "#,
    )
    .context("init sqlite schema failed")?;
    Ok(c)
}

pub fn append(db_path: &Path, item: &HistoryItem) -> Result<()> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let span = Span::start(
        data_dir,
        Some(item.task_id.as_str()),
        "History",
        "HISTORY.append",
        Some(serde_json::json!({
            "template_id": item.template_id,
            "asr_chars": item.asr_text.len(),
            "final_chars": item.final_text.len(),
        })),
    );

    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err("db", "E_HISTORY_CONN", &e.to_string(), None);
            return Err(e);
        }
    };
    let r = c.execute(
        r#"
        INSERT OR REPLACE INTO history
        (task_id, created_at_ms, asr_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            item.task_id,
            item.created_at_ms,
            item.asr_text,
            item.final_text,
            item.template_id,
            item.rtf,
            item.device_used,
            item.preprocess_ms,
            item.asr_ms,
        ],
    );
    match r {
        Ok(_) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err("db", "E_HISTORY_INSERT", &format!("{e}"), None);
            Err(anyhow::anyhow!(e).context("insert history failed"))
        }
    }
}

pub fn list(db_path: &Path, limit: i64, before_ms: Option<i64>) -> Result<Vec<HistoryItem>> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let span = Span::start(
        data_dir,
        None,
        "History",
        "HISTORY.list",
        Some(serde_json::json!({"limit": limit, "before_ms": before_ms})),
    );

    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err("db", "E_HISTORY_CONN", &e.to_string(), None);
            return Err(e);
        }
    };
    let mut out = Vec::new();
    match before_ms {
        Some(ms) => {
            let mut stmt = c
                .prepare(
                    r#"
                    SELECT task_id, created_at_ms, asr_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms
                    FROM history
                    WHERE created_at_ms < ?1
                    ORDER BY created_at_ms DESC
                    LIMIT ?2
                    "#,
                )
                .context("prepare history list failed")?;
            let rows = stmt
                .query_map(params![ms, limit], |row| {
                    Ok(HistoryItem {
                        task_id: row.get(0)?,
                        created_at_ms: row.get(1)?,
                        asr_text: row.get(2)?,
                        final_text: row.get(3)?,
                        template_id: row.get(4)?,
                        rtf: row.get(5)?,
                        device_used: row.get(6)?,
                        preprocess_ms: row.get(7)?,
                        asr_ms: row.get(8)?,
                    })
                })
                .context("query history list failed")?;
            for r in rows {
                out.push(r?);
            }
        }
        None => {
            let mut stmt = c
                .prepare(
                    r#"
                    SELECT task_id, created_at_ms, asr_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms
                    FROM history
                    ORDER BY created_at_ms DESC
                    LIMIT ?1
                    "#,
                )
                .context("prepare history list failed")?;
            let rows = stmt
                .query_map(params![limit], |row| {
                    Ok(HistoryItem {
                        task_id: row.get(0)?,
                        created_at_ms: row.get(1)?,
                        asr_text: row.get(2)?,
                        final_text: row.get(3)?,
                        template_id: row.get(4)?,
                        rtf: row.get(5)?,
                        device_used: row.get(6)?,
                        preprocess_ms: row.get(7)?,
                        asr_ms: row.get(8)?,
                    })
                })
                .context("query history list failed")?;
            for r in rows {
                out.push(r?);
            }
        }
    }
    span.ok(Some(serde_json::json!({"items": out.len()})));
    Ok(out)
}

pub fn clear(db_path: &Path) -> Result<()> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let span = Span::start(data_dir, None, "History", "HISTORY.clear", None);
    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err("db", "E_HISTORY_CONN", &e.to_string(), None);
            return Err(e);
        }
    };
    match c.execute("DELETE FROM history", []) {
        Ok(_) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            span.err("db", "E_HISTORY_CLEAR", &format!("{e}"), None);
            Err(anyhow::anyhow!(e).context("clear history failed"))
        }
    }
}
