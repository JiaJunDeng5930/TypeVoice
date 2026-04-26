use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::obs::Span;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub task_id: String,
    pub created_at_ms: i64,
    pub asr_text: String,
    pub rewritten_text: String,
    pub inserted_text: String,
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
          rewritten_text TEXT NOT NULL DEFAULT '',
          inserted_text TEXT NOT NULL DEFAULT '',
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
    ensure_column(&c, "rewritten_text", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(&c, "inserted_text", "TEXT NOT NULL DEFAULT ''")?;
    Ok(c)
}

fn ensure_column(c: &Connection, column: &str, definition: &str) -> Result<()> {
    let mut stmt = c
        .prepare("PRAGMA table_info(history)")
        .context("prepare history schema inspection failed")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .context("inspect history schema failed")?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }
    c.execute(
        &format!("ALTER TABLE history ADD COLUMN {column} {definition}"),
        [],
    )
    .with_context(|| format!("add history column {column} failed"))?;
    Ok(())
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
            "rewritten_chars": item.rewritten_text.len(),
            "inserted_chars": item.inserted_text.len(),
            "final_chars": item.final_text.len(),
        })),
    );

    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err_anyhow("db", "E_HISTORY_CONN", &e, None);
            return Err(e);
        }
    };
    let r = c.execute(
        r#"
        INSERT OR REPLACE INTO history
        (task_id, created_at_ms, asr_text, rewritten_text, inserted_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            item.task_id,
            item.created_at_ms,
            item.asr_text,
            item.rewritten_text,
            item.inserted_text,
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
            let ae = anyhow::anyhow!(e).context("insert history failed");
            span.err_anyhow("db", "E_HISTORY_INSERT", &ae, None);
            Err(ae)
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

    let result: Result<Vec<HistoryItem>> = (|| {
        let c = conn(db_path)?;
        let mut out = Vec::new();
        match before_ms {
            Some(ms) => {
                let mut stmt = c
                    .prepare(
                        r#"
                        SELECT task_id, created_at_ms, asr_text, rewritten_text, inserted_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms
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
                            rewritten_text: row.get(3)?,
                            inserted_text: row.get(4)?,
                            final_text: row.get(5)?,
                            template_id: row.get(6)?,
                            rtf: row.get(7)?,
                            device_used: row.get(8)?,
                            preprocess_ms: row.get(9)?,
                            asr_ms: row.get(10)?,
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
                        SELECT task_id, created_at_ms, asr_text, rewritten_text, inserted_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms
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
                            rewritten_text: row.get(3)?,
                            inserted_text: row.get(4)?,
                            final_text: row.get(5)?,
                            template_id: row.get(6)?,
                            rtf: row.get(7)?,
                            device_used: row.get(8)?,
                            preprocess_ms: row.get(9)?,
                            asr_ms: row.get(10)?,
                        })
                    })
                    .context("query history list failed")?;
                for r in rows {
                    out.push(r?);
                }
            }
        }
        Ok(out)
    })();

    match result {
        Ok(out) => {
            span.ok(Some(serde_json::json!({"items": out.len()})));
            Ok(out)
        }
        Err(e) => {
            span.err_anyhow("db", "E_HISTORY_LIST", &e, None);
            Err(e)
        }
    }
}

pub fn update_final_text(
    db_path: &Path,
    task_id: &str,
    final_text: &str,
    template_id: Option<&str>,
) -> Result<()> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let span = Span::start(
        data_dir,
        Some(task_id),
        "History",
        "HISTORY.update_final_text",
        Some(serde_json::json!({
            "template_id": template_id,
            "final_chars": final_text.len(),
        })),
    );
    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err_anyhow("db", "E_HISTORY_CONN", &e, None);
            return Err(e);
        }
    };
    let r = c.execute(
        r#"
        UPDATE history
        SET rewritten_text = ?2, final_text = ?2, template_id = ?3
        WHERE task_id = ?1
        "#,
        params![task_id, final_text, template_id],
    );
    match r {
        Ok(0) => {
            let ae = anyhow::anyhow!("E_HISTORY_NOT_FOUND: task_id not found");
            span.err_anyhow("db", "E_HISTORY_NOT_FOUND", &ae, None);
            Err(ae)
        }
        Ok(_) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            let ae = anyhow::anyhow!(e).context("update history final_text failed");
            span.err_anyhow("db", "E_HISTORY_UPDATE", &ae, None);
            Err(ae)
        }
    }
}

