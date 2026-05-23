# Papermill

[![CI](https://github.com/DASPRiD/papermill/actions/workflows/ci.yml/badge.svg)](https://github.com/DASPRiD/papermill/actions/workflows/ci.yml)

A CLI that pulls invoices from web portals and files them into Paperless-ngx, a local directory,
or both. Built to grow: each portal is a `Source` implementation behind a small trait, registered
through one macro line.

Runs from cron or interactively. Tracks each `(source, sink, invoice)` tuple in a local SQLite
database, so reruns only fetch what's new and adding a new sink later backfills cleanly.

## Supported portals

- `vodafone`: Vodafone Germany
- `o2`: Telefónica O2 Germany

See [Extending](#extending) for adding your own.

## Supported destinations (sinks)

- **Paperless-ngx**: uploads to a Paperless instance with per-source correspondent, document type,
  and tag mapping.
- **Filesystem**: writes the PDF to a directory with a configurable filename template (`{date}`,
  `{number}`, `{source}` tokens, optional per-year subdirectories).

Sources and sinks are both *instanced*. You can run two Paperless servers, an O2 account alongside
a Vodafone account, and route each source to any subset of sinks.

## Quick start

Grab a prebuilt binary for Linux, macOS, or Windows from the
[latest release](https://github.com/DASPRiD/papermill/releases/latest), or build from source with
`cargo install --path .`.

```sh
papermill setup
papermill transfer --all
```

The wizard walks you through adding at least one source, one sink, and a binding between them.

## Setup wizard

`papermill setup` is the only configuration entry point. From it you can:

- Add, rename, delete sources (one source = one telco account).
- Add, rename, delete sinks (one Paperless server or one filesystem root).
- Bind sources to sinks. A binding holds destination-specific metadata: correspondent / document
  type / tags for Paperless, subdirectory / template for filesystem.

Credentials live in the OS keyring, not in the config file. The wizard verifies them against the
live portal before saving.

## Daily usage

```sh
# Transfer new invoices from one source instance
papermill transfer my-vodafone

# Transfer from every configured source
papermill transfer --all

# Transfer from every Vodafone instance only
papermill transfer --kind vodafone

# Show what would happen without uploading
papermill transfer --all --dry-run

# Record invoices as already-transferred without uploading
# (for backfilling state when importing an existing archive)
papermill mark my-o2 --until 2026-01-01
```

Ctrl-C interrupts cleanly: the in-flight invoice finishes so its Paperless task ID and filesystem
path get recorded in the state DB before exit.

### Cron

```sh
RUST_LOG=papermill=warn papermill transfer --all
```

`tracing` drives per-invoice progress (default INFO); summary lines go to stdout. WARN keeps the
cron log to one line per source unless something fails.

## How it's wired

```
Source ──downloads invoice once──┬──▶ Sink A  (e.g. paperless-home)
                                 ├──▶ Sink B  (e.g. paperless-work)
                                 └──▶ Sink C  (e.g. filesystem backup)
```

A **source** lists and downloads invoices from a portal. A **sink** delivers them somewhere.
A **binding** sits between a `(source, sink)` pair and holds the per-destination configuration.

The state database stores one row per `(source, sink, invoice)`, so adding a new sink to an
existing source delivers only what that new sink hasn't seen yet, instead of re-uploading
everything.

## Storage

- Config: `$XDG_CONFIG_HOME/papermill/config.toml`
- State DB: `$XDG_DATA_HOME/papermill/state.db`
- Secrets: OS keyring (libsecret on Linux, Keychain on macOS, Credential Manager on Windows)

## Extending

Adding a new portal is two pieces of work:

1. Implement the `Source` trait (`src/sources/mod.rs`). Two async methods: `list_invoices(since)`
   and `download_invoice(invoice)`.
2. Register it via the `sources!` macro. The macro generates the dispatch enum, the `name` /
   `label` / `build` methods, and a compile-time assertion that the source's `KIND` const matches
   its registration.

For username+password portals, plug `UsernamePasswordSourceWizard` into the source's
`SourceKind::wizard()` arm. For anything weirder (OAuth, MFA, JavaScript challenges), write a
`SourceWizard` impl.

A companion `papermill-trace` binary captures real Chromium network traffic to make portal
reverse-engineering tractable:

```sh
cargo build --features trace
./target/debug/papermill-trace login-flow --url https://portal.example/login
```

It's behind the `trace` feature so the main binary doesn't pull in `chromiumoxide`.

## Development

```sh
cargo build
cargo test
cargo clippy
```

State DB schema changes go in `migrations/`. `sqlx` tracks applied migrations and runs new ones at
startup.

## License

BSD 2-Clause. See [LICENSE](LICENSE).
