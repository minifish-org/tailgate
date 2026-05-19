import { GatewayError, messageFromUnknown } from "./errors.js";
import { HealthRegistry } from "./health.js";
import { logger } from "./logger.js";
import { parseOpenRouterPricing } from "./pricing.js";
import { AppConfig, ModelCapabilities, ModelCatalog, ModelConfig, RuntimeModelMetadata, SelectedModel } from "./types.js";

interface OpenRouterModel {
  id: string;
  name?: string;
  context_length?: number;
  pricing?: unknown;
  supported_parameters?: string[];
  architecture?: unknown;
  created?: number;
}

export interface OpenRouterSyncSummary {
  ok: boolean;
  fetched: number;
  updated_configured: number;
  added_virtual: number;
  warnings: string[];
  duration_ms: number;
  synced_at: string;
}

export interface OpenRouterSyncStatus {
  enabled: boolean;
  last_success_at?: string;
  last_error_at?: string;
  last_error?: string;
  last_fetched_count?: number;
  last_updated_configured?: number;
  last_added_virtual?: number;
}

export class OpenRouterSyncService implements ModelCatalog {
  private readonly overlays = new Map<string, RuntimeModelMetadata>();
  private readonly virtualModels = new Map<string, ModelConfig>();
  private timer?: NodeJS.Timeout;
  private status: OpenRouterSyncStatus;

  constructor(
    private readonly config: AppConfig,
    private readonly health: HealthRegistry,
  ) {
    this.status = { enabled: config.openrouter_sync.enabled };
  }

  start() {
    if (!this.config.openrouter_sync.enabled) return;
    setTimeout(() => void this.sync().catch(() => undefined), 1_000).unref();
    this.timer = setInterval(() => void this.sync().catch(() => undefined), this.config.openrouter_sync.interval_seconds * 1_000);
    this.timer.unref();
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
  }

  async sync(): Promise<OpenRouterSyncSummary> {
    const apiKey = process.env.OPENROUTER_API_KEY;
    if (!apiKey) {
      throw new GatewayError("OPENROUTER_API_KEY is not configured", 500, "openrouter_key_not_configured");
    }

    const startedAt = Date.now();
    const warnings: string[] = [];
    const syncedAt = new Date().toISOString();
    if (this.config.openrouter_sync.update_config_file) {
      warnings.push("openrouter_sync.update_config_file is not implemented; using runtime metadata only");
      logger.warn("openrouter_sync.update_config_file is not implemented; using runtime metadata only");
    }

    try {
      const response = await fetch(this.config.openrouter_sync.source_url, {
        method: "GET",
        headers: { Authorization: `Bearer ${apiKey}` },
      });
      if (!response.ok) {
        throw new GatewayError(`OpenRouter models sync returned ${response.status}`, 502, "openrouter_sync_failed");
      }

      const payload = (await response.json()) as unknown;
      const models = parseModelsPayload(payload);
      const byId = new Map(models.map((model) => [model.id, model]));
      let updatedConfigured = 0;
      let addedVirtual = 0;

      for (const [tailgateModelId, model] of Object.entries(this.config.models)) {
        if (model.provider !== "openrouter") continue;
        const openrouterModel = byId.get(model.upstream_model);
        if (!openrouterModel) {
          warnings.push(`configured OpenRouter model not found: ${tailgateModelId} -> ${model.upstream_model}`);
          continue;
        }
        this.overlays.set(tailgateModelId, metadataFromOpenRouterModel(openrouterModel, this.config, syncedAt));
        updatedConfigured += 1;
      }

      const allowedIds = this.config.openrouter_sync.include_unconfigured_models ? models.map((model) => model.id) : this.config.openrouter_sync.allowlist;
      for (const openrouterId of allowedIds) {
        if (configuredUpstreamIds(this.config).has(openrouterId)) continue;
        const openrouterModel = byId.get(openrouterId);
        if (!openrouterModel) {
          warnings.push(`allowlist OpenRouter model not found: ${openrouterId}`);
          continue;
        }

        const virtualId = virtualModelId(openrouterId);
        const metadata = metadataFromOpenRouterModel(openrouterModel, this.config, syncedAt);
        this.virtualModels.set(virtualId, virtualModelConfig(openrouterModel, metadata));
        this.overlays.set(virtualId, metadata);
        this.health.ensureModel(virtualId);
        addedVirtual += 1;
      }

      const summary: OpenRouterSyncSummary = {
        ok: true,
        fetched: models.length,
        updated_configured: updatedConfigured,
        added_virtual: addedVirtual,
        warnings,
        duration_ms: Date.now() - startedAt,
        synced_at: syncedAt,
      };

      this.status = {
        enabled: this.config.openrouter_sync.enabled,
        last_success_at: syncedAt,
        last_fetched_count: models.length,
        last_updated_configured: updatedConfigured,
        last_added_virtual: addedVirtual,
      };

      logger.info(
        {
          fetched: summary.fetched,
          updated_configured: summary.updated_configured,
          added_virtual: summary.added_virtual,
          warnings_count: summary.warnings.length,
          duration_ms: summary.duration_ms,
        },
        "openrouter sync completed",
      );

      return summary;
    } catch (error) {
      const message = messageFromUnknown(error);
      this.status = {
        ...this.status,
        enabled: this.config.openrouter_sync.enabled,
        last_error_at: new Date().toISOString(),
        last_error: truncate(message),
      };
      logger.warn({ error: truncate(message), duration_ms: Date.now() - startedAt }, "openrouter sync failed");
      if (error instanceof GatewayError) throw error;
      throw new GatewayError(message, 502, "openrouter_sync_failed");
    }
  }

