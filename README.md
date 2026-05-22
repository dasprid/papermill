# papermill

[![CI](https://github.com/DASPRiD/papermill/actions/workflows/ci.yml/badge.svg)](https://github.com/DASPRiD/papermill/actions/workflows/ci.yml)

Pull invoices from telco-style portals into a Paperless-ngx instance.

A CLI that replaces the manual "log in, find the invoice, download the PDF, upload to paperless" loop. Runs from cron or interactively. Tracks each transferred invoice in a local SQLite database so reruns skip what's already done.

## Supported sources

- `vodafone`: Vodafone Germany
- `o2`: Telefónica O2 Germany
- `mock`: local test source

To add a new source: implement the `Source` trait and register it via the `sources!` macro in `src/sources/mod.rs`. The macro generates the dispatch enum, the `name`/`label`/`build` methods, and a compile-time assertion that each source's `KIND` const matches its macro registration.

## Installation

```sh
cargo install --path .
```

Or build a release binary with `cargo build --release` and copy `target/release/papermill` somewhere on your `PATH`.

## Setup

```sh
papermill setup
```

The wizard collects:

- Paperless base URL
- Paperless API token (stored in the OS keyring)
- Per-source Paperless target metadata: correspondent, document type, tags

Source credentials are prompted on first use of each source.

Storage locations:

- Config: `$XDG_CONFIG_HOME/papermill/config.toml`
- State DB: `$XDG_DATA_HOME/papermill/state.db`
- Secrets: OS keyring (libsecret on Linux, Keychain on macOS, Credential Manager on Windows)

## Usage

```sh
# Transfer new invoices from one source
papermill transfer vodafone

# Transfer from every configured source
papermill transfer --all

# Show what would happen without uploading
papermill transfer --all --dry-run

# Record invoices as transferred without uploading (for backfilling state)
papermill mark o2 --until 2026-01-01
papermill mark --all
```

Press Ctrl-C to interrupt a run. The in-flight invoice finishes so the paperless task ID gets recorded in the state DB, then the process exits before the next invoice.

## Logging

Per-invoice progress comes through `tracing`; per-source summary lines go to stdout for cron-friendly capture.

```sh
# Default: per-invoice info + summary
papermill transfer --all

# Just summaries (cron mode)
RUST_LOG=papermill=warn papermill transfer --all

# Verbose: also show skipped (already-transferred) invoices
RUST_LOG=papermill=debug papermill transfer --all
```

## Development

```sh
cargo build
cargo test
cargo clippy
```

State DB schema changes go in `migrations/`. `sqlx` tracks applied migrations and runs new ones at startup.

A separate trace tool records browser network traffic from a real Chromium session. Use it to capture requests, cookies, and response bodies when investigating a new portal:

```sh
cargo build --features trace
./target/debug/papermill-trace <label> --url https://example.com/login
```

The trace tool is only built with the `trace` feature, so the main binary doesn't pull in `chromiumoxide`.

## License

BSD 2-Clause. See [LICENSE](LICENSE).
