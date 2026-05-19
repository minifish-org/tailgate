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
- authenticated `/tailgate/health` and `/tailgate/config`

Phase 2 intentionally does not include SQLite, Web UI, users, multi-tenancy, price sync, prompt classification, session policy, complex billing, vector databases, or dashboards.

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
- `models.*.capabilities`: simple capability tags used by auto routes.
- `models.*.max_concurrency`: auto routes skip the model when busy.
- `routes.*.latency`: optional threshold filters.

Recommended route meanings:

- `auto/private`: local only, never external.
- `auto/default`: general standard-cost route.
- `auto/coding`: coding tier 2 route.
- `auto/reasoning`: allows premium.
- `auto/embedding`: local only.
- `auto/tts`: local only.
- `auto/asr`: local only.

Auto routing uses hard filtering, then cheapest selection by `price_rank`. Ties use lower network latency.

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

Chat private auto:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto/private","messages":[{"role":"user","content":"Reply with only: ok"}]}'
```

Chat default auto:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto/default","messages":[{"role":"user","content":"Write one short TypeScript tip."}]}'
```

Chat coding auto:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto/coding","messages":[{"role":"user","content":"Explain a Promise in one sentence."}]}'
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
- No Web UI.
- No user or multi-tenant system.
- No OpenRouter price sync or automatic pricing.
- No prompt classifier or session policy.
- No complex billing.
- No vector database.
- No queue dashboard.
- Concrete model requests can still forward even if the model is unhealthy or busy.
