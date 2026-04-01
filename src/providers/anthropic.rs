use super::UsageInfo;
use serde::Deserialize;

/// Parse token usage from an Anthropic non-streaming response body.
pub fn parse_response(body: &[u8]) -> Option<UsageInfo> {
    let resp: AnthropicResponse = serde_json::from_slice(body).ok()?;
    let usage = resp.usage?;

    Some(UsageInfo {
        model: resp.model.unwrap_or_default(),
        input_tokens: usage.input_tokens.unwrap_or(0),
        output_tokens: usage.output_tokens.unwrap_or(0),
        cached_tokens: usage.cache_read_input_tokens.unwrap_or(0)
            + usage.cache_creation_input_tokens.unwrap_or(0),
    })
}

/// Parse token usage from accumulated SSE chunks for Anthropic streaming.
/// Chunks should be pre-stripped JSON strings (no "data: " prefix).
pub fn parse_stream_chunks(chunks: &[String]) -> Option<UsageInfo> {
    let mut model = String::new();
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut cached_tokens: i64 = 0;

    for chunk in chunks {
        // Accept both pre-stripped and prefixed formats for robustness
        let data = chunk
            .strip_prefix("data: ")
            .or_else(|| chunk.strip_prefix("data:"))
            .unwrap_or(chunk);

        if let Ok(event) = serde_json::from_str::<AnthropicStreamEvent>(data) {
            match event.r#type.as_deref() {
                Some("message_start") => {
                    if let Some(msg) = event.message {
                        if let Some(m) = msg.model {
                            model = m;
                        }
                        if let Some(usage) = msg.usage {
                            input_tokens = usage.input_tokens.unwrap_or(0);
                            cached_tokens = usage.cache_read_input_tokens.unwrap_or(0)
                                + usage.cache_creation_input_tokens.unwrap_or(0);
                        }
                    }
                }
                Some("message_delta") => {
                    if let Some(usage) = event.usage {
                        output_tokens = usage.output_tokens.unwrap_or(0);
                    }
                }
                _ => {}
            }
        }
    }

    if input_tokens > 0 || output_tokens > 0 {
        Some(UsageInfo {
            model,
            input_tokens,
            output_tokens,
            cached_tokens,
        })
    } else {
        None
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    model: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicStreamEvent {
    r#type: Option<String>,
    message: Option<AnthropicMessage>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicMessage {
    model: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
}