pub fn update_inserted_text(db_path: &Path, task_id: &str, inserted_text: &str) -> Result<()> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let span = Span::start(
        data_dir,
        Some(task_id),
        "History",
        "HISTORY.update_inserted_text",
        Some(serde_json::json!({
            "inserted_chars": inserted_text.len(),
        })),
    );
    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err_anyhow("db", "E_HISTORY_CONN", &e, None);
            return Err(e);
        }
    };
    let r = c.execute(
        r#"
        UPDATE history
        SET inserted_text = ?2, final_text = ?2
        WHERE task_id = ?1
        "#,
        params![task_id, inserted_text],
    );
    match r {
        Ok(0) => {
            let ae = anyhow::anyhow!("E_HISTORY_NOT_FOUND: task_id not found");
            span.err_anyhow("db", "E_HISTORY_NOT_FOUND", &ae, None);
            Err(ae)
        }
        Ok(_) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            let ae = anyhow::anyhow!(e).context("update history inserted_text failed");
            span.err_anyhow("db", "E_HISTORY_UPDATE", &ae, None);
            Err(ae)
        }
    }
}

pub fn clear(db_path: &Path) -> Result<()> {
    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let span = Span::start(data_dir, None, "History", "HISTORY.clear", None);
    let c = match conn(db_path) {
        Ok(c) => c,
        Err(e) => {
            span.err_anyhow("db", "E_HISTORY_CONN", &e, None);
            return Err(e);
        }
    };
    match c.execute("DELETE FROM history", []) {
        Ok(_) => {
            span.ok(None);
            Ok(())
        }
        Err(e) => {
            let ae = anyhow::anyhow!(e).context("clear history failed");
            span.err_anyhow("db", "E_HISTORY_CLEAR", &ae, None);
            Err(ae)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_final_text_changes_existing_history_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("history.sqlite3");
        append(
            &db,
            &HistoryItem {
                task_id: "task-1".to_string(),
                created_at_ms: 1,
                asr_text: "raw".to_string(),
                rewritten_text: String::new(),
                inserted_text: String::new(),
                final_text: "raw".to_string(),
                template_id: None,
                rtf: 0.4,
                device_used: "cuda".to_string(),
                preprocess_ms: 10,
                asr_ms: 20,
            },
        )
        .expect("append");

        update_final_text(&db, "task-1", "rewritten", Some("template-1")).expect("update");

        let rows = list(&db, 10, None).expect("list");
        assert_eq!(rows[0].final_text, "rewritten");
        assert_eq!(rows[0].rewritten_text, "rewritten");
        assert_eq!(rows[0].template_id.as_deref(), Some("template-1"));
    }

    #[test]
    fn update_inserted_text_changes_existing_history_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("history.sqlite3");
        append(
            &db,
            &HistoryItem {
                task_id: "task-1".to_string(),
                created_at_ms: 1,
                asr_text: "raw".to_string(),
                rewritten_text: "rewritten".to_string(),
                inserted_text: String::new(),
                final_text: "rewritten".to_string(),
                template_id: Some("template-1".to_string()),
                rtf: 0.4,
                device_used: "cuda".to_string(),
                preprocess_ms: 10,
                asr_ms: 20,
            },
        )
        .expect("append");

        update_inserted_text(&db, "task-1", "inserted").expect("update");

        let rows = list(&db, 10, None).expect("list");
        assert_eq!(rows[0].inserted_text, "inserted");
        assert_eq!(rows[0].final_text, "inserted");
        assert_eq!(rows[0].rewritten_text, "rewritten");
    }

    #[test]
    fn old_history_schema_gets_new_text_columns() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = tmp.path().join("history.sqlite3");
        {
            let c = Connection::open(&db).expect("open");
            c.execute_batch(
                r#"
                CREATE TABLE history (
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
                INSERT INTO history
                (task_id, created_at_ms, asr_text, final_text, template_id, rtf, device_used, preprocess_ms, asr_ms)
                VALUES ('task-1', 1, 'raw', 'final', NULL, 0.4, 'cuda', 10, 20);
                "#,
            )
            .expect("seed");
        }

        let rows = list(&db, 10, None).expect("list");
        assert_eq!(rows[0].asr_text, "raw");
        assert_eq!(rows[0].rewritten_text, "");
        assert_eq!(rows[0].inserted_text, "");
        assert_eq!(rows[0].final_text, "final");
    }
}
