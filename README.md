# tailgate

tailgate is a personal OpenAI-compatible AI gateway. It gives Codex, Cursor, OpenAI SDK clients, and local agents one private `base_url` while keeping provider keys on your server.

It is designed for an always-on AWS Lightsail Singapore server inside a Tailscale network. It unifies:

- local `qwen-local` over Tailscale
- DeepSeek
- OpenRouter
- future OpenAI-compatible providers

tailgate is not an OpenRouter replacement. OpenRouter handles the public model marketplace. tailgate handles local model integration, centralized secrets, private-only policy, capability routing, latency filtering, simple fallback, and cheapest selection among qualified models.

## Features

- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /v1/embeddings`
- `POST /v1/audio/speech`
- `POST /v1/audio/transcriptions`
- streaming chat passthrough
- local / DeepSeek / OpenRouter providers
- concrete model IDs and auto routes
- local `max_concurrency=1` protection for auto routes
- in-memory health and runtime state
- optional OpenRouter model metadata and price sync
- authenticated `/tailgate/health` and `/tailgate/config`

This project intentionally does not include SQLite, Web UI, users, multi-tenancy, prompt classification, session policy, complex billing, vector databases, or dashboards.

## Architecture

Clients use:

```text
base_url = http://lightsail-tailscale-name:11435/v1
api_key = <ROUTER_API_KEY>
```

tailgate validates `Authorization: Bearer <ROUTER_API_KEY>`, selects a concrete model or auto route, replaces `model` with the provider's upstream model name, and forwards the request with the provider key from server environment variables.

Provider keys are never sent by clients and are not logged.

## Local qwen-local

Expected local service:

```text
Base URL: http://<mac-tailscale-host>:8000/v1
API key: local
Chat: local-llm
Embedding: local-embedding
TTS: local-tts
ASR: local-asr
```

Supported local endpoints:

- `/v1/chat/completions`
- `/v1/embeddings`
- `/v1/audio/speech`
- `/v1/audio/transcriptions`

The local provider has a single serialized worker, so local models should use `max_concurrency: 1`.

## Environment

Create `.env`:

```bash
cp .env.example .env
```

Variables:

```bash
ROUTER_API_KEY=change-me
LOCAL_API_KEY=local
DEEPSEEK_API_KEY=
OPENROUTER_API_KEY=
CONFIG_PATH=./config.yaml
LOG_LEVEL=info
```

`ROUTER_API_KEY` is the only key clients use. Provider keys stay on the tailgate server.

## Config

Create `config.yaml`:

```bash
cp config.example.yaml config.yaml
```

Important fields:

- `server.host`: listen address. Use the Lightsail Tailscale IP to bind only to Tailscale.
- `server.port`: listen port.
- `server.request_timeout_ms`: upstream timeout.
- `server.fallback_max_attempts`: max candidates tried for auto routes.
- `models.*.upstream_model`: model name sent to the provider.
- `models.*.api_key_env`: environment variable containing that provider key.
- `models.*.endpoint`: `chat`, `embeddings`, `audio_speech`, or `audio_transcriptions`.
- `models.*.max_concurrency`: auto routes skip the model when busy.
- `routing.latency`: global threshold filters for built-in routes.

Recommended route meanings:

- `private/chat`: private chat only.
- `private/embedding`: private embeddings only.
- `private/tts`: private TTS only.
- `private/asr`: private ASR only.
- `auto/chat`: automatic chat, local or external.
- `auto/embedding`: automatic embeddings, local or external.
- `auto/tts`: automatic TTS, local or external.
- `auto/asr`: automatic ASR, local or external.

Auto routing uses hard filtering, then the cheapest dynamic OpenRouter price when available. Without dynamic pricing, it uses the simple built-in provider order: local, DeepSeek, then OpenRouter. Ties use lower network latency.

## OpenRouter Sync

Phase 3 adds optional OpenRouter metadata sync from:

```text
GET https://openrouter.ai/api/v1/models
```

It refreshes runtime metadata for configured OpenRouter models and optionally creates a small set of runtime-only allowlist models. It does not import the full OpenRouter marketplace by default, does not sync DeepSeek prices, and does not rewrite `config.yaml`.

It is disabled by default:

```yaml
openrouter_sync:
  enabled: false
  interval_seconds: 21600
  update_config_file: false
  source_url: https://openrouter.ai/api/v1/models
  include_unconfigured_models: false
  allowlist:
    - openrouter/auto
    - openrouter/free
    - deepseek/deepseek-chat
    - anthropic/claude-sonnet-4
  cost_tiers:
    free_max_usd_per_1m_tokens: 0
    standard_max_usd_per_1m_tokens: 2
    premium_max_usd_per_1m_tokens: 9999
