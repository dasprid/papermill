use std::fmt;

use anyhow::Context;
use inquire::{MultiSelect, Password, PasswordDisplayMode, Select, Text};
use secrecy::SecretString;
use strum::IntoEnumIterator;
use url::Url;

use crate::config::{self, Config, SourceConfig};
use crate::paperless::{self, NamedItem, PaperlessClient};
use crate::sources::SourceKind;

enum Action {
    PaperlessUrl,
    PaperlessToken,
    ConfigureSource,
    DeleteSource,
    ViewConfig,
    Exit,
}

impl fmt::Display for Action {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PaperlessUrl => write!(formatter, "Paperless URL"),
            Self::PaperlessToken => write!(formatter, "Paperless API token"),
            Self::ConfigureSource => write!(formatter, "Configure source"),
            Self::DeleteSource => write!(formatter, "Delete source"),
            Self::ViewConfig => write!(formatter, "View config"),
            Self::Exit => write!(formatter, "Exit"),
        }
    }
}

enum SourcePick {
    Source(SourceKind),
    Cancel,
}

impl fmt::Display for SourcePick {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(kind) => write!(formatter, "{}", kind.label()),
            Self::Cancel => write!(formatter, "Cancel"),
        }
    }
}

struct Pickable<'a>(&'a NamedItem);

impl<'a> fmt::Display for Pickable<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (#{})", self.0.name, self.0.id)
    }
}

enum OptionalPick<'a> {
    None,
    Item(Pickable<'a>),
}

impl<'a> fmt::Display for OptionalPick<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(formatter, "(none)"),
            Self::Item(item) => item.fmt(formatter),
        }
    }
}

pub async fn run() -> anyhow::Result<()> {
    let mut config = load_or_default()?;

    loop {
        let actions = vec![
            Action::PaperlessUrl,
            Action::PaperlessToken,
            Action::ConfigureSource,
            Action::DeleteSource,
            Action::ViewConfig,
            Action::Exit,
        ];

        let action = Select::new("What do you want to do?", actions).prompt()?;

        match action {
            Action::PaperlessUrl => edit_paperless_url(&mut config)?,
            Action::PaperlessToken => edit_paperless_token()?,
            Action::ConfigureSource => configure_source_menu(&mut config).await?,
            Action::DeleteSource => delete_source(&mut config)?,
            Action::ViewConfig => view_config(&config)?,
            Action::Exit => return Ok(()),
        }
    }
}

async fn configure_source_menu(config: &mut Config) -> anyhow::Result<()> {
    let mut choices: Vec<SourcePick> = SourceKind::iter()
        .filter(|kind| !matches!(kind, SourceKind::Mock))
        .map(SourcePick::Source)
        .collect();
    choices.push(SourcePick::Cancel);

    let pick = Select::new("Configure which source?", choices).prompt()?;

    match pick {
        SourcePick::Cancel => Ok(()),
        SourcePick::Source(kind) => configure_source(config, kind).await,
    }
}

fn delete_source(config: &mut Config) -> anyhow::Result<()> {
    let mut choices: Vec<SourcePick> = SourceKind::iter()
        .filter(|kind| config.sources.contains_key(kind))
        .map(SourcePick::Source)
        .collect();

    if choices.is_empty() {
        println!("No source targets configured.");
        return Ok(());
    }

    choices.push(SourcePick::Cancel);

    let pick = Select::new("Delete which source?", choices).prompt()?;

    match pick {
        SourcePick::Cancel => Ok(()),
        SourcePick::Source(kind) => {
            config.sources.remove(&kind);
            config::save(config)?;
            println!("{} target removed.", kind.label());
            Ok(())
        }
    }
}

