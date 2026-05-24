import { GatewayError, messageFromUnknown } from "./errors.js";
import { logger } from "./logger.js";
import { rankPer1MPricing } from "./pricing.js";
import { RuntimeCatalog } from "./runtimeCatalog.js";
import { AppConfig, RuntimeModelMetadata } from "./types.js";

interface DeepSeekModelPricing {
  id: string;
  contextWindow?: number;
  promptPer1M?: number;
  completionPer1M?: number;
  displayName?: string;
}

export interface DeepSeekSyncSummary {
  ok: boolean;
  fetched: number;
  updated_configured: number;
  warnings: string[];
  duration_ms: number;
  synced_at: string;
}

export interface DeepSeekSyncStatus {
  enabled: boolean;
  last_success_at?: string;
  last_error_at?: string;
  last_error?: string;
  last_fetched_count?: number;
  last_updated_configured?: number;
}

export class DeepSeekSyncService {
  private timer?: NodeJS.Timeout;
  private status: DeepSeekSyncStatus;

  constructor(
    private readonly config: AppConfig,
    private readonly catalog: RuntimeCatalog,
  ) {
    this.status = { enabled: config.deepseek_sync.enabled };
  }

  start() {
    if (!this.config.deepseek_sync.enabled) return;
    setTimeout(() => void this.sync().catch(() => undefined), 1_000).unref();
    this.timer = setInterval(() => void this.sync().catch(() => undefined), this.config.deepseek_sync.interval_seconds * 1_000);
    this.timer.unref();
  }

  stop() {
    if (this.timer) clearInterval(this.timer);
  }

  async sync(): Promise<DeepSeekSyncSummary> {
    const startedAt = Date.now();
    const syncedAt = new Date().toISOString();
    const warnings: string[] = [];

    try {
      const response = await fetch(this.config.deepseek_sync.source_url, { method: "GET" });
      if (!response.ok) {
        throw new GatewayError(`DeepSeek pricing sync returned ${response.status}`, 502, "deepseek_sync_failed");
      }

      const html = await response.text();
      const models = parseDeepSeekPricingPage(html);
      const byId = new Map<string, DeepSeekModelPricing>();
      for (const model of models) {
        byId.set(model.id, model);
        if (model.id === "deepseek-v4-flash") {
          byId.set("deepseek-chat", model);
          byId.set("deepseek-reasoner", model);
        }
      }

      let updatedConfigured = 0;
      for (const [tailgateModelId, model] of Object.entries(this.config.models)) {
        if (model.provider !== "deepseek") continue;
        const pricing = byId.get(model.upstream_model);
        if (!pricing) {
          warnings.push(`configured DeepSeek model not found: ${tailgateModelId} -> ${model.upstream_model}`);
          continue;
        }
        this.catalog.setOverlay(tailgateModelId, metadataFromDeepSeekPricing(pricing, this.config, syncedAt));
        updatedConfigured += 1;
      }

      const summary: DeepSeekSyncSummary = {
        ok: true,
        fetched: models.length,
        updated_configured: updatedConfigured,
        warnings,
        duration_ms: Date.now() - startedAt,
        synced_at: syncedAt,
      };

      this.status = {
        enabled: this.config.deepseek_sync.enabled,
        last_success_at: syncedAt,
        last_fetched_count: models.length,
        last_updated_configured: updatedConfigured,
      };

      logger.info(
        {
          fetched: summary.fetched,
          updated_configured: summary.updated_configured,
          warnings_count: summary.warnings.length,
          duration_ms: summary.duration_ms,
        },
        "deepseek sync completed",
      );

      return summary;
    } catch (error) {
      const message = messageFromUnknown(error);
      this.status = {
        ...this.status,
        enabled: this.config.deepseek_sync.enabled,
        last_error_at: new Date().toISOString(),
        last_error: truncate(message),
      };
      logger.warn({ error: truncate(message), duration_ms: Date.now() - startedAt }, "deepseek sync failed");
      if (error instanceof GatewayError) throw error;
      throw new GatewayError(message, 502, "deepseek_sync_failed");
    }
  }

  getStatus(): DeepSeekSyncStatus {
    return this.status;
  }
}

function parseDeepSeekPricingPage(html: string): DeepSeekModelPricing[] {
  const text = htmlToText(html);
  const modelIds = Array.from(text.matchAll(/deepseek-v4-(?:flash|pro)/g)).map((match) => match[0]);
  const uniqueModelIds = Array.from(new Set(modelIds));
  const cacheMissPrices = pricesAfterLabel(text, "1M INPUT TOKENS (CACHE MISS)");
  const outputPrices = pricesAfterLabel(text, "1M OUTPUT TOKENS");
  const contextWindow = parseContextWindow(text);

  return uniqueModelIds.map((id, index) => ({
    id,
    displayName: id,
    contextWindow,
    promptPer1M: cacheMissPrices[index],
    completionPer1M: outputPrices[index],
  }));
}

function metadataFromDeepSeekPricing(pricing: DeepSeekModelPricing, config: AppConfig, syncedAt: string): RuntimeModelMetadata {
  const ranked = rankPer1MPricing(pricing.promptPer1M, pricing.completionPer1M, config.deepseek_sync.cost_tiers);
  const tierOverride = deepSeekTierOverride(pricing.id, ranked.priceRank);
  return {
    context_window: pricing.contextWindow,
    dynamic_price_prompt: pricing.promptPer1M,
    dynamic_price_completion: pricing.completionPer1M,
    dynamic_cost_tier: tierOverride?.cost_tier ?? ranked.costTier,
    dynamic_price_rank: tierOverride?.price_rank ?? ranked.priceRank,
    provider_model_name: pricing.displayName,
    last_price_sync_at: syncedAt,
  };
}

function deepSeekTierOverride(modelId: string, priceRank: number) {
  if (modelId === "deepseek-v4-pro") return { cost_tier: "premium" as const, price_rank: Math.max(priceRank, 10_000) };
  return undefined;
}

function pricesAfterLabel(text: string, label: string): number[] {
  const start = text.indexOf(label);
  if (start === -1) return [];
  const nextLabel = text.slice(start + label.length).search(/1M [A-Z ]+TOKENS|Deduction Rules|\(\d\)/);
  const segment = nextLabel === -1 ? text.slice(start, start + 300) : text.slice(start, start + label.length + nextLabel);
  return Array.from(segment.matchAll(/\$([0-9]+(?:\.[0-9]+)?)/g)).map((match) => Number(match[1])).filter((value) => Number.isFinite(value));
}

function parseContextWindow(text: string): number | undefined {
  const match = text.match(/CONTEXT LENGTH\s+([0-9]+(?:\.[0-9]+)?)([MK])/i);
  if (!match) return undefined;
  const value = Number(match[1]);
  const unit = match[2]?.toUpperCase();
  if (!Number.isFinite(value)) return undefined;
  return unit === "M" ? value * 1_000_000 : value * 1_000;
}

function htmlToText(html: string): string {
  return html
    .replace(/<script[\s\S]*?<\/script>/gi, " ")
    .replace(/<style[\s\S]*?<\/style>/gi, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

function truncate(value: string): string {
  return value.replace(/\s+/g, " ").slice(0, 240);
}