```

`enabled=false` is the safe default because runtime metadata can change route choices. When enabled, tailgate syncs shortly after startup and then every `interval_seconds`.

Pricing conversion:

- `pricing.prompt` and `pricing.completion` are parsed as USD per token.
- `prompt_per_1m = prompt * 1_000_000`
- `completion_per_1m = completion * 1_000_000`
- `blended_per_1m = prompt_per_1m * 0.4 + completion_per_1m * 0.6`
- zero prompt and completion means free
- runtime ranking uses the blended price when OpenRouter metadata is available

For configured OpenRouter models, sync updates runtime metadata used by routing:

- context window
- cost tier
- price ranking
- prompt/completion/request/image prices
- supported parameters
- OpenRouter display name and created timestamp

For allowlist entries not already configured, tailgate creates runtime-only chat models:

```text
openrouter/<sanitized-openrouter-id>
```

Example:

```text
deepseek/deepseek-chat -> openrouter/deepseek-deepseek-chat
```

Capability inference is conservative. Supported parameters may add metadata hints such as `tool_calling`, `structured_output`, or `reasoning`; built-in routing does not require capability configuration.

## Running Locally

```bash
npm install
npm run dev
```

Build and run:

```bash
npm run build
npm start
```

Scripts:

```bash
npm run dev
npm run build
npm run start
npm run typecheck
```

## Lightsail + Tailscale

On the server:

```bash
sudo mkdir -p /opt/tailgate
sudo chown "$USER":"$USER" /opt/tailgate
git clone https://github.com/minifish-org/tailgate.git /opt/tailgate
cd /opt/tailgate
npm install
npm run build
cp .env.example .env
cp config.example.yaml config.yaml
```

Edit `.env` and `config.yaml`. For Tailscale-only binding:

```yaml
server:
  host: 100.x.y.z
  port: 11435
```

For qwen-local via MagicDNS:

```yaml
base_url: http://macbook-air-for-home.taila2cd17.ts.net:8000/v1
```

## systemd

```bash
sudo cp deploy/tailgate.service /etc/systemd/system/tailgate.service
sudo systemctl daemon-reload
sudo systemctl enable tailgate
sudo systemctl restart tailgate
sudo systemctl status tailgate
```

Logs:

```bash
journalctl -u tailgate -f
```

## Curl Examples

Set variables:

```bash
export TAILGATE_URL=http://localhost:11435/v1
export ROUTER_API_KEY=change-me
```

Models:

```bash
curl -s "$TAILGATE_URL/models" \
  -H "Authorization: Bearer $ROUTER_API_KEY"
```

Chat local:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/chat","messages":[{"role":"user","content":"Reply with only: ok"}]}'
```

Private chat:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"private/chat","messages":[{"role":"user","content":"Reply with only: ok"}]}'
```

Auto chat:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto/chat","messages":[{"role":"user","content":"Write one short TypeScript tip."}]}'
```

Streaming chat:

```bash
curl -N "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/chat","stream":true,"messages":[{"role":"user","content":"Count from 1 to 5."}]}'
```

Embeddings:

```bash
curl -s "$TAILGATE_URL/embeddings" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/embedding","input":"hello world"}'
```

Auto embeddings:

```bash
curl -s "$TAILGATE_URL/embeddings" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto/embedding","input":"hello world"}'
```

TTS:

```bash
curl -s "$TAILGATE_URL/audio/speech" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/tts","input":"hello from tailgate","voice":"default","response_format":"wav"}' \
  --output speech.wav
```

Auto TTS:

```bash
curl -s "$TAILGATE_URL/audio/speech" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto/tts","input":"hello from tailgate","voice":"default","response_format":"wav"}' \
  --output speech.wav
```

ASR:

```bash
curl -s "$TAILGATE_URL/audio/transcriptions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -F model=local/asr \
  -F file=@speech.wav
```

Auto ASR:

```bash
curl -s "$TAILGATE_URL/audio/transcriptions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -F model=auto/asr \
  -F file=@speech.wav
```

Health:

```bash
curl -s "${TAILGATE_URL%/v1}/tailgate/health" \
  -H "Authorization: Bearer $ROUTER_API_KEY"
```

Manual OpenRouter sync:

```bash
curl -s -X POST "${TAILGATE_URL%/v1}/tailgate/sync/openrouter" \
  -H "Authorization: Bearer $ROUTER_API_KEY"
```

Sanitized config:

```bash
curl -s "${TAILGATE_URL%/v1}/tailgate/config" \
  -H "Authorization: Bearer $ROUTER_API_KEY"
```

Successful proxied responses include:

```text
X-Tailgate-Model: <selected_model>
X-Tailgate-Provider: <provider>
X-Tailgate-Route: <requested_model>
X-Tailgate-Fallback: true|false
```

## Current Limitations

- No database; runtime state is in memory.
- OpenRouter sync overlay disappears after restart.
- Config file rewrite is not implemented yet.
- No Web UI.
- No user or multi-tenant system.
- No DeepSeek price sync.
- No automatic provider marketplace beyond OpenRouter allowlist.
- Only OpenRouter chat models are imported as virtual models.
- Capability inference from OpenRouter metadata is conservative.
- No prompt classifier or session policy.
- No complex billing.
- No vector database.
- No queue dashboard.
- Concrete model requests can still forward even if the model is unhealthy or busy.
