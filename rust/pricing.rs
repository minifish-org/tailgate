use serde_json::Value;

use crate::types::CostTierConfig;

#[derive(Clone, Debug)]
pub struct ParsedOpenRouterPricing {
    pub prompt: Option<f64>,
    pub completion: Option<f64>,
    pub request: Option<f64>,
    pub image: Option<f64>,
    pub prompt_per_1m: Option<f64>,
    pub completion_per_1m: Option<f64>,
    pub cost_tier: String,
    pub price_rank: i64,
}

#[derive(Clone, Debug)]
pub struct RankedPricing {
    pub blended_per_1m: Option<f64>,
    pub cost_tier: String,
    pub price_rank: i64,
}

pub fn parse_openrouter_pricing(
    pricing: Option<&Value>,
    cost_tiers: &CostTierConfig,
) -> ParsedOpenRouterPricing {
    let raw = pricing.and_then(Value::as_object);
    let prompt = raw.and_then(|raw| raw.get("prompt")).and_then(parse_price);
    let completion = raw
        .and_then(|raw| raw.get("completion"))
        .and_then(parse_price);
    let request = raw.and_then(|raw| raw.get("request")).and_then(parse_price);
    let image = raw.and_then(|raw| raw.get("image")).and_then(parse_price);
    let prompt_per_1m = prompt.map(|price| price * 1_000_000.0);
    let completion_per_1m = completion.map(|price| price * 1_000_000.0);
    let ranked = rank_per_1m_pricing(prompt_per_1m, completion_per_1m, cost_tiers);
    ParsedOpenRouterPricing {
        prompt,
        completion,
        request,
        image,
        prompt_per_1m,
        completion_per_1m,
        cost_tier: ranked.cost_tier,
        price_rank: ranked.price_rank,
    }
}

pub fn rank_per_1m_pricing(
    prompt_per_1m: Option<f64>,
    completion_per_1m: Option<f64>,
    cost_tiers: &CostTierConfig,
) -> RankedPricing {
    let blended_per_1m = blend(prompt_per_1m, completion_per_1m);
    if prompt_per_1m == Some(0.0) && completion_per_1m == Some(0.0) {
        return RankedPricing {
            blended_per_1m,
            cost_tier: "free".to_string(),
            price_rank: 0,
        };
    }
    let blended = blended_per_1m.unwrap_or(f64::INFINITY);
    let cost_tier = if blended <= cost_tiers.standard_max_usd_per_1m_tokens {
        "standard"
    } else {
        "premium"
    };
    let base_rank = if blended.is_finite() {
        (blended * 100.0).round() as i64
    } else {
        999_999
    };
    let price_rank = if cost_tier == "standard" {
        base_rank
    } else {
        base_rank + 10_000
    };
    RankedPricing {
        blended_per_1m,
        cost_tier: cost_tier.to_string(),
        price_rank,
    }
}

fn parse_price(value: &Value) -> Option<f64> {
    match value {
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => text.parse::<f64>().ok().filter(|price| *price >= 0.0),
        Value::Number(number) => number.as_f64().filter(|price| *price >= 0.0),
        Value::Null => None,
        _ => None,
    }
}

fn blend(prompt_per_1m: Option<f64>, completion_per_1m: Option<f64>) -> Option<f64> {
    if prompt_per_1m.is_none() && completion_per_1m.is_none() {
        return None;
    }
    Some(prompt_per_1m.unwrap_or(0.0) * 0.4 + completion_per_1m.unwrap_or(0.0) * 0.6)
}
