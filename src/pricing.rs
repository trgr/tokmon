use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const REMOTE_PRICES_URL: &str =
    "https://raw.githubusercontent.com/trgr/tokmon/main/prices.json";

/// Ensure a local prices.json exists and is reasonably fresh (< 24h).
/// Fetches from the remote repo on first run or when stale.
/// Failures are silent — hardcoded defaults are always available as fallback.
pub fn ensure_prices() {
    let path = match prices_path() {
        Ok(p) => p,
        Err(_) => return,
    };

    let needs_fetch = if path.exists() {
        // Stale if older than 24 hours
        std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .map(|mtime| {
                mtime.elapsed().unwrap_or_default() > std::time::Duration::from_secs(86400)
            })
            .unwrap_or(true)
    } else {
        true
    };

    if needs_fetch {
        fetch_remote_prices(&path);
    }
}

/// Force-fetch prices from remote, ignoring cache age.
pub fn force_fetch_prices() -> Result<()> {
    let path = prices_path()?;
    if fetch_remote_prices(&path) {
        println!("Prices updated from remote: {}", path.display());
    } else {
        // If fetch failed but no local file exists, write defaults
        if !path.exists() {
            let config = default_price_config();
            let json = serde_json::to_string_pretty(&config)?;
            std::fs::write(&path, json)?;
            println!("Wrote default prices to: {}", path.display());
        } else {
            println!("Fetch failed, keeping existing prices at: {}", path.display());
        }
    }
    println!("Edit this file to customize pricing. Models are matched by prefix/contains.");
    Ok(())
}

/// Fetch prices from the remote URL. Returns true on success.
fn fetch_remote_prices(path: &PathBuf) -> bool {
    // Use a short timeout so we never block the user for long
    let result = std::thread::spawn({
        let path = path.clone();
        move || -> bool {
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .ok();
            let client = match client {
                Some(c) => c,
                None => return false,
            };
            let resp = match client.get(REMOTE_PRICES_URL).send() {
                Ok(r) if r.status().is_success() => r,
                _ => return false,
            };
            let body = match resp.text() {
                Ok(t) => t,
                Err(_) => return false,
            };
            // Validate it's valid PriceConfig JSON before writing
            if serde_json::from_str::<PriceConfig>(&body).is_err() {
                return false;
            }
            std::fs::write(&path, &body).is_ok()
        }
    })
    .join()
    .unwrap_or(false);

    result
}

/// A single model's pricing per 1M tokens in USD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cached: f64,
}

/// Full pricing config file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceConfig {
    /// Map from model pattern to price. Patterns use prefix/contains matching.
    pub models: BTreeMap<String, ModelPrice>,
}

fn prices_path() -> Result<PathBuf> {
    let data_dir = dirs::data_dir()
        .context("Could not determine data directory")?
        .join("tokmon");
    std::fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("prices.json"))
}

