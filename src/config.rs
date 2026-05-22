use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

use crate::sources::SourceKind;

#[derive(Debug, Serialize, Deserialize)]
pub struct PaperlessConfig {
    #[serde(deserialize_with = "deserialize_base_url")]
    pub base_url: Url,
}

fn deserialize_base_url<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let url = Url::deserialize(deserializer)?;
    Ok(ensure_trailing_slash(url))
}

pub fn ensure_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }

    url
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correspondent_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_type_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_ids: Vec<u32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub paperless: PaperlessConfig,
    pub sources: HashMap<SourceKind, SourceConfig>,
}

impl Default for PaperlessConfig {
    fn default() -> Self {
        Self {
            base_url: Url::parse("https://paperless.example.com/")
                .expect("static placeholder URL parses"),
        }
    }
}

fn project_dirs() -> anyhow::Result<ProjectDirs> {
    ProjectDirs::from("de", "dasprids", "papermill")
        .context("Failed to determine OS config directory for papermill")
}

pub fn config_path() -> anyhow::Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join("config.toml"))
}

pub fn state_path() -> anyhow::Result<PathBuf> {
    Ok(project_dirs()?.data_dir().join("state.db"))
}

pub fn load() -> anyhow::Result<Config> {
    let path = config_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config at {}", path.display()))?;
    let config: Config = toml::from_str(&raw)
        .with_context(|| format!("Failed to parse config at {}", path.display()))?;
    Ok(config)
}

pub fn save(config: &Config) -> anyhow::Result<()> {
    let path = config_path()?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
    }

    let raw = toml::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(&path, raw)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_trailing_slash_keeps_existing_slash() {
        let url = Url::parse("https://example.com/").unwrap();
        assert_eq!(ensure_trailing_slash(url).as_str(), "https://example.com/");
    }

    #[test]
    fn ensure_trailing_slash_appends_when_missing() {
        let url = Url::parse("https://example.com/paperless").unwrap();
        assert_eq!(
            ensure_trailing_slash(url).as_str(),
            "https://example.com/paperless/"
        );
    }

    #[test]
    fn ensure_trailing_slash_handles_nested_paths() {
        let url = Url::parse("https://example.com/a/b/c").unwrap();
        assert_eq!(
            ensure_trailing_slash(url).as_str(),
            "https://example.com/a/b/c/"
        );
    }
}
