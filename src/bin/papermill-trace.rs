//! Network trace recorder for investigating new sources.
//!
//! Launches a visible Chromium window via `chromiumoxide`. Captures every
//! request, response (including JSON/text bodies), and final cookie state. Press
//! Enter in the terminal when done.
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams, EventRequestWillBeSent, EventResponseReceived, GetRequestPostDataParams,
    GetResponseBodyParams,
};
use chromiumoxide::{Browser, BrowserConfig};
use chrono::Utc;
use clap::Parser;
use futures::StreamExt;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(
    name = "papermill-trace",
    about = "Open Chromium and record network traffic for source investigation."
)]
struct Args {
    /// Label for output filenames.
    label: String,

    /// Optional starting URL.
    #[arg(long)]
    url: Option<String>,

    /// Output directory for trace files.
    #[arg(long, default_value = "tools/traces")]
    out: PathBuf,
}

#[derive(Serialize, Clone)]
struct TraceEntry {
    captured_at: String,
    method: String,
    url: String,
    request_headers: HashMap<String, serde_json::Value>,
    request_body: Option<String>,
    response: Option<TraceResponse>,
}

#[derive(Serialize, Clone)]
struct TraceResponse {
    status: i64,
    status_text: String,
    mime_type: String,
    headers: HashMap<String, serde_json::Value>,
    body: Option<String>,
}

const SKIP_HOSTS: &[&str] = &[
    "google-analytics.com",
    "doubleclick.net",
    "googletagmanager.com",
];

fn should_skip(url: &str) -> bool {
    SKIP_HOSTS.iter().any(|host| url.contains(host))
}

fn is_capturable_body(mime_type: &str) -> bool {
    mime_type.contains("json")
        || mime_type.starts_with("text/")
        || mime_type.contains("xml")
        || mime_type.contains("form-urlencoded")
}

fn headers_to_map(value: &serde_json::Value) -> HashMap<String, serde_json::Value> {
    value
        .as_object()
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let (mut browser, mut browser_handler) = Browser::launch(
        BrowserConfig::builder()
            .with_head()
            .build()
            .map_err(|error| anyhow!("Failed to build browser config: {error}"))?,
    )
    .await
    .context("Failed to launch browser")?;

    let handler_task =
        tokio::spawn(async move { while let Some(_event) = browser_handler.next().await {} });

    let page = browser
        .new_page("about:blank")
        .await
        .context("Failed to open new page")?;

    page.execute(EnableParams::default())
        .await
        .context("Failed to enable Network domain")?;

    let entries: Arc<Mutex<HashMap<String, TraceEntry>>> = Arc::new(Mutex::new(HashMap::new()));
    let order: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let request_task = {
        let entries = Arc::clone(&entries);
        let order = Arc::clone(&order);
        let page = page.clone();

        tokio::spawn(async move {
            let mut events = page
                .event_listener::<EventRequestWillBeSent>()
                .await
                .context("Failed to subscribe to request events")?;

            while let Some(event) = events.next().await {
                let url = event.request.url.clone();

                if should_skip(&url) {
                    continue;
                }

                let request_id = event.request_id.inner().clone();

                let request_body = if event.request.has_post_data == Some(true) {
                    page.execute(GetRequestPostDataParams {
                        request_id: event.request_id.clone(),
                    })
                    .await
                    .ok()
                    .map(|result| result.post_data.clone())
                } else {
                    None
                };

                let entry = TraceEntry {
                    captured_at: Utc::now().to_rfc3339(),
                    method: event.request.method.clone(),
                    url,
                    request_headers: headers_to_map(&event.request.headers.inner().clone()),
                    request_body,
                    response: None,
                };

                let mut entries = entries.lock().await;

                if !entries.contains_key(&request_id) {
                    order.lock().await.push(request_id.clone());
                }

                entries.insert(request_id, entry);
            }

            Ok::<_, anyhow::Error>(())
        })
    };

    let response_task = {
        let entries = Arc::clone(&entries);
        let page = page.clone();

        tokio::spawn(async move {
            let mut events = page
                .event_listener::<EventResponseReceived>()
                .await
                .context("Failed to subscribe to response events")?;

            while let Some(event) = events.next().await {
                let request_id = event.request_id.inner().clone();
                let mime_type = event.response.mime_type.clone();

                let body = if is_capturable_body(&mime_type) {
                    page.execute(GetResponseBodyParams {
                        request_id: event.request_id.clone(),
                    })
                    .await
                    .ok()
                    .map(|result| result.body.clone())
                } else {
                    None
                };

                let response = TraceResponse {
                    status: event.response.status,
                    status_text: event.response.status_text.clone(),
                    mime_type,
                    headers: headers_to_map(&event.response.headers.inner().clone()),
                    body,
                };

                let mut entries = entries.lock().await;

                if let Some(entry) = entries.get_mut(&request_id) {
                    entry.response = Some(response);
                }
            }

            Ok::<_, anyhow::Error>(())
        })
    };

    if let Some(url) = args.url.as_deref() {
        page.goto(url)
            .await
            .context("Failed to navigate to start URL")?;
    }

    println!("[trace] browser open. Interact with the page, then press Enter here to stop.");

    let mut reader = BufReader::new(tokio::io::stdin());
    let mut buf = String::new();
    reader
        .read_line(&mut buf)
        .await
        .context("Failed to read Enter from stdin")?;

    request_task.abort();
    response_task.abort();

    let cookies = page
        .get_cookies()
        .await
        .context("Failed to collect cookies")?;

    let _ = browser.close().await;
    let _ = handler_task.await;

    let entries = entries.lock().await;
    let order = order.lock().await;
    let ordered: Vec<&TraceEntry> = order.iter().filter_map(|id| entries.get(id)).collect();

    fs::create_dir_all(&args.out)
        .with_context(|| format!("Failed to create {}", args.out.display()))?;

    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();

    let trace_path = args.out.join(format!("{}-{stamp}.json", args.label));
    let trace_json = serde_json::to_string_pretty(&ordered).context("Failed to serialize trace")?;
    fs::write(&trace_path, trace_json)
        .with_context(|| format!("Failed to write trace to {}", trace_path.display()))?;

    let cookies_path = args
        .out
        .join(format!("{}-{stamp}.cookies.json", args.label));
    let cookies_json =
        serde_json::to_string_pretty(&cookies).context("Failed to serialize cookies")?;
    fs::write(&cookies_path, cookies_json)
        .with_context(|| format!("Failed to write cookies to {}", cookies_path.display()))?;

    println!("[trace] captured {} requests", ordered.len());
    println!("[trace] trace:   {}", trace_path.display());
    println!("[trace] cookies: {}", cookies_path.display());

    Ok(())
}
