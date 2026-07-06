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
- `POST /v1/translations`
- streaming chat passthrough
- local / DeepSeek / OpenRouter providers
- concrete model IDs and price-tier routes
- local `max_concurrency=1` protection for tier routes
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

tailgate validates `Authorization: Bearer <ROUTER_API_KEY>`, selects a concrete model or price-tier route, replaces `model` with the provider's upstream model name, and forwards the request with the provider key from server environment variables.

Provider keys are never sent by clients and are not logged.

## Local qwen-local

Expected local service:

```text
Base URL: http://<mac-tailscale-host>:8000/v1
API key: local
Chat: local-llm
Embedding: local-embedding
TTS: local-tts
TTS VoiceDesign: local-tts-voice-design
ASR: local-asr
Translation: local-translation
```

Supported local endpoints:

- `/v1/chat/completions`
- `/v1/embeddings`
- `/v1/audio/speech`
- `/v1/audio/transcriptions`
- `/v1/translations`

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

Minimal shape:

```yaml
server:
  host: 100.100.89.60
  port: 11435
  cors_allowed_origins:
    - http://127.0.0.1:5173
    - http://localhost:5173
    # Use "*" only when tailgate is reachable from trusted networks such as Tailscale.
    # - "*"

sync:
  openrouter: true
  deepseek: true

local:
  base_url: http://macbook-air-for-home.taila2cd17.ts.net:8000/v1

deepseek:
  model: deepseek-v4-flash
  premium_model: deepseek-v4-pro

openrouter:
  free_model: openrouter/free
  standard_model: openrouter/auto
  premium_model: moonshotai/kimi-k2.6
```

Important fields:

- `server.host`: listen address. Use the Lightsail Tailscale IP to bind only to Tailscale.
- `server.port`: listen port.
- `server.cors_allowed_origins`: browser origins allowed to call tailgate routes. Configure your Cloudflare Pages origin explicitly. The safe default is localhost only; do not use `*` when tailgate is reachable from the public internet.
- `sync.openrouter`: enable OpenRouter metadata and price sync.
- `sync.deepseek`: enable DeepSeek price sync.
- `routing.network_ms_max`: global network latency cutoff for tier routes.
- `routing.first_token_ms_max`: global first-token latency cutoff for tier routes.
- `pricing.standard_max_usd_per_1m_tokens`: max blended price still considered standard.
- `local.base_url`: qwen-local base URL.
- `deepseek.model`: DeepSeek standard upstream model name.
- `deepseek.premium_model`: DeepSeek premium upstream model name.
- `openrouter.free_model`: OpenRouter free-tier model.
- `openrouter.standard_model`: OpenRouter standard-tier model.
- `openrouter.premium_model`: OpenRouter premium fallback model.

Recommended route meanings:

- `local/chat`: concrete local chat model.
- `local/embedding`: concrete local embedding model.
- `local/tts`: concrete local default TTS model.
- `local/tts-quality`: concrete local higher-quality TTS model.
- `local/tts-voice-design`: concrete local TTS VoiceDesign model.
- `local/asr`: concrete local ASR model.
- `local/translation`: concrete local translation model.
- `free/translation`: select a free translation model.
- `free/chat`: select a free chat model.
- `standard/chat`: select a standard-priced chat model.
- `premium/chat`: select a premium-priced chat model.

The same price-tier pattern exists for `embedding`, `tts`, `asr`, and `translation`: `free/embedding`, `standard/embedding`, `premium/embedding`, and so on. Tier routing uses hard filtering, then provider price when available. Without dynamic pricing, tailgate treats local as `free`, DeepSeek as `standard`, and most OpenRouter models as `standard` unless dynamic sync or the model ID marks them differently.

With the simplified config, `premium/chat` includes `deepseek/premium` first and then the configured OpenRouter premium fallback. The default DeepSeek premium model is `deepseek-v4-pro`.

When multiple candidates have the same price ranking, tailgate uses the model order from config as the tiebreaker. In the simplified config, DeepSeek is generated before OpenRouter, so `standard/chat` prefers `deepseek/chat` over `openrouter/auto` when they are otherwise tied. If price rankings differ, the cheaper qualified model still wins.

## Price Sync

