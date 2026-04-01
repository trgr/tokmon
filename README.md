# tokmon

htop for your AI spend. Track tokens, latency and cost across LLM providers.

tokmon runs a local proxy that sits between your code and the API. It intercepts requests, logs usage to a local SQLite database, and gives you a live dashboard and CLI reports. Your code doesn't need any changes. Just wrap the command.

## Install

```
brew tap trgr/tokmon https://github.com/trgr/tokmon
brew install trgr/tokmon/tokmon
```

Or build from source:

```
cargo install --path .
```

## Quick start

```
tokmon wrap -- python my_script.py
```

That's it. tokmon captures your API keys, starts a local proxy, and routes your process through it. When the process finishes, you get a summary:

```
┌─ tokmon session complete
│ requests: 42  tokens: 15.2k in / 3.1k out
│ cost: $0.23  avg latency: 847ms
└─
```

## Commands

### `tokmon wrap <command>`

Wrap any command and track its LLM API calls.

```
tokmon wrap -- node index.js
tokmon wrap --label "batch-job" -- python pipeline.py
tokmon wrap --quiet -- claude   # suppress per-request output (useful for TUI apps)
```

### `tokmon status`

Open a live terminal dashboard. Shows today's cost, 7-day total, per-model breakdown and recent requests. Refreshes every 2 seconds.

### `tokmon report`

Print cost and usage reports.

```
tokmon report                          # today, grouped by session
tokmon report --range 7d --group-by model
tokmon report --range 30d --group-by provider
tokmon report --range all
```

### `tokmon budget <amount>`

Set a daily spending alert in USD. The dashboard turns yellow at 80% and red when you go over.

```
tokmon budget 5.00
```

### `tokmon log`

Show recent requests with token counts, cost and latency.

```
tokmon log --count 50
```

### `tokmon update-prices`

Fetch the latest model pricing from the repo. Prices are also fetched automatically on first run and refreshed every 24 hours.

```
tokmon update-prices          # fetch latest prices
tokmon update-prices --show   # display current price table
```

## Supported providers

- **OpenAI** (GPT-4o, GPT-4, o1, o3, o4-mini, ...)
- **Anthropic** (Claude Opus, Sonnet, Haiku)
- **DeepSeek** (DeepSeek-V3, DeepSeek-R1)
- **Mistral** (Mistral Large, Codestral, ...)
- **Groq** (Llama, Mixtral, Gemma, ...)

## How it works

1. tokmon captures your `OPENAI_API_KEY` and `ANTHROPIC_API_KEY` from the environment
2. It starts an HTTP proxy on a random localhost port
3. It sets `OPENAI_BASE_URL` and `ANTHROPIC_BASE_URL` to point at the proxy
4. Your process makes API calls as normal, but they go through tokmon
5. tokmon forwards each request to the real API (injecting the real key), parses the response for token usage, and logs everything to SQLite
6. For streaming responses, chunks are forwarded in real time while tokmon collects usage data in the background

All data is stored locally at `~/.local/share/tokmon/tokmon.db` (macOS: `~/Library/Application Support/tokmon/`).

## Pricing

Model prices are fetched from this repo and cached locally. You can also edit the prices file directly:

```
~/Library/Application Support/tokmon/prices.json   # macOS
~/.local/share/tokmon/prices.json                   # Linux
```

Models are matched by longest prefix, so `gpt-4o-mini` correctly takes priority over `gpt-4o`.

## Environment variables

| Variable | Description |
|---|---|
| `TOKMON_VERBOSE` | Set to `1` to print per-request details to stderr |
| `TOKMON_DEBUG` | Set to `1` for detailed proxy debug output |

## License

MIT