  getStatus(): OpenRouterSyncStatus {
    return this.status;
  }

  getModel(modelName: string): SelectedModel | undefined {
    const configured = this.config.models[modelName];
    if (configured) return { name: modelName, config: effectiveModel(configured, this.overlays.get(modelName)), metadata: this.overlays.get(modelName) };
    const virtual = this.virtualModels.get(modelName);
    if (virtual) return { name: modelName, config: effectiveModel(virtual, this.overlays.get(modelName)), metadata: this.overlays.get(modelName) };
    return undefined;
  }

  getModelEntries(): Array<[string, ModelConfig, RuntimeModelMetadata | undefined]> {
    return [
      ...Object.entries(this.config.models).map(([id, model]) => [id, effectiveModel(model, this.overlays.get(id)), this.overlays.get(id)] as [string, ModelConfig, RuntimeModelMetadata | undefined]),
      ...Array.from(this.virtualModels.entries()).map(([id, model]) => [id, effectiveModel(model, this.overlays.get(id)), this.overlays.get(id)] as [string, ModelConfig, RuntimeModelMetadata | undefined]),
    ];
  }

  getConfiguredModelEntries(): Array<[string, ModelConfig]> {
    return Object.entries(this.config.models);
  }

  getVirtualModelIds(): string[] {
    return Array.from(this.virtualModels.keys()).sort();
  }

  getRuntimeMetadataSummary(): Record<string, RuntimeModelMetadata> {
    return Object.fromEntries(Array.from(this.overlays.entries()).sort(([a], [b]) => a.localeCompare(b)));
  }
}

function effectiveModel(model: ModelConfig, metadata?: RuntimeModelMetadata): ModelConfig {
  if (!metadata) return model;
  return {
    ...model,
    context_window: metadata.context_window ?? model.context_window,
    cost_tier: metadata.dynamic_cost_tier ?? model.cost_tier,
    price_rank: metadata.dynamic_price_rank ?? model.price_rank,
    supported_parameters: metadata.supported_parameters ?? model.supported_parameters,
    capabilities: capabilityHints(model.capabilities, metadata.supported_parameters),
  };
}

function capabilityHints(capabilities: ModelCapabilities, supportedParameters?: string[]): ModelCapabilities {
  if (!supportedParameters) return capabilities;
  const result: ModelCapabilities = { ...capabilities };
  const parameters = new Set(supportedParameters);
  if (parameters.has("tools")) result.tool_calling = 1;
  if (parameters.has("response_format") || parameters.has("structured_outputs")) result.structured_output = 1;
  if (parameters.has("reasoning") || parameters.has("include_reasoning")) result.reasoning = Math.max(typeof result.reasoning === "number" ? result.reasoning : 0, 2);
  return result;
}

function metadataFromOpenRouterModel(model: OpenRouterModel, config: AppConfig, syncedAt: string): RuntimeModelMetadata {
  const pricing = parseOpenRouterPricing(model.pricing, config.openrouter_sync.cost_tiers);
  return {
    context_window: model.context_length,
    dynamic_price_prompt: pricing.promptPer1M,
    dynamic_price_completion: pricing.completionPer1M,
    dynamic_price_request: pricing.request,
    dynamic_price_image: pricing.image,
    dynamic_cost_tier: pricing.costTier,
    dynamic_price_rank: pricing.priceRank,
    supported_parameters: model.supported_parameters,
    openrouter_model_name: model.name,
    openrouter_created: model.created,
    last_price_sync_at: syncedAt,
  };
}

function virtualModelConfig(model: OpenRouterModel, metadata: RuntimeModelMetadata): ModelConfig {
  return {
    provider: "openrouter",
    upstream_model: model.id,
    base_url: "https://openrouter.ai/api/v1",
    api_key_env: "OPENROUTER_API_KEY",
    endpoint: "chat",
    capabilities: capabilityHints({ general: 2 }, metadata.supported_parameters),
    cost_tier: metadata.dynamic_cost_tier ?? "standard",
    price_rank: metadata.dynamic_price_rank ?? 999_999,
    context_window: metadata.context_window,
    supported_parameters: metadata.supported_parameters,
  };
}

function parseModelsPayload(payload: unknown): OpenRouterModel[] {
  const data = isRecord(payload) && Array.isArray(payload.data) ? payload.data : [];
  return data.filter(isOpenRouterModel);
}

function isOpenRouterModel(value: unknown): value is OpenRouterModel {
  return isRecord(value) && typeof value.id === "string";
}

function configuredUpstreamIds(config: AppConfig): Set<string> {
  return new Set(Object.values(config.models).filter((model) => model.provider === "openrouter").map((model) => model.upstream_model));
}

function virtualModelId(openrouterId: string): string {
  return `openrouter/${openrouterId.replace(/[^a-zA-Z0-9]+/g, "-").replace(/^-|-$/g, "")}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function truncate(value: string): string {
  return value.replace(/\s+/g, " ").slice(0, 240);
}
