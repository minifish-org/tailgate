import { CostTier, OpenRouterCostTierConfig } from "./types.js";

export interface ParsedOpenRouterPricing {
  prompt?: number;
  completion?: number;
  request?: number;
  image?: number;
  promptPer1M?: number;
  completionPer1M?: number;
  blendedPer1M?: number;
  costTier: CostTier;
  priceRank: number;
}

export function parseOpenRouterPricing(pricing: unknown, costTiers: OpenRouterCostTierConfig): ParsedOpenRouterPricing {
  const raw = isRecord(pricing) ? pricing : {};
  const prompt = parsePrice(raw.prompt);
  const completion = parsePrice(raw.completion);
  const request = parsePrice(raw.request);
  const image = parsePrice(raw.image);
  const promptPer1M = prompt === undefined ? undefined : prompt * 1_000_000;
  const completionPer1M = completion === undefined ? undefined : completion * 1_000_000;
  const blendedPer1M = blend(promptPer1M, completionPer1M);

  if (promptPer1M === 0 && completionPer1M === 0) {
    return { prompt, completion, request, image, promptPer1M, completionPer1M, blendedPer1M: 0, costTier: "free", priceRank: 0 };
  }

  const blended = blendedPer1M ?? Number.POSITIVE_INFINITY;
  const costTier: CostTier = blended <= costTiers.standard_max_usd_per_1m_tokens ? "standard" : "premium";
  const baseRank = Number.isFinite(blended) ? Math.round(blended * 100) : 999_999;
  const priceRank = costTier === "standard" ? baseRank : baseRank + 10_000;

  return { prompt, completion, request, image, promptPer1M, completionPer1M, blendedPer1M, costTier, priceRank };
}

function parsePrice(value: unknown): number | undefined {
  if (value === undefined || value === null || value === "") return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : undefined;
}

function blend(promptPer1M: number | undefined, completionPer1M: number | undefined): number | undefined {
  if (promptPer1M === undefined && completionPer1M === undefined) return undefined;
  return (promptPer1M ?? 0) * 0.4 + (completionPer1M ?? 0) * 0.6;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