/// Load user price overrides from ~/.local/share/tokmon/prices.json
fn load_price_config() -> Option<PriceConfig> {
    let path = prices_path().ok()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Try to find a price in the user config file.
/// Uses longest matching pattern to avoid e.g. "gpt-4" matching before "gpt-4o-mini".
fn lookup_user_price(model: &str) -> Option<(f64, f64, f64)> {
    let config = load_price_config()?;
    // Try exact match first
    if let Some(p) = config.models.get(model) {
        return Some((p.input, p.output, p.cached));
    }
    // Find the longest matching pattern (prefix or contains)
    let mut best: Option<(&str, &ModelPrice)> = None;
    for (pattern, price) in &config.models {
        if model.starts_with(pattern.as_str()) || model.contains(pattern.as_str()) {
            if best.is_none() || pattern.len() > best.unwrap().0.len() {
                best = Some((pattern.as_str(), price));
            }
        }
    }
    best.map(|(_, p)| (p.input, p.output, p.cached))
}

/// Token pricing per 1M tokens in USD.
/// Checks user config file first, then falls back to hardcoded defaults.
///
/// Pricing: (input_per_1m, output_per_1m, cached_input_per_1m)
pub fn get_price(provider: &str, model: &str) -> (f64, f64, f64) {
    // Check user overrides first
    if let Some(price) = lookup_user_price(model) {
        return price;
    }

    hardcoded_price(provider, model)
}

fn hardcoded_price(_provider: &str, model: &str) -> (f64, f64, f64) {
    match model {
        // OpenAI
        m if m.starts_with("gpt-4o-mini") => (0.15, 0.60, 0.075),
        m if m.starts_with("gpt-4o") => (2.50, 10.00, 1.25),
        m if m.starts_with("gpt-4-turbo") => (10.00, 30.00, 5.00),
        m if m.starts_with("gpt-4") => (30.00, 60.00, 15.00),
        m if m.starts_with("gpt-3.5") => (0.50, 1.50, 0.25),
        m if m.starts_with("o1-mini") => (3.00, 12.00, 1.50),
        m if m.starts_with("o1-pro") => (150.00, 600.00, 75.00),
        m if m.starts_with("o1") => (15.00, 60.00, 7.50),
        m if m.starts_with("o3-mini") => (1.10, 4.40, 0.55),
        m if m.starts_with("o3") => (10.00, 40.00, 5.00),
        m if m.starts_with("o4-mini") => (1.10, 4.40, 0.55),

        // Anthropic
        m if m.contains("claude-3-5-haiku") || m.contains("claude-haiku-4") => {
            (0.80, 4.00, 0.08)
        }
        m if m.contains("claude-3-5-sonnet") || m.contains("claude-sonnet-4") => {
            (3.00, 15.00, 0.30)
        }
        m if m.contains("claude-3-opus") || m.contains("claude-opus-4") => {
            (15.00, 75.00, 1.50)
        }
        m if m.contains("claude") => (3.00, 15.00, 0.30), // default to sonnet-tier

        // Google Gemini (via OpenAI-compatible API)
        m if m.contains("gemini-2.0-flash") => (0.10, 0.40, 0.025),
        m if m.contains("gemini-1.5-pro") => (1.25, 5.00, 0.3125),
        m if m.contains("gemini-1.5-flash") => (0.075, 0.30, 0.01875),

        // DeepSeek
        m if m.contains("deepseek-chat") || m.contains("deepseek-v3") => {
            (0.27, 1.10, 0.07)
        }
        m if m.contains("deepseek-reasoner") || m.contains("deepseek-r1") => {
            (0.55, 2.19, 0.14)
        }

        // Fallback — reasonable mid-tier estimate
        _ => (3.00, 15.00, 1.50),
    }
}

/// Build the default price config from hardcoded values.
fn default_price_config() -> PriceConfig {
    let mut models = BTreeMap::new();

    let entries = [
        // OpenAI
        ("gpt-4o-mini", (0.15, 0.60, 0.075)),
        ("gpt-4o", (2.50, 10.00, 1.25)),
        ("gpt-4-turbo", (10.00, 30.00, 5.00)),
        ("gpt-4", (30.00, 60.00, 15.00)),
        ("gpt-3.5", (0.50, 1.50, 0.25)),
        ("o1-mini", (3.00, 12.00, 1.50)),
        ("o1-pro", (150.00, 600.00, 75.00)),
        ("o1", (15.00, 60.00, 7.50)),
        ("o3-mini", (1.10, 4.40, 0.55)),
        ("o3", (10.00, 40.00, 5.00)),
        ("o4-mini", (1.10, 4.40, 0.55)),
        // Anthropic
        ("claude-haiku-4", (0.80, 4.00, 0.08)),
        ("claude-3-5-haiku", (0.80, 4.00, 0.08)),
        ("claude-sonnet-4", (3.00, 15.00, 0.30)),
        ("claude-3-5-sonnet", (3.00, 15.00, 0.30)),
        ("claude-opus-4", (15.00, 75.00, 1.50)),
        ("claude-3-opus", (15.00, 75.00, 1.50)),
        // Gemini
        ("gemini-2.0-flash", (0.10, 0.40, 0.025)),
        ("gemini-1.5-pro", (1.25, 5.00, 0.3125)),
        ("gemini-1.5-flash", (0.075, 0.30, 0.01875)),
        // DeepSeek
        ("deepseek-chat", (0.27, 1.10, 0.07)),
        ("deepseek-v3", (0.27, 1.10, 0.07)),
        ("deepseek-reasoner", (0.55, 2.19, 0.14)),
        ("deepseek-r1", (0.55, 2.19, 0.14)),
    ];

    for (pattern, (input, output, cached)) in entries {
        models.insert(
            pattern.to_string(),
            ModelPrice {
                input,
                output,
                cached,
            },
        );
    }

    PriceConfig { models }
}

/// Fetch latest prices from remote, or write defaults if offline.
pub fn update_prices() -> Result<()> {
    force_fetch_prices()
}

/// Print current effective prices.
pub fn show_prices() {
    println!();
    println!("  \x1b[1mtokmon prices\x1b[0m — per 1M tokens (USD)");
    println!();
    println!(
        "  \x1b[90m{:<25} {:>10} {:>10} {:>10}\x1b[0m",
        "MODEL", "INPUT", "OUTPUT", "CACHED"
    );
    println!("  {}", "\x1b[90m─\x1b[0m".repeat(58));

    let config = load_price_config();
    let display_config = config.unwrap_or_else(default_price_config);

    for (pattern, price) in &display_config.models {
        println!(
            "  {:<25} {:>10} {:>10} {:>10}",
            pattern,
            format!("${:.4}", price.input),
            format!("${:.4}", price.output),
            format!("${:.4}", price.cached),
        );
    }

    println!();
    println!("  \x1b[90mFallback (unknown models): $3.00 / $15.00 / $1.50\x1b[0m");
    if let Ok(path) = prices_path() {
        if path.exists() {
            println!("  \x1b[90mConfig: {}\x1b[0m", path.display());
        } else {
            println!("  \x1b[90mUsing hardcoded defaults. Run `tokmon update-prices` to create config.\x1b[0m");
        }
    }
    println!();
}

pub fn calculate_cost(
    provider: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
) -> f64 {
    let (input_price, output_price, cached_price) = get_price(provider, model);

    let input_cost = (input_tokens as f64 / 1_000_000.0) * input_price;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * output_price;
    let cached_cost = (cached_tokens as f64 / 1_000_000.0) * cached_price;

    input_cost + output_cost + cached_cost
}

pub fn format_cost(cost: f64) -> String {
    if cost < 0.01 {
        format!("${:.4}", cost)
    } else if cost < 1.0 {
        format!("${:.3}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

pub fn format_tokens(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}
