import { HealthRegistry } from "./health.js";
import { AppConfig, ModelCatalog, ModelConfig, RuntimeModelMetadata, SelectedModel } from "./types.js";

export class RuntimeCatalog implements ModelCatalog {
  private readonly overlays = new Map<string, RuntimeModelMetadata>();
  private readonly virtualModels = new Map<string, ModelConfig>();

  constructor(
    private readonly config: AppConfig,
    private readonly health: HealthRegistry,
  ) {}

  setOverlay(modelId: string, metadata: RuntimeModelMetadata) {
    this.overlays.set(modelId, metadata);
  }

  setVirtualModel(modelId: string, model: ModelConfig, metadata: RuntimeModelMetadata) {
    this.virtualModels.set(modelId, model);
    this.overlays.set(modelId, metadata);
    this.health.ensureModel(modelId, model);
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
  };
}
