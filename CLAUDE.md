# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is tokmon

tokmon is "htop for your AI spend" — a Rust CLI tool that tracks tokens, latency, and cost across LLM providers (Anthropic, OpenAI). It works by running a local HTTP proxy that intercepts API calls from a wrapped process, parses usage from responses, logs to SQLite, and displays results in a live TUI or CLI reports.

## Build Commands

```bash
cargo build --release        # optimized binary (lto + strip enabled)
cargo build                  # debug build
cargo check                  # type-check without building
cargo clippy                 # lint
cargo test                   # run tests (none currently exist)
```

The release binary is at `target/release/tokmon`.

## Architecture

**Core data flow:** Wrapped process → local proxy → upstream API → parse response → log to SQLite → display in TUI/reports.

### Proxy (`proxy.rs`)
The central component. An axum HTTP server on a random localhost port. Routes by path prefix: `/openai` → api.openai.com, `/anthropic` → api.anthropic.com. Captures real API keys from env before overriding them for the wrapped process. Handles both streaming (SSE via channel-based forwarding with background chunk collection) and non-streaming responses. After parsing usage, logs to SQLite via `storage`.

### Providers (`providers/`)
Each provider module (`openai.rs`, `anthropic.rs`) implements two parsers: `parse_response` for non-streaming JSON and `parse_stream_chunks` for SSE. Both return `UsageInfo { model, input_tokens, output_tokens, cached_tokens }`. The `mod.rs` has fallback text extraction (`parse_usage_from_text`) for when structured parsing fails.

### Storage (`storage.rs`)
SQLite at `~/.local/share/tokmon/tokmon.db`. Three tables: `sessions`, `requests`, `config` (for budget). Provides queries for session summaries, daily/weekly aggregates, model breakdowns, and budget management.

### Pricing (`pricing.rs`)
Hardcoded per-1M-token rates for 20+ models with user override from `~/.local/share/tokmon/prices.json`. Uses prefix/substring matching for model name lookup. Cost = (tokens / 1M) × rate, with separate rates for input, output, and cached tokens.

### TUI (`tui.rs`)
Live ratatui dashboard refreshing every 2 seconds. Shows today/7-day costs, model breakdown table, recent 30 requests, and budget status (green/yellow at 80%/red over).

### CLI (`main.rs`)
Clap-derived subcommands: `wrap` (proxy a command), `status` (TUI), `report` (CLI reports with time ranges and grouping), `budget` (set daily limit), `log` (recent requests), `update-prices` (manage pricing config).

## Key Design Decisions

- API key injection: real keys captured at startup, proxy swaps dummy keys back to real ones in forwarded requests
- Streaming: uses `tokio::sync::mpsc::channel(32)` to forward chunks to client in real-time while accumulating SSE data for post-stream usage parsing
- For OpenAI streaming, the proxy injects `stream_options.include_usage` into the request body to ensure usage data is included in the stream
- `Accept-Encoding` is stripped from forwarded requests to force uncompressed upstream responses (reqwest lacks gzip feature, so compressed SSE would be unparseable)
