# tailgate

tailgate is a personal OpenAI-compatible AI gateway for a 7x24 AWS Lightsail Singapore server inside a Tailscale network.

The first MVP forwards chat completions to:

- `local/chat`: local `qwen-local` service over Tailscale
- `deepseek/chat`: DeepSeek
- `openrouter/default`: OpenRouter
- `auto/private`: cheapest healthy private model
- `auto/default`: cheapest healthy allowed model

Only `/v1/models` and `/v1/chat/completions` are implemented in this version.

## Architecture

Clients call tailgate with one API key:

```text
base_url = http://lightsail-sg:11435/v1
api_key = <ROUTER_API_KEY>
```

tailgate validates `Authorization: Bearer <ROUTER_API_KEY>`, selects a configured model or auto route, replaces `body.model` with the upstream model name, and forwards to the provider with that provider's API key from the server environment.

Provider API keys are never accepted from clients and are not logged.

## Local Development

```bash
npm install
cp .env.example .env
cp config.example.yaml config.yaml
npm run dev
```

Build and run:

```bash
npm run build
npm start
```

Useful scripts:

```bash
npm run dev
npm run build
npm run start
npm run typecheck
```

## config.yaml

`CONFIG_PATH` defaults to `./config.yaml`. Start from:

```bash
cp config.example.yaml config.yaml
```

Important fields:

- `server.host`: listen host, usually `0.0.0.0`
- `server.port`: listen port, default example is `11435`
- `server.request_timeout_ms`: upstream request timeout
- `models.*.base_url`: OpenAI-compatible provider base URL ending in `/v1`
- `models.*.api_key_env`: environment variable name containing that provider key
- `models.*.max_concurrency`: kept for local provider metadata; no queue is implemented yet
- `routes.*`: auto route filters used by the selector

Auto route selection is intentionally simple: endpoint match, capability filter, privacy/external filter, cost tier filter, healthy only, then lowest `price_rank`.

## .env

```bash
ROUTER_API_KEY=change-me
LOCAL_API_KEY=local
DEEPSEEK_API_KEY=
OPENROUTER_API_KEY=
CONFIG_PATH=./config.yaml
LOG_LEVEL=info
```

`ROUTER_API_KEY` is the only key clients use. `LOCAL_API_KEY`, `DEEPSEEK_API_KEY`, and `OPENROUTER_API_KEY` stay on the tailgate server.

## Curl Tests

Set a shell variable first:

```bash
export ROUTER_API_KEY=change-me
export TAILGATE_URL=http://localhost:11435/v1
```

List models:

```bash
curl -s "$TAILGATE_URL/models" \
  -H "Authorization: Bearer $ROUTER_API_KEY"
```

Direct local chat:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "local/chat",
    "messages": [{"role": "user", "content": "Say hello in one sentence."}]
  }'
```

Private auto route:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "auto/private",
    "messages": [{"role": "user", "content": "Summarize tailgate in one sentence."}]
  }'
```

Default auto route:

```bash
curl -s "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "auto/default",
    "messages": [{"role": "user", "content": "Write a short TypeScript tip."}]
  }'
```

Streaming:

```bash
curl -N "$TAILGATE_URL/chat/completions" \
  -H "Authorization: Bearer $ROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "local/chat",
    "stream": true,
    "messages": [{"role": "user", "content": "Count from 1 to 5 slowly."}]
  }'
```

Responses include:

```text
X-Tailgate-Model: <selected_model>
X-Tailgate-Provider: <provider>
```

## Lightsail + Tailscale Deployment

On the Lightsail Singapore server:

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

Edit `/opt/tailgate/.env` and `/opt/tailgate/config.yaml`.

For the local provider, set `base_url` to the Mac's Tailscale address or MagicDNS name:

```yaml
base_url: http://<mac-tailscale-ip>:8000/v1
api_key_env: LOCAL_API_KEY
```

Make sure the Lightsail instance can reach the Mac service:

```bash
curl http://<mac-tailscale-ip>:8000/v1/models \
  -H "Authorization: Bearer local"
```

## systemd

Install the service:

```bash
sudo cp deploy/tailgate.service /etc/systemd/system/tailgate.service
sudo systemctl daemon-reload
sudo systemctl enable tailgate
sudo systemctl start tailgate
sudo systemctl status tailgate
```

View logs:

```bash
journalctl -u tailgate -f
```

## Current Limits

- Chat completions only
- No embeddings, TTS, or ASR yet
- No Web UI
- No SQLite or user system
- No automatic price fetching
- No prompt classifier
- No detailed cost dashboard
- Local provider `max_concurrency=1` is represented in config/types, but this MVP does not implement a queue
