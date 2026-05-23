use std::fmt;

use inquire::Select;

use crate::config::{self, Config};

pub(crate) mod prompts;
mod sinks;
mod sources;

use prompts::{pause, render_header};

pub async fn run() -> anyhow::Result<()> {
    let mut config = load_or_default()?;
    let mut crumbs = vec!["Papermill setup".to_string()];

    loop {
        render_header(&crumbs);

        let action = Select::new(
            "Choose:",
            vec![
                TopAction::Sources,
                TopAction::Sinks,
                TopAction::View,
                TopAction::Exit,
            ],
        )
        .prompt()?;

        match action {
            TopAction::Sources => sources::run_menu(&mut config, &mut crumbs).await?,
            TopAction::Sinks => sinks::run_menu(&mut config, &mut crumbs).await?,
            TopAction::View => view_config(&config, &crumbs)?,
            TopAction::Exit => return Ok(()),
        }
    }
}

#[derive(Clone)]
enum TopAction {
    Sources,
    Sinks,
    View,
    Exit,
}

impl fmt::Display for TopAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sources => write!(f, "Sources"),
            Self::Sinks => write!(f, "Sinks"),
            Self::View => write!(f, "View config"),
            Self::Exit => write!(f, "Exit"),
        }
    }
}

fn view_config(config: &Config, crumbs: &[String]) -> anyhow::Result<()> {
    let mut local = crumbs.to_vec();
    local.push("View".to_string());
    render_header(&local);

    if config.sinks.is_empty() {
        println!("Sinks: (none)");
    } else {
        println!("Sinks:");
        let mut names: Vec<&String> = config.sinks.keys().collect();
        names.sort();

        for name in names {
            let kind = config.sinks[name].kind();
            println!("  {name} ({})", kind.label());
            kind.wizard().print_summary(config, name, "    ");
        }
    }

    println!();

    if config.sources.is_empty() {
        println!("Sources: (none)");
    } else {
        println!("Sources:");
        let mut names: Vec<&String> = config.sources.keys().collect();
        names.sort();

        for name in names {
            let source = &config.sources[name];
            println!("  {name} ({})", source.kind.label());

            if source.sinks.is_empty() {
                println!("    Bindings: (none)");
            } else {
                let mut bnames: Vec<&String> = source.sinks.keys().collect();
                bnames.sort();
                let bnames_str = bnames
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");

                println!("    Bindings: {bnames_str}");
            }
        }
    }

    println!();
    pause()
}

fn load_or_default() -> anyhow::Result<Config> {
    let path = config::config_path()?;

    if path.exists() {
        config::load()
    } else {
        Ok(Config::default())
    }
}
