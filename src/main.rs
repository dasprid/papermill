use std::process::ExitCode;

mod cli;
mod config;
mod credentials;
mod http;
mod keystore;
mod mark;
mod setup;
mod shutdown;
mod sinks;
mod sources;
mod state;
mod transfer;
mod tty;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("papermill=info")),
        )
        .init();

    if let Err(error) = keyring::use_native_store(true) {
        eprintln!("Failed to initialize keyring: {error}");
        return ExitCode::from(1);
    }

    shutdown::install_handler();

    match cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:?}");
            ExitCode::from(1)
        }
    }
}
