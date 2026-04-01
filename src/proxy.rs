use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, Response, StatusCode},
    routing::any,
    Router,
};
use bytes::{Bytes, BytesMut};
use chrono::Utc;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use std::process::Stdio;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::pricing;
use crate::providers::{self, Provider};
use crate::storage::{Database, RequestLog, Session};

/// Shared state for the proxy
struct ProxyState {
    session_id: String,
    db: Mutex<Database>,
    client: reqwest::Client,
    /// Original API keys captured from the environment
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    /// Suppress all stderr output during the session (for TUI-wrapped apps)
    quiet: bool,
}

pub async fn run_wrap(cmd: Vec<String>, label: Option<String>, quiet: bool) -> Result<()> {
    let session_id = uuid::Uuid::new_v4().to_string();

    // Capture original API keys and base URLs before we override them
    let openai_key = std::env::var("OPENAI_API_KEY").ok();
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").ok();

    let db = Database::open()?;
    db.create_session(&Session {
        session_id: session_id.clone(),
        label: label.clone(),
        started_at: Utc::now(),
        ended_at: None,
        pid: std::process::id(),
    })?;

    let state = Arc::new(ProxyState {
        session_id: session_id.clone(),
        db: Mutex::new(db),
        client: reqwest::Client::new(),
        openai_api_key: openai_key,
        anthropic_api_key: anthropic_key,
        quiet,
    });

    // Start proxy on a random port
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let app = Router::new()
        .route("/{*path}", any(proxy_handler))
        .with_state(state.clone());

    // Spawn the proxy server
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let proxy_url = format!("http://{}", addr);
    if !quiet {
        let display_label = label.as_deref().unwrap_or(&cmd[0]);
        eprintln!(
            "\x1b[36m┌─ tokmon\x1b[0m tracking LLM calls for: \x1b[1m{}\x1b[0m",
            display_label
        );
        eprintln!(
            "\x1b[36m│\x1b[0m proxy: {} | session: {}",
            proxy_url,
            &session_id[..8]
        );
        eprintln!("\x1b[36m└─\x1b[0m");
    }

    // Run the wrapped command with proxy env vars
    let status = tokio::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .env("OPENAI_BASE_URL", format!("{}/openai", proxy_url))
        .env("OPENAI_API_BASE", format!("{}/openai", proxy_url))
        .env("ANTHROPIC_BASE_URL", format!("{}/anthropic", proxy_url))
        .env("TOKMON_SESSION", &session_id)
        .env("TOKMON_PROXY", &proxy_url)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .with_context(|| format!("Failed to execute: {}", cmd[0]))?;

    // Shut down proxy
    server_handle.abort();

    // End session and print summary
    let db = Database::open()?;
    db.end_session(&session_id)?;
    let summary = db.get_session_summary(&session_id)?;

    eprintln!();
    eprintln!("\x1b[36m┌─ tokmon\x1b[0m session complete");
    eprintln!(
        "\x1b[36m│\x1b[0m requests: {}  tokens: {} in / {} out",
        summary.request_count,
        pricing::format_tokens(summary.total_input_tokens),
        pricing::format_tokens(summary.total_output_tokens),
    );
    if summary.total_cached_tokens > 0 {
        eprintln!(
            "\x1b[36m│\x1b[0m cached: {}",
            pricing::format_tokens(summary.total_cached_tokens),
        );
    }
    eprintln!(
        "\x1b[36m│\x1b[0m cost: \x1b[1;33m{}\x1b[0m  avg latency: {:.0}ms",
        pricing::format_cost(summary.total_cost),
        summary.avg_latency_ms,
    );

    // Budget check
    if let Ok(Some(budget)) = db.get_budget() {
        if let Ok(today_cost) = db.get_today_cost() {
            let pct = (today_cost / budget) * 100.0;
            if today_cost > budget {
                eprintln!(
                    "\x1b[36m│\x1b[0m \x1b[1;31m⚠ OVER BUDGET\x1b[0m today: {} / {} ({:.0}%)",
                    pricing::format_cost(today_cost),
                    pricing::format_cost(budget),
                    pct,
                );
            } else if pct > 80.0 {
                eprintln!(
                    "\x1b[36m│\x1b[0m \x1b[1;33m⚠ budget warning\x1b[0m today: {} / {} ({:.0}%)",
                    pricing::format_cost(today_cost),
                    pricing::format_cost(budget),
                    pct,
                );
            }
        }
    }

    eprintln!("\x1b[36m└─\x1b[0m");

    // Exit with the same code as the wrapped process
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

async fn proxy_handler(
    State(state): State<Arc<ProxyState>>,
    req: Request<Body>,
) -> Result<Response<Body>, StatusCode> {
    let start = std::time::Instant::now();

    // Determine target provider from the path prefix
    let path = req.uri().path();
    let (provider, target_host, api_path) = if path.starts_with("/openai") {
        let api_path = path.strip_prefix("/openai").unwrap_or("/");
        (
            Provider::OpenAI,
            "https://api.openai.com",
            api_path.to_string(),
        )
    } else if path.starts_with("/anthropic") {
        let api_path = path.strip_prefix("/anthropic").unwrap_or("/");
        (
            Provider::Anthropic,
            "https://api.anthropic.com",
            api_path.to_string(),
        )
    } else {
        return Err(StatusCode::BAD_GATEWAY);
    };

    let method = req.method().clone();
    let headers = req.headers().clone();
    let accept_sse = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false);

    // Read request body
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_bytes();

    let request_model = providers::extract_model_from_request(&body_bytes);
    let body_is_stream = providers::is_stream_request(&body_bytes);
    let is_stream = accept_sse || body_is_stream;

    // Build target URL
    let target_url = format!(
        "{}{}",
        target_host,
        if api_path.is_empty() { "/" } else { &api_path }
    );

    // Build forwarded request
    let mut forward = state.client.request(method, &target_url);

    // Copy headers, replacing auth as needed
    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        // Skip hop-by-hop headers, host, and accept-encoding
        // (we need uncompressed responses from upstream for SSE parsing)
        if matches!(
            name_str.as_str(),
            "host" | "connection" | "transfer-encoding" | "content-length" | "accept-encoding"
        ) {
            continue;
        }

        // Inject the real API key
        if name_str == "authorization" {
            if provider == Provider::OpenAI {
                if let Some(ref key) = state.openai_api_key {
                    forward = forward.header("authorization", format!("Bearer {}", key));
                    continue;
                }
            }
        }
        if name_str == "x-api-key" {
            if provider == Provider::Anthropic {
                if let Some(ref key) = state.anthropic_api_key {
                    forward = forward.header("x-api-key", key.as_str());
                    continue;
                }
            }
        }

        forward = forward.header(name.clone(), value.clone());
    }

    // Explicitly request uncompressed responses — reqwest doesn't have
    // the gzip feature enabled so we cannot decode compressed bodies.
    // Without this, the server is free to compress and our parsers silently fail.
    forward = forward.header("accept-encoding", "identity");

    // For OpenAI streaming, inject stream_options.include_usage
    if is_stream && provider == Provider::OpenAI {
        if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
            if json.get("stream_options").is_none() {
                json["stream_options"] = serde_json::json!({"include_usage": true});
            }
            forward = forward.json(&json);
        } else {
            forward = forward.body(body_bytes.to_vec());
        }
    } else {
        forward = forward.body(body_bytes.to_vec());
    }

    // Send the request
    let response = forward.send().await.map_err(|e| {
        eprintln!("\x1b[36mtokmon\x1b[0m proxy error: {}", e);
        StatusCode::BAD_GATEWAY
    })?;

    let status = response.status();
    let resp_headers = response.headers().clone();

    if is_stream && status.is_success() {
        handle_streaming_response(
            state,
            provider,
            request_model,
            target_url,
            response,
            start,
            status,
            resp_headers,
        )
        .await
    } else {
        handle_non_streaming_response(
            state, provider, request_model, &target_url, response, start, status, resp_headers,
        )
        .await
    }
}

