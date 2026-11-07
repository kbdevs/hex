//! Read meeting templates imported from Superwhisper. Importing a prompt
//! never sends a transcript or runs a model.

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTemplate {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub icon_name: String,
    #[serde(default)]
    pub is_starred: bool,
    pub version: u32,
}

pub fn read(path: &Path) -> Result<MeetingTemplate> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_TEMPLATE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_TEMPLATE_BYTES {
        return Err(eyre!("meeting template exceeds 1 MiB: {}", path.display()));
    }
    let template: MeetingTemplate = serde_json::from_slice(&bytes)
        .wrap_err_with(|| format!("could not decode template {}", path.display()))?;
    if template.key.trim().is_empty()
        || template.name.trim().is_empty()
        || template.prompt.trim().is_empty()
    {
        return Err(eyre!("invalid meeting template {}", path.display()));
    }
    Ok(template)
}

pub fn list() -> Result<Vec<MeetingTemplate>> {
    let root = crate::app_paths::support_dir()?.join("meeting-templates");
    let mut templates = Vec::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(templates),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("json")
        {
            continue;
        }
        templates.push(read(&entry.path())?);
    }
    templates.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(templates)
}

pub fn find(key: &str) -> Result<MeetingTemplate> {
    select(&list()?, key)
        .cloned()
        .ok_or_else(|| eyre!("no meeting template is {key}"))
}

fn select<'a>(templates: &'a [MeetingTemplate], key: &str) -> Option<&'a MeetingTemplate> {
    templates
        .iter()
        .find(|template| template.key == key)
        .or_else(|| {
            templates
                .iter()
                .find(|template| template.name.eq_ignore_ascii_case(key))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_keys_win_over_another_templates_display_name() {
        let templates: Vec<MeetingTemplate> = serde_json::from_value(serde_json::json!([
            {"key":"a","name":"b","prompt":"first","version":1},
            {"key":"b","name":"Other","prompt":"second","version":1}
        ]))
        .unwrap();
        assert_eq!(select(&templates, "b").unwrap().prompt, "second");
        assert_eq!(select(&templates, "other").unwrap().key, "b");
    }

    #[test]
    fn oversized_templates_use_the_same_limit_for_import_and_listing() {
        let path =
            std::env::temp_dir().join(format!("hex-template-limit-{}.json", std::process::id()));
        fs::write(&path, vec![b' '; MAX_TEMPLATE_BYTES as usize + 1]).unwrap();
        let error = read(&path).unwrap_err();
        fs::remove_file(path).unwrap();
        assert!(error.to_string().contains("exceeds 1 MiB"));
    }
}