tailgate can optionally refresh runtime price metadata for configured OpenRouter and DeepSeek models.

OpenRouter sync reads JSON metadata from:

```text
GET https://openrouter.ai/api/v1/models
```

DeepSeek sync reads the official pricing page:

```text
GET https://api-docs.deepseek.com/quick_start/pricing/
```

OpenRouter sync refreshes configured OpenRouter models and creates a runtime-only free model from `openrouter.free_model`. DeepSeek sync refreshes the configured DeepSeek model. tailgate does not import a full provider marketplace and does not rewrite `config.yaml`.

Both sync jobs are disabled by default:

```yaml
sync:
  openrouter: false
  deepseek: false
  interval_seconds: 21600
```

`false` is the safe default because runtime metadata can change route choices. When enabled, tailgate syncs shortly after startup and then every `interval_seconds`.

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

For configured DeepSeek models, sync updates:

- context window
- prompt price per 1M tokens
- completion price per 1M tokens
- cost tier
- price ranking

For allowlist entries not already configured, tailgate creates runtime-only chat models:

```text
openrouter/<model-name>
```

Example:

```text
openrouter/free -> openrouter/free
deepseek/deepseek-chat -> openrouter/deepseek-deepseek-chat
```

Capability inference is conservative. Supported parameters may add metadata hints such as `tool_calling`, `structured_output`, or `reasoning`; built-in routing does not require capability configuration.

## Running Locally

```bash
cargo run
```

Build and run:

```bash
cargo build --release
./target/release/tailgate
```

Compatibility checks while the TypeScript reference implementation remains in the repo:

```bash
npm install
npm run dev
npm run build
npm run start
npm run typecheck
npm test
```

## Lightsail + Tailscale

On the server:

```bash
sudo mkdir -p /opt/tailgate
sudo chown "$USER":"$USER" /opt/tailgate
git clone https://github.com/minifish-org/tailgate.git /opt/tailgate
cd /opt/tailgate
cargo build --release
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

Free-tier chat:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"free/chat","messages":[{"role":"user","content":"Reply with only: ok"}]}'
```

Standard-tier chat:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"standard/chat","messages":[{"role":"user","content":"Write one short TypeScript tip."}]}'
```

Premium-tier chat:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"premium/chat","messages":[{"role":"user","content":"Write one short TypeScript tip."}]}'
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

Free-tier embeddings:

```bash
curl -s "$TAILGATE_URL/embeddings" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"free/embedding","input":"hello world"}'
```

TTS:

```bash
curl -s "$TAILGATE_URL/audio/speech" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/tts","input":"hello from tailgate","voice":"default","response_format":"wav"}' \
  --output speech.wav
```

Quality TTS:

```bash
curl -s "$TAILGATE_URL/audio/speech" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/tts-quality","input":"hello from tailgate","voice":"vivian","response_format":"wav"}' \
  --output speech-quality.wav
```

VoiceDesign TTS:

```bash
curl -s "$TAILGATE_URL/audio/speech" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"local/tts-voice-design","input":"hello from tailgate","instruct":"natural warm conversational voice","response_format":"wav"}' \
  --output speech-voice-design.wav
```

Free-tier TTS:

```bash
curl -s "$TAILGATE_URL/audio/speech" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"free/tts","input":"hello from tailgate","voice":"default","response_format":"wav"}' \
  --output speech.wav
```

ASR:

```bash
curl -s "$TAILGATE_URL/audio/transcriptions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -F model=local/asr \
  -F file=@speech.wav
```

Free-tier ASR:

```bash
curl -s "$TAILGATE_URL/audio/transcriptions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -F model=free/asr \
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

Manual DeepSeek sync:

```bash
curl -s -X POST "${TAILGATE_URL%/v1}/tailgate/sync/deepseek" \
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
- DeepSeek sync overlay disappears after restart.
- Config file rewrite is not implemented yet.
- No Web UI.
- No user or multi-tenant system.
- No automatic provider marketplace beyond OpenRouter allowlist.
- Only OpenRouter chat models are imported as virtual models.
- Capability inference from OpenRouter metadata is conservative.
- No prompt classifier or session policy.
- No complex billing.
- No vector database.
- No queue dashboard.
- Concrete model requests can still forward even if the model is unhealthy or busy.
