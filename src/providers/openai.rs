use super::UsageInfo;
use serde::Deserialize;

/// Parse token usage from an OpenAI non-streaming response body.
pub fn parse_response(body: &[u8]) -> Option<UsageInfo> {
    let resp: OpenAIResponse = serde_json::from_slice(body).ok()?;
    let usage = resp.usage?;

    Some(UsageInfo {
        model: resp.model.unwrap_or_default(),
        input_tokens: usage.prompt_tokens.unwrap_or(0),
        output_tokens: usage.completion_tokens.unwrap_or(0),
        cached_tokens: usage
            .prompt_tokens_details
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0),
    })
}

/// Parse token usage from accumulated SSE chunks for OpenAI streaming.
/// The final chunk with `usage` is what we want.
pub fn parse_stream_chunks(chunks: &[String]) -> Option<UsageInfo> {
    let mut last_usage: Option<UsageInfo> = None;
    let mut model = String::new();

    for chunk in chunks {
        // Accept both pre-stripped and prefixed formats for robustness
        let data = chunk
            .strip_prefix("data: ")
            .or_else(|| chunk.strip_prefix("data:"))
            .unwrap_or(chunk);

        if data.trim() == "[DONE]" {
            continue;
        }

        if let Ok(parsed) = serde_json::from_str::<OpenAIStreamChunk>(data) {
            if let Some(m) = &parsed.model {
                if !m.is_empty() {
                    model = m.clone();
                }
            }
            if let Some(usage) = parsed.usage {
                last_usage = Some(UsageInfo {
                    model: model.clone(),
                    input_tokens: usage.prompt_tokens.unwrap_or(0),
                    output_tokens: usage.completion_tokens.unwrap_or(0),
                    cached_tokens: usage
                        .prompt_tokens_details
                        .and_then(|d| d.cached_tokens)
                        .unwrap_or(0),
                });
            }
        }
    }

    if let Some(ref mut u) = last_usage {
        if u.model.is_empty() {
            u.model = model;
        }
    }

    last_usage
}

#[derive(Deserialize)]
struct OpenAIResponse {
    model: Option<String>,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIStreamChunk {
    model: Option<String>,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    prompt_tokens_details: Option<PromptTokenDetails>,
}

#[derive(Deserialize)]
struct PromptTokenDetails {
    cached_tokens: Option<i64>,
}
