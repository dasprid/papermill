use std::io::{self, IsTerminal};

use anyhow::bail;

pub fn require() -> anyhow::Result<()> {
    if !io::stdin().is_terminal() {
        bail!(
            "interactive input required but stdin is not a TTY. Run the command interactively to set credentials or tokens"
        );
    }

    Ok(())
}