async fn handle_non_streaming_response(
    state: Arc<ProxyState>,
    provider: Provider,
    request_model: Option<String>,
    endpoint: &str,
    response: reqwest::Response,
    start: std::time::Instant,
    status: reqwest::StatusCode,
    resp_headers: HeaderMap,
) -> Result<Response<Body>, StatusCode> {
    let resp_bytes = response
        .bytes()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let latency = start.elapsed().as_millis() as i64;

    let mut usage = match provider {
        Provider::OpenAI => providers::openai::parse_response(&resp_bytes),
        Provider::Anthropic => providers::anthropic::parse_response(&resp_bytes),
    };

    // Fallback: try text extraction if JSON parsing failed
    if usage.is_none() && !resp_bytes.is_empty() {
        let text = String::from_utf8_lossy(&resp_bytes);

        // Maybe this was actually a streaming response handled as non-streaming
        if text.contains("data: ") || text.contains("data:") {
            let chunks: Vec<String> = text
                .lines()
                .filter_map(|line| {
                    providers::strip_sse_data_prefix(line).map(|data| data.to_string())
                })
                .collect();
            usage = match provider {
                Provider::OpenAI => providers::openai::parse_stream_chunks(&chunks),
                Provider::Anthropic => providers::anthropic::parse_stream_chunks(&chunks),
            };
        }

        if usage.is_none() {
            usage = providers::parse_usage_from_text(&text);
        }

        if std::env::var("TOKMON_DEBUG").is_ok() {
            let preview: String = text.chars().take(500).collect();
            eprintln!(
                "\x1b[36mtokmon:debug\x1b[0m non-stream parse failed for {}, preview:\n{}",
                provider.as_str(),
                preview
            );
        }
    }

    log_usage(
        &state,
        provider,
        request_model,
        endpoint,
        latency,
        status.as_u16(),
        false,
        usage,
    )
    .await;

    build_response(status, &resp_headers, resp_bytes)
}

