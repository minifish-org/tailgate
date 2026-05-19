export type Provider = "local" | "deepseek" | "openrouter" | string;
export type Endpoint = "chat" | "embeddings" | "audio_speech" | "audio_transcriptions";
export type CostTier = "free" | "standard" | "premium";

export interface ServerConfig {
  host: string;
  port: number;
  request_timeout_ms: number;
  fallback_max_attempts: number;
}

export interface OpenRouterCostTierConfig {
  free_max_usd_per_1m_tokens: number;
  standard_max_usd_per_1m_tokens: number;
  premium_max_usd_per_1m_tokens: number;
}

export interface OpenRouterSyncConfig {
  enabled: boolean;
  interval_seconds: number;
  update_config_file: boolean;
  source_url: string;
  include_unconfigured_models: boolean;
  allowlist: string[];
  cost_tiers: OpenRouterCostTierConfig;
}

export interface ModelCapabilities {
  general?: number;
  coding?: number;
  reasoning?: number;
  private?: boolean;
  [key: string]: number | boolean | undefined;
}

export interface ModelConfig {
  provider: Provider;
  upstream_model: string;
  base_url: string;
  api_key_env: string;
  endpoint: Endpoint;
  capabilities: ModelCapabilities;
  cost_tier: CostTier;
  price_rank: number;
  context_window?: number;
  max_concurrency?: number;
  supported_parameters?: string[];
}

export interface RouteLatencyConfig {
  network_p95_ms_max?: number;
  first_token_p95_ms_max?: number;
}

export interface RoutingConfig {
  latency?: RouteLatencyConfig;
}

export interface RouteConfig {
  endpoint: Endpoint;
  required_capabilities?: Record<string, number>;
  require_private?: boolean;
  allow_external?: boolean;
  max_cost_tier?: CostTier;
  latency?: RouteLatencyConfig;
  optimize: "cheapest";
}

export interface AppConfig {
  server: ServerConfig;
  models: Record<string, ModelConfig>;
  routes: Record<string, RouteConfig>;
  routing: RoutingConfig;
  openrouter_sync: OpenRouterSyncConfig;
}

export interface RuntimeModelMetadata {
  context_window?: number;
  dynamic_price_prompt?: number;
  dynamic_price_completion?: number;
  dynamic_price_request?: number;
  dynamic_price_image?: number;
  dynamic_cost_tier?: CostTier;
  dynamic_price_rank?: number;
  supported_parameters?: string[];
  openrouter_model_name?: string;
  openrouter_created?: number;
  last_price_sync_at?: string;
}

export interface HealthState {
  healthy: boolean;
  busy: boolean;
  in_flight: number;
  last_error?: string;
  last_checked_at?: string;
  network_latency_ms?: number;
  first_token_latency_ms?: number;
  total_latency_ms?: number;
  error_rate: number;
  success_count: number;
  error_count: number;
}

export interface SelectedModel {
  name: string;
  config: ModelConfig;
  metadata?: RuntimeModelMetadata;
}

export interface OpenAIJsonBody {
  model?: unknown;
  stream?: unknown;
  [key: string]: unknown;
}

export interface ProxyEndpointSpec {
  endpoint: Endpoint;
  upstreamPath: string;
  bodyKind: "json" | "multipart";
  supportsStream: boolean;
}

export interface ModelCatalog {
  getModel(modelName: string): SelectedModel | undefined;
  getModelEntries(): Array<[string, ModelConfig, RuntimeModelMetadata | undefined]>;
  getConfiguredModelEntries(): Array<[string, ModelConfig]>;
  getVirtualModelIds(): string[];
  getRuntimeMetadataSummary(): Record<string, RuntimeModelMetadata>;
}
