use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

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
    let c = conn(db_path)?;
    c.execute(
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
    )
    .context("insert history failed")?;
    Ok(())
}

pub fn list(db_path: &Path, limit: i64) -> Result<Vec<HistoryItem>> {
    let c = conn(db_path)?;
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
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn clear(db_path: &Path) -> Result<()> {
    let c = conn(db_path)?;
    c.execute("DELETE FROM history", [])
        .context("clear history failed")?;
    Ok(())
}
