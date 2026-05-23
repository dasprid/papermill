use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

use crate::sinks::filesystem::FilesystemBinding;
use crate::sources::SourceKind;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub sinks: HashMap<String, SinkInstance>,
    #[serde(default)]
    pub sources: HashMap<String, SourceInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SinkInstance {
    Paperless {
        #[serde(deserialize_with = "deserialize_base_url")]
        base_url: Url,
    },
    Filesystem {
        root: PathBuf,
    },
}

impl SinkInstance {
    pub fn kind(&self) -> crate::sinks::SinkKind {
        match self {
            Self::Paperless { .. } => crate::sinks::SinkKind::Paperless,
            Self::Filesystem { .. } => crate::sinks::SinkKind::Filesystem,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInstance {
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sinks: HashMap<String, SinkBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SinkBinding {
    Paperless(PaperlessBinding),
    Filesystem(FilesystemBinding),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PaperlessBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correspondent_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_type_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag_ids: Vec<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum NameError {
    #[error(
        "instance name must match ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ \
         (lowercase letters, digits, dashes; cannot start or end with a dash)"
    )]
    Invalid,
    #[error("instance name \"{0}\" is already taken")]
    Taken(String),
}

pub fn validate_instance_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Invalid);
    }

    let bytes = name.as_bytes();
    let first_last_ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let interior_ok = |b: u8| first_last_ok(b) || b == b'-';

    if !first_last_ok(bytes[0]) || !first_last_ok(bytes[bytes.len() - 1]) {
        return Err(NameError::Invalid);
    }

    if !bytes.iter().copied().all(interior_ok) {
        return Err(NameError::Invalid);
    }

    Ok(())
}

impl Config {
    // ---- source instance helpers ----

    pub fn source_instance(&self, instance_name: &str) -> Option<&SourceInstance> {
        self.sources.get(instance_name)
    }

    pub fn has_source(&self, instance_name: &str) -> bool {
        self.sources.contains_key(instance_name)
    }

    pub fn add_source_instance(
        &mut self,
        instance_name: &str,
        kind: SourceKind,
    ) -> Result<(), NameError> {
        validate_instance_name(instance_name)?;
        if self.has_source(instance_name) {
            return Err(NameError::Taken(instance_name.to_string()));
        }
        self.sources.insert(
            instance_name.to_string(),
            SourceInstance {
                kind,
                sinks: HashMap::new(),
            },
        );
        Ok(())
    }

    pub fn rename_source_in_config(&mut self, old: &str, new: &str) -> Result<(), NameError> {
        validate_instance_name(new)?;
        if old == new {
            return Ok(());
        }
        if self.has_source(new) {
            return Err(NameError::Taken(new.to_string()));
        }
        let Some(instance) = self.sources.remove(old) else {
            return Ok(());
        };
        self.sources.insert(new.to_string(), instance);
        Ok(())
    }

    pub fn remove_source(&mut self, instance_name: &str) -> bool {
        self.sources.remove(instance_name).is_some()
    }

    // ---- sink instance helpers ----

    pub fn has_sink(&self, instance_name: &str) -> bool {
        self.sinks.contains_key(instance_name)
    }

    pub fn set_paperless_sink(
        &mut self,
        instance_name: &str,
        base_url: Url,
    ) -> Result<(), NameError> {
        validate_instance_name(instance_name)?;
        self.sinks.insert(
            instance_name.to_string(),
            SinkInstance::Paperless { base_url },
        );
        Ok(())
    }

    pub fn set_filesystem_sink(
        &mut self,
        instance_name: &str,
        root: PathBuf,
    ) -> Result<(), NameError> {
        validate_instance_name(instance_name)?;
        self.sinks
            .insert(instance_name.to_string(), SinkInstance::Filesystem { root });
        Ok(())
    }

    pub fn delete_sink(&mut self, instance_name: &str) -> bool {
        if self.sinks.remove(instance_name).is_none() {
            return false;
        }
        for source in self.sources.values_mut() {
            source.sinks.remove(instance_name);
        }
        true
    }

    pub fn rename_sink_in_config(&mut self, old: &str, new: &str) -> Result<(), NameError> {
        validate_instance_name(new)?;
        if old == new {
            return Ok(());
        }
        if self.has_sink(new) {
            return Err(NameError::Taken(new.to_string()));
        }
        let Some(sink) = self.sinks.remove(old) else {
            return Ok(());
        };
        self.sinks.insert(new.to_string(), sink);
        for source in self.sources.values_mut() {
            if let Some(binding) = source.sinks.remove(old) {
                source.sinks.insert(new.to_string(), binding);
            }
        }
        Ok(())
    }

    // ---- binding helpers ----

    pub fn set_binding(
        &mut self,
        source_name: &str,
        sink_name: &str,
        binding: SinkBinding,
    ) -> anyhow::Result<()> {
        let source = self
            .sources
            .get_mut(source_name)
            .with_context(|| format!("Source instance \"{source_name}\" does not exist"))?;
        source.sinks.insert(sink_name.to_string(), binding);
        Ok(())
    }

    pub fn remove_binding(&mut self, source_name: &str, sink_name: &str) -> bool {
        self.sources
            .get_mut(source_name)
            .is_some_and(|source| source.sinks.remove(sink_name).is_some())
    }
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

    load_from_str(&raw)
}

pub fn save(config: &Config) -> anyhow::Result<()> {
    let path = config_path()?;
    write_to(&path, config)
}

fn write_to(path: &Path, config: &Config) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
    }

    let raw = toml::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(path, raw)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}

fn load_from_str(raw: &str) -> anyhow::Result<Config> {
    toml::from_str(raw).context("Failed to parse config as TOML")
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

    #[test]
    fn new_shape_passes_through_unchanged() {
        let new_shape = r#"
[sinks.paperless]
kind = "paperless"
base_url = "https://example.com/"

[sources.o2]
kind = "o2"

[sources.o2.sinks.paperless]
kind = "paperless"
correspondent_id = 16
tag_ids = [5]
"#;
        let config = load_from_str(new_shape).unwrap();
        assert!(config.sinks.contains_key("paperless"));
        let o2 = config.sources.get("o2").unwrap();
        let SinkBinding::Paperless(binding) = o2.sinks.get("paperless").unwrap() else {
            panic!("expected paperless binding");
        };
        assert_eq!(binding.correspondent_id, Some(16));
    }

    #[test]
    fn round_trip_preserves_new_shape() {
        let new_shape = r#"
[sinks.paperless]
kind = "paperless"
base_url = "https://example.com/"

[sources.o2]
kind = "o2"

[sources.o2.sinks.paperless]
kind = "paperless"
correspondent_id = 16
document_type_id = 4
tag_ids = [5]
"#;
        let config = load_from_str(new_shape).unwrap();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let reloaded = load_from_str(&serialized).unwrap();

        assert_eq!(reloaded.sinks.len(), 1);
        assert_eq!(reloaded.sources.len(), 1);
        let o2 = reloaded.sources.get("o2").unwrap();
        let SinkBinding::Paperless(binding) = o2.sinks.get("paperless").unwrap() else {
            panic!("expected paperless binding");
        };
        assert_eq!(binding.correspondent_id, Some(16));
        assert_eq!(binding.document_type_id, Some(4));
        assert_eq!(binding.tag_ids, vec![5]);
    }
}
