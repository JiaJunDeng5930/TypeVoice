use std::{fs, path::{Path, PathBuf}};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    pub system_prompt: String,
}

pub fn default_templates() -> Vec<PromptTemplate> {
    vec![
        PromptTemplate {
            id: "correct".to_string(),
            name: "纠错".to_string(),
            system_prompt: r#"你是一个中文文本校对助手。请在不改变原意的前提下，修正错别字、标点、断句与明显的语病；统一术语；输出最终文本即可，不要输出解释。"#.to_string(),
        },
        PromptTemplate {
            id: "clarify".to_string(),
            name: "表达澄清".to_string(),
            system_prompt: r#"你是一个中文表达优化助手。请在不编造事实的前提下，对含糊、指代不明、表达不清的句子进行补全与重述，使其更清晰、更书面、更易读；保持信息量不减少；输出最终文本即可，不要输出解释。"#.to_string(),
        },
    ]
}

pub fn templates_path(data_dir: &Path) -> PathBuf {
    data_dir.join("templates.json")
}

pub fn load_templates(data_dir: &Path) -> Result<Vec<PromptTemplate>> {
    let p = templates_path(data_dir);
    if !p.exists() {
        return Ok(default_templates());
    }
    let s = fs::read_to_string(&p).context("read templates.json failed")?;
    let t: Vec<PromptTemplate> = serde_json::from_str(&s).context("parse templates.json failed")?;
    Ok(t)
}

pub fn save_templates(data_dir: &Path, templates: &[PromptTemplate]) -> Result<()> {
    fs::create_dir_all(data_dir).ok();
    let p = templates_path(data_dir);
    let s = serde_json::to_string_pretty(templates).context("serialize templates failed")?;
    fs::write(&p, s).context("write templates.json failed")?;
    Ok(())
}

pub fn upsert_template(data_dir: &Path, mut tpl: PromptTemplate) -> Result<PromptTemplate> {
    if tpl.name.trim().is_empty() {
        return Err(anyhow!("template name is required"));
    }
    if tpl.system_prompt.trim().is_empty() {
        return Err(anyhow!("system_prompt is required"));
    }
    if tpl.id.trim().is_empty() {
        tpl.id = Uuid::new_v4().to_string();
    }
    let mut all = load_templates(data_dir)?;
    if let Some(i) = all.iter().position(|x| x.id == tpl.id) {
        all[i] = tpl.clone();
    } else {
        all.push(tpl.clone());
    }
    save_templates(data_dir, &all)?;
    Ok(tpl)
}

pub fn delete_template(data_dir: &Path, id: &str) -> Result<()> {
    let mut all = load_templates(data_dir)?;
    all.retain(|x| x.id != id);
    save_templates(data_dir, &all)?;
    Ok(())
}

pub fn get_template(data_dir: &Path, id: &str) -> Result<PromptTemplate> {
    let all = load_templates(data_dir)?;
    all.into_iter()
        .find(|x| x.id == id)
        .ok_or_else(|| anyhow!("template not found: {id}"))
}