/// Stream the response through to the caller in real-time while tapping
/// each chunk to collect usage data for logging after the stream ends.
async fn handle_streaming_response(
    state: Arc<ProxyState>,
    provider: Provider,
    request_model: Option<String>,
    endpoint: String,
    response: reqwest::Response,
    start: std::time::Instant,
    status: reqwest::StatusCode,
    resp_headers: HeaderMap,
) -> Result<Response<Body>, StatusCode> {
    // Channel: proxy reads from upstream and sends chunks into tx.
    // The axum response body reads from rx, forwarding to the client in real-time.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(32);

    // Spawn a task that reads the upstream response stream, forwards each chunk
    // through the channel, and collects SSE lines for usage parsing.
    let state_clone = state.clone();
    tokio::spawn(async move {
        let mut stream = response.bytes_stream();
        let mut collected = BytesMut::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    collected.extend_from_slice(&chunk);
                    if tx.send(Ok(chunk)).await.is_err() {
                        // Client disconnected
                        break;
                    }
                }
                Err(e) => {
                    let _ = tx
                        .send(Err(std::io::Error::new(std::io::ErrorKind::Other, e)))
                        .await;
                    break;
                }
            }
        }
        // tx is dropped here, closing the channel and ending the response body.

        let latency = start.elapsed().as_millis() as i64;

        // Parse usage from the collected SSE data
        let text = String::from_utf8_lossy(&collected);
        let chunks: Vec<String> = text
            .lines()
            .filter_map(|line| {
                providers::strip_sse_data_prefix(line).map(|data| data.to_string())
            })
            .collect();

        let is_debug = std::env::var("TOKMON_DEBUG").is_ok();
        if is_debug {
            eprintln!(
                "\x1b[36mtokmon:debug\x1b[0m {} stream: {} bytes, {} data lines",
                provider.as_str(),
                collected.len(),
                chunks.len(),
            );
            if chunks.is_empty() && !collected.is_empty() {
                // Show first 500 chars of raw response for diagnosis
                let preview: String = text.chars().take(500).collect();
                eprintln!("\x1b[36mtokmon:debug\x1b[0m raw response preview:\n{}", preview);
            }
        }

        let mut usage = match provider {
            Provider::OpenAI => providers::openai::parse_stream_chunks(&chunks),
            Provider::Anthropic => providers::anthropic::parse_stream_chunks(&chunks),
        };

        // Fallback: if structured parsing failed, try text extraction
        if usage.is_none() && !collected.is_empty() {
            if is_debug {
                eprintln!("\x1b[36mtokmon:debug\x1b[0m structured parse failed, trying text fallback");
            }
            usage = providers::parse_usage_from_text(&text);
        }

        log_usage(
            &state_clone,
            provider,
            request_model,
            &endpoint,
            latency,
            status.as_u16(),
            true,
            usage,
        )
        .await;
    });

    // Build the streaming response using a ReceiverStream
    let body_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let body = Body::from_stream(body_stream);

    let mut builder = Response::builder().status(status.as_u16());
    for (name, value) in resp_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        // Keep transfer-encoding and other streaming headers intact
        if matches!(name_str.as_str(), "connection") {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }

    builder
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn log_usage(
    state: &ProxyState,
    provider: Provider,
    request_model: Option<String>,
    endpoint: &str,
    latency_ms: i64,
    status_code: u16,
    is_stream: bool,
    usage: Option<providers::UsageInfo>,
) {
    let model = usage
        .as_ref()
        .map(|u| u.model.clone())
        .filter(|m| !m.is_empty())
        .or(request_model)
        .unwrap_or_else(|| "unknown".to_string());

    let (input_tokens, output_tokens, cached_tokens) = usage
        .as_ref()
        .map(|u| (u.input_tokens, u.output_tokens, u.cached_tokens))
        .unwrap_or((0, 0, 0));

    let cost = pricing::calculate_cost(
        provider.as_str(),
        &model,
        input_tokens,
        output_tokens,
        cached_tokens,
    );

    if !state.quiet {
        eprintln!(
            "\x1b[36mtokmon\x1b[0m {} \x1b[33m{}\x1b[0m {}in/{}out {} {:.0}ms",
            provider.as_str(),
            model,
            pricing::format_tokens(input_tokens),
            pricing::format_tokens(output_tokens),
            pricing::format_cost(cost),
            latency_ms,
        );
    }

    let log = RequestLog {
        id: None,
        session_id: state.session_id.clone(),
        timestamp: Utc::now(),
        provider: provider.as_str().to_string(),
        model,
        endpoint: endpoint.to_string(),
        input_tokens,
        output_tokens,
        cached_tokens,
        latency_ms,
        cost_usd: cost,
        status_code,
        is_stream,
    };

    let db = state.db.lock().await;
    if let Err(e) = db.log_request(&log) {
        eprintln!("\x1b[36mtokmon\x1b[0m failed to log request: {}", e);
    }
}

fn build_response(
    status: reqwest::StatusCode,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response<Body>, StatusCode> {
    let mut builder = Response::builder().status(status.as_u16());

    for (name, value) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if matches!(
            name_str.as_str(),
            "transfer-encoding" | "content-length" | "connection"
        ) {
            continue;
        }
        builder = builder.header(name.clone(), value.clone());
    }

    builder
        .body(Body::from(body))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
