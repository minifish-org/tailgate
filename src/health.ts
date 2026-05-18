import { AppConfig, HealthState } from "./types.js";
import { logger } from "./logger.js";
import { messageFromUnknown } from "./errors.js";

export class HealthRegistry {
  private readonly states = new Map<string, HealthState>();
  private readonly timers: NodeJS.Timeout[] = [];

  constructor(private readonly config: AppConfig) {
    for (const modelName of Object.keys(config.models)) {
      this.states.set(modelName, { healthy: false });
    }
  }

  get(modelName: string): HealthState {
    return this.states.get(modelName) || { healthy: false };
  }

  start() {
    for (const [modelName, modelConfig] of Object.entries(this.config.models)) {
      const intervalMs = modelConfig.provider === "local" ? 15_000 : 30_000;
      void this.checkModel(modelName);
      const timer = setInterval(() => void this.checkModel(modelName), intervalMs);
      timer.unref();
      this.timers.push(timer);
    }
  }

  stop() {
    for (const timer of this.timers) clearInterval(timer);
  }

  private async checkModel(modelName: string) {
    const model = this.config.models[modelName];
    if (!model) return;

    const startedAt = Date.now();
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), Math.min(this.config.server.request_timeout_ms, 10_000));

    try {
      const response = await fetch(`${model.base_url}/models`, {
        method: "GET",
        headers: providerHeaders(model.api_key_env),
        signal: controller.signal,
      });
      const latency = Date.now() - startedAt;

      if (!response.ok) {
        this.states.set(modelName, {
          healthy: false,
          last_error: `health check returned ${response.status}`,
          last_checked_at: new Date().toISOString(),
          network_latency_ms: latency,
        });
        return;
      }

      this.states.set(modelName, {
        healthy: true,
        last_checked_at: new Date().toISOString(),
        network_latency_ms: latency,
      });
    } catch (error) {
      this.states.set(modelName, {
        healthy: false,
        last_error: messageFromUnknown(error),
        last_checked_at: new Date().toISOString(),
        network_latency_ms: Date.now() - startedAt,
      });
      logger.debug({ model: modelName, provider: model.provider, error: messageFromUnknown(error) }, "health check failed");
    } finally {
      clearTimeout(timeout);
    }
  }
}

function providerHeaders(apiKeyEnv: string): HeadersInit {
  const apiKey = process.env[apiKeyEnv];
  if (!apiKey) return {};
  return { Authorization: `Bearer ${apiKey}` };
}
