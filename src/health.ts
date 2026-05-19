import { AppConfig, HealthModelConfig, HealthState } from "./types.js";
import { logger } from "./logger.js";
import { messageFromUnknown } from "./errors.js";

const EWMA_ALPHA = 0.2;

export class HealthRegistry {
  private readonly states = new Map<string, HealthState>();
  private readonly models = new Map<string, HealthModelConfig>();
  private readonly timerModels = new Set<string>();
  private readonly timers: NodeJS.Timeout[] = [];

  constructor(private readonly config: AppConfig) {
    for (const [modelName, model] of Object.entries(config.models)) {
      this.models.set(modelName, model);
      this.states.set(modelName, initialState());
    }
  }

  get(modelName: string): HealthState {
    return this.states.get(modelName) || initialState();
  }

  ensureModel(modelName: string, model?: HealthModelConfig, healthy = false) {
    if (model) {
      this.models.set(modelName, model);
      this.ensureTimer(modelName, model);
    }
    const state = this.mutableState(modelName);
    if (healthy) state.healthy = true;
  }

  snapshot(models = Object.entries(this.config.models).map(([id, model]) => ({ id, provider: model.provider, endpoint: model.endpoint }))) {
    return models
      .sort((a, b) => a.id.localeCompare(b.id))
      .map((model) => ({
        id: model.id,
        provider: model.provider,
        endpoint: model.endpoint,
        ...this.get(model.id),
      }));
  }

  beginRequest(modelName: string) {
    const state = this.mutableState(modelName);
    state.in_flight += 1;
    state.busy = isBusy(this.models.get(modelName), state.in_flight);
  }

  finishRequest(modelName: string, result: { ok: boolean; totalLatencyMs: number; firstTokenLatencyMs?: number; error?: string }) {
    const state = this.mutableState(modelName);
    state.in_flight = Math.max(0, state.in_flight - 1);
    state.busy = isBusy(this.models.get(modelName), state.in_flight);
    state.total_latency_ms = ewma(state.total_latency_ms, result.totalLatencyMs);
    if (result.firstTokenLatencyMs !== undefined) {
      state.first_token_latency_ms = ewma(state.first_token_latency_ms, result.firstTokenLatencyMs);
    }

    if (result.ok) {
      state.success_count += 1;
      state.last_error = undefined;
    } else {
      state.error_count += 1;
      state.last_error = truncateError(result.error || "request failed");
    }

    const total = state.success_count + state.error_count;
    state.error_rate = total === 0 ? 0 : state.error_count / total;
  }

  recordFirstToken(modelName: string, firstTokenLatencyMs: number) {
    const state = this.mutableState(modelName);
    state.first_token_latency_ms = ewma(state.first_token_latency_ms, firstTokenLatencyMs);
  }

  start() {
    for (const [modelName, modelConfig] of this.models.entries()) {
      this.ensureTimer(modelName, modelConfig);
    }
  }

  stop() {
    for (const timer of this.timers) clearInterval(timer);
  }

  private async checkModel(modelName: string) {
    const model = this.models.get(modelName);
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
        this.updateHealth(modelName, false, latency, `health check returned ${response.status}`);
        return;
      }

      this.updateHealth(modelName, true, latency);
    } catch (error) {
      this.updateHealth(modelName, false, Date.now() - startedAt, messageFromUnknown(error));
      logger.debug({ model: modelName, provider: model.provider, error: messageFromUnknown(error) }, "health check failed");
    } finally {
      clearTimeout(timeout);
    }
  }

  private updateHealth(modelName: string, healthy: boolean, latencyMs: number, error?: string) {
    const state = this.mutableState(modelName);
    state.healthy = healthy;
    state.last_checked_at = new Date().toISOString();
    state.network_latency_ms = ewma(state.network_latency_ms, latencyMs);
    state.last_error = error ? truncateError(error) : state.last_error;
  }

  private mutableState(modelName: string): HealthState {
    const existing = this.states.get(modelName);
    if (existing) return existing;
    const created = initialState();
    this.states.set(modelName, created);
    return created;
  }

  private ensureTimer(modelName: string, modelConfig: HealthModelConfig) {
    if (this.timerModels.has(modelName)) return;
    this.timerModels.add(modelName);
    const intervalMs = modelConfig.provider === "local" ? 15_000 : 30_000;
    void this.checkModel(modelName);
    const timer = setInterval(() => void this.checkModel(modelName), intervalMs);
    timer.unref();
    this.timers.push(timer);
  }
}

function providerHeaders(apiKeyEnv: string): HeadersInit {
  const apiKey = process.env[apiKeyEnv];
  if (!apiKey) return {};
  return { Authorization: `Bearer ${apiKey}` };
}

function initialState(): HealthState {
  return {
    healthy: false,
    busy: false,
    in_flight: 0,
    error_rate: 0,
    success_count: 0,
    error_count: 0,
  };
}

function ewma(previous: number | undefined, next: number): number {
  return previous === undefined ? Math.round(next) : Math.round(previous * (1 - EWMA_ALPHA) + next * EWMA_ALPHA);
}

function isBusy(model: HealthModelConfig | undefined, inFlight: number): boolean {
  return model?.max_concurrency !== undefined && inFlight >= model.max_concurrency;
}

function truncateError(error: string): string {
  return error.replace(/\s+/g, " ").slice(0, 240);
}
