use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};

use crossterm::cursor::MoveTo;
use crossterm::execute;
use crossterm::terminal::{Clear, ClearType};
use inquire::validator::Validation;
use inquire::{MultiSelect, Select, Text};

use crate::config::{NameError, validate_instance_name};

/// One entry in a wizard's kind-specific action menu. `id` is a stable
/// dispatch key (typically a variant name from the wizard's internal action
/// enum); `label` is what the user sees.
#[derive(Clone)]
pub struct WizardAction {
    pub id: &'static str,
    pub label: &'static str,
}

impl fmt::Display for WizardAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

pub(crate) fn render_header(crumbs: &[String]) {
    execute!(io::stdout(), Clear(ClearType::All), MoveTo(0, 0)).ok();
    println!("{}", crumbs.join(" › "));
    println!();
    io::stdout().flush().ok();
}

pub(crate) fn pause() -> anyhow::Result<()> {
    Text::new("Press Enter to continue").prompt()?;
    Ok(())
}

pub(crate) fn sorted_keys<V>(map: &HashMap<String, V>) -> Vec<String> {
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

pub(crate) fn pick_with_cancel(prompt: &str, names: &[String]) -> anyhow::Result<Option<String>> {
    let mut options: Vec<String> = names.to_vec();
    options.push("(cancel)".to_string());

    let pick = Select::new(prompt, options).prompt()?;

    if pick == "(cancel)" {
        Ok(None)
    } else {
        Ok(Some(pick))
    }
}

pub(crate) fn prompt_new_name(
    prompt: &str,
    default: &str,
    taken_names: Vec<String>,
    allow_same_as: Option<String>,
) -> anyhow::Result<String> {
    let validator =
        move |value: &str| -> Result<Validation, Box<dyn std::error::Error + Send + Sync>> {
            let trimmed = value.trim();

            if trimmed.is_empty() {
                return Ok(Validation::Invalid("name cannot be empty".into()));
            }

            if let Err(error) = validate_instance_name(trimmed) {
                return Ok(Validation::Invalid(error.to_string().into()));
            }

            if let Some(allow) = &allow_same_as
                && allow == trimmed
            {
                return Ok(Validation::Valid);
            }

            if taken_names.iter().any(|n| n == trimmed) {
                return Ok(Validation::Invalid(
                    NameError::Taken(trimmed.to_string()).to_string().into(),
                ));
            }

            Ok(Validation::Valid)
        };

    let raw = Text::new(prompt)
        .with_default(default)
        .with_validator(validator)
        .prompt()?;
    Ok(raw.trim().to_string())
}

pub(crate) fn suggest_unique_name(taken_names: &[String], base: &str) -> String {
    if !taken_names.iter().any(|n| n == base) {
        return base.to_string();
    }

    for n in 2..1000 {
        let candidate = format!("{base}-{n}");

        if !taken_names.iter().any(|n| n == &candidate) {
            return candidate;
        }
    }

    base.to_string()
}

#[derive(Clone, Copy)]
pub(crate) enum FailureChoice {
    Retry,
    SaveAnyway,
    Cancel,
}

impl fmt::Display for FailureChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Retry => write!(f, "Retry"),
            Self::SaveAnyway => write!(f, "Save anyway (skip verification)"),
            Self::Cancel => write!(f, "Cancel"),
        }
    }
}

pub(crate) fn prompt_failure_choice(error: &anyhow::Error) -> anyhow::Result<FailureChoice> {
    println!();
    eprintln!("Verification failed:");
    eprintln!();
    eprintln!("    {error:?}");
    eprintln!();

    Ok(Select::new(
        "How do you want to proceed?",
        vec![
            FailureChoice::Retry,
            FailureChoice::SaveAnyway,
            FailureChoice::Cancel,
        ],
    )
    .prompt()?)
}

/// Trait implemented by items that can be presented in `pick_optional`
/// or `pick_multiple` pickers. The picker shows `name (#id)` and uses
/// `id` as the dispatch key.
pub(crate) trait Pickable {
    fn id(&self) -> u32;
    fn name(&self) -> &str;
}

struct DisplayPick<'a, T: Pickable>(&'a T);

impl<'a, T: Pickable> fmt::Display for DisplayPick<'a, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (#{})", self.0.name(), self.0.id())
    }
}

enum OptionalPick<'a, T: Pickable> {
    None,
    Item(DisplayPick<'a, T>),
}

impl<'a, T: Pickable> fmt::Display for OptionalPick<'a, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(formatter, "(none)"),
            Self::Item(item) => item.fmt(formatter),
        }
    }
}

pub(crate) fn pick_optional<T: Pickable>(
    prompt: &str,
    items: &[T],
    current: Option<u32>,
) -> anyhow::Result<Option<u32>> {
    let mut options: Vec<OptionalPick<'_, T>> = Vec::with_capacity(items.len() + 1);
    options.push(OptionalPick::None);

    for item in items {
        options.push(OptionalPick::Item(DisplayPick(item)));
    }

    let default_index = current
        .and_then(|id| items.iter().position(|item| item.id() == id).map(|p| p + 1))
        .unwrap_or(0);

    let picked = Select::new(prompt, options)
        .with_starting_cursor(default_index)
        .prompt()?;

    Ok(match picked {
        OptionalPick::None => None,
        OptionalPick::Item(item) => Some(item.0.id()),
    })
}

pub(crate) fn pick_multiple<T: Pickable>(
    prompt: &str,
    items: &[T],
    current: &[u32],
) -> anyhow::Result<Vec<u32>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let pickables: Vec<DisplayPick<'_, T>> = items.iter().map(DisplayPick).collect();
    let defaults: Vec<usize> = pickables
        .iter()
        .enumerate()
        .filter_map(|(index, item)| current.contains(&item.0.id()).then_some(index))
        .collect();

    let picked = MultiSelect::new(prompt, pickables)
        .with_default(&defaults)
        .prompt()?;

    Ok(picked.iter().map(|item| item.0.id()).collect())
}
