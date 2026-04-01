pub mod anthropic;
pub mod openai;

use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct UsageInfo {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
}

/// Strip the SSE `data:` prefix from a line (handles both "data: " and "data:").
pub fn strip_sse_data_prefix(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    trimmed
        .strip_prefix("data: ")
        .or_else(|| trimmed.strip_prefix("data:"))
}

/// Fallback: extract usage from raw response text using string matching.
/// Useful when structured SSE parsing fails.
pub fn parse_usage_from_text(text: &str) -> Option<UsageInfo> {
    fn extract_i64(text: &str, key: &str) -> Option<i64> {
        let pattern = format!("\"{}\"", key);
        let idx = text.find(&pattern)?;
        let after = &text[idx + pattern.len()..];
        // skip optional whitespace and colon
        let after = after.trim_start().strip_prefix(':')?.trim_start();
        // parse number
        let end = after.find(|c: char| !c.is_ascii_digit()).unwrap_or(after.len());
        after[..end].parse().ok()
    }

    let input = extract_i64(text, "input_tokens").unwrap_or(0);
    let output = extract_i64(text, "output_tokens").unwrap_or(0);
    let cached = extract_i64(text, "cache_read_input_tokens").unwrap_or(0)
        + extract_i64(text, "cache_creation_input_tokens").unwrap_or(0);

    // For OpenAI, try prompt_tokens / completion_tokens
    let input = if input == 0 {
        extract_i64(text, "prompt_tokens").unwrap_or(0)
    } else {
        input
    };
    let output = if output == 0 {
        extract_i64(text, "completion_tokens").unwrap_or(0)
    } else {
        output
    };

    if input > 0 || output > 0 {
        // Try to find model
        let model = extract_string(text, "model").unwrap_or_default();
        Some(UsageInfo {
            model,
            input_tokens: input,
            output_tokens: output,
            cached_tokens: cached,
        })
    } else {
        None
    }
}

fn extract_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{}\"", key);
    let idx = text.find(&pattern)?;
    let after = &text[idx + pattern.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Anthropic,
    DeepSeek,
    Mistral,
    Groq,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenAI => "openai",
            Provider::Anthropic => "anthropic",
            Provider::DeepSeek => "deepseek",
            Provider::Mistral => "mistral",
            Provider::Groq => "groq",
        }
    }

    /// Whether this provider uses the OpenAI-compatible API format.
    pub fn is_openai_compatible(&self) -> bool {
        matches!(
            self,
            Provider::OpenAI | Provider::DeepSeek | Provider::Mistral | Provider::Groq
        )
    }
}

/// Detect provider from the original target host
#[allow(dead_code)]
pub fn detect_provider(host: &str) -> Option<Provider> {
    if host.contains("openai") {
        Some(Provider::OpenAI)
    } else if host.contains("anthropic") {
        Some(Provider::Anthropic)
    } else {
        None
    }
}

/// Try to extract model from the request body
#[derive(Deserialize)]
struct ModelField {
    model: Option<String>,
}

pub fn extract_model_from_request(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<ModelField>(body)
        .ok()
        .and_then(|m| m.model)
}

/// Check if a request is SSE streaming
#[derive(Deserialize)]
struct StreamField {
    stream: Option<bool>,
}

pub fn is_stream_request(body: &[u8]) -> bool {
    serde_json::from_slice::<StreamField>(body)
        .ok()
        .and_then(|s| s.stream)
        .unwrap_or(false)
}
