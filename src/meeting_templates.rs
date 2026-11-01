//! Read meeting templates imported from Superwhisper. Importing a prompt
//! never sends a transcript or runs a model.

use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};

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
        let mut bytes = Vec::new();
        fs::File::open(entry.path())?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 1024 * 1024 {
            return Err(eyre!("meeting template exceeds 1 MiB"));
        }
        let template: MeetingTemplate = serde_json::from_slice(&bytes)
            .wrap_err_with(|| format!("could not decode template {}", entry.path().display()))?;
        templates.push(template);
    }
    templates.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(templates)
}

pub fn find(key: &str) -> Result<MeetingTemplate> {
    list()?
        .into_iter()
        .find(|template| template.key == key || template.name.eq_ignore_ascii_case(key))
        .ok_or_else(|| eyre!("no meeting template is {key}"))
}