fn view_config(config: &Config) -> anyhow::Result<()> {
    println!("Paperless URL: {}", config.paperless.base_url);

    let token_status = if paperless::read_token()?.is_some() {
        "set (in keyring)"
    } else {
        "not set"
    };
    println!("Paperless token: {token_status}");

    if config.sources.is_empty() {
        println!();
        println!("No source targets configured.");
        return Ok(());
    }

    println!();
    println!("Configured source targets:");

    for kind in SourceKind::iter() {
        let Some(target) = config.sources.get(&kind) else {
            continue;
        };

        println!("  {}:", kind.label());

        let correspondent = target
            .correspondent_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "(none)".to_string());
        println!("    correspondent_id: {correspondent}");

        let doc_type = target
            .document_type_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "(none)".to_string());
        println!("    document_type_id: {doc_type}");

        if target.tag_ids.is_empty() {
            println!("    tag_ids: (none)");
        } else {
            let tags: Vec<String> = target.tag_ids.iter().map(u32::to_string).collect();
            println!("    tag_ids: [{}]", tags.join(", "));
        }
    }

    Ok(())
}

fn load_or_default() -> anyhow::Result<Config> {
    let path = config::config_path()?;

    if path.exists() {
        config::load()
    } else {
        Ok(Config::default())
    }
}

fn edit_paperless_url(config: &mut Config) -> anyhow::Result<()> {
    let current = config.paperless.base_url.to_string();
    let value = Text::new("Paperless base URL:")
        .with_default(&current)
        .prompt()?;

    let parsed = match Url::parse(&value) {
        Ok(url) => url,
        Err(error) => {
            eprintln!("Invalid URL: {error}. Leaving URL unchanged.");
            return Ok(());
        }
    };

    config.paperless.base_url = config::ensure_trailing_slash(parsed);
    config::save(config)?;
    Ok(())
}

fn edit_paperless_token() -> anyhow::Result<()> {
    let raw = Password::new("Paperless API token:")
        .with_display_mode(PasswordDisplayMode::Masked)
        .without_confirmation()
        .prompt()
        .context("Failed to read Paperless token")?;

    if raw.is_empty() {
        anyhow::bail!("Paperless token cannot be empty");
    }

    paperless::write_token(&SecretString::from(raw))?;
    println!("Token saved to keyring.");
    Ok(())
}

async fn configure_source(config: &mut Config, kind: SourceKind) -> anyhow::Result<()> {
    let token = paperless::read_token()?.context(
        "Failed to find Paperless API token (configure it before adding source targets)",
    )?;

    let client = PaperlessClient::new(config.paperless.base_url.clone(), token)?;

    let correspondents = client.list_correspondents().await?;
    let document_types = client.list_document_types().await?;
    let tags = client.list_tags().await?;

    let current = config.sources.get(&kind).cloned().unwrap_or_default();

    let correspondent_id = pick_optional(
        "Correspondent (optional):",
        &correspondents,
        current.correspondent_id,
    )?;

    let document_type_id = pick_optional(
        "Document type (optional):",
        &document_types,
        current.document_type_id,
    )?;

    let tag_ids = pick_multiple("Tags (optional, space to toggle):", &tags, &current.tag_ids)?;

    config.sources.insert(
        kind,
        SourceConfig {
            correspondent_id,
            document_type_id,
            tag_ids,
        },
    );

    config::save(config)?;
    println!("{} target saved.", kind.label());
    Ok(())
}

fn pick_optional(
    prompt: &str,
    items: &[NamedItem],
    current: Option<u32>,
) -> anyhow::Result<Option<u32>> {
    let mut options: Vec<OptionalPick<'_>> = Vec::with_capacity(items.len() + 1);
    options.push(OptionalPick::None);

    for item in items {
        options.push(OptionalPick::Item(Pickable(item)));
    }

    let default_index = current
        .and_then(|id| items.iter().position(|item| item.id == id).map(|p| p + 1))
        .unwrap_or(0);

    let picked = Select::new(prompt, options)
        .with_starting_cursor(default_index)
        .prompt()?;

    Ok(match picked {
        OptionalPick::None => None,
        OptionalPick::Item(item) => Some(item.0.id),
    })
}

fn pick_multiple(prompt: &str, items: &[NamedItem], current: &[u32]) -> anyhow::Result<Vec<u32>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let pickables: Vec<Pickable<'_>> = items.iter().map(Pickable).collect();
    let defaults: Vec<usize> = pickables
        .iter()
        .enumerate()
        .filter_map(|(index, item)| current.contains(&item.0.id).then_some(index))
        .collect();

    let picked = MultiSelect::new(prompt, pickables)
        .with_default(&defaults)
        .prompt()?;

    Ok(picked.iter().map(|item| item.0.id).collect())
}
