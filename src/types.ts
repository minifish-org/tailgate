export type Provider = "local" | "deepseek" | "openrouter" | string;
export type Endpoint = "chat";
export type CostTier = "free" | "standard" | "premium";

export interface ServerConfig {
  host: string;
  port: number;
  request_timeout_ms: number;
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
  context_window: number;
  max_concurrency?: number;
}

export interface RouteConfig {
  endpoint: Endpoint;
  required_capabilities?: Record<string, number>;
  require_private?: boolean;
  allow_external?: boolean;
  max_cost_tier?: CostTier;
  optimize: "cheapest";
}

export interface AppConfig {
  server: ServerConfig;
  models: Record<string, ModelConfig>;
  routes: Record<string, RouteConfig>;
}

export interface HealthState {
  healthy: boolean;
  last_error?: string;
  last_checked_at?: string;
  network_latency_ms?: number;
}

export interface SelectedModel {
  name: string;
  config: ModelConfig;
}

export interface ChatCompletionBody {
  model?: unknown;
  stream?: unknown;
  [key: string]: unknown;
}
