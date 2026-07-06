use std::{convert::Infallible, env, pin::Pin, sync::Arc, time::Duration};

use axum::{
    body::{Body, Bytes},
    extract::{multipart::MultipartRejection, Multipart, State},
    http::{header, HeaderMap, HeaderValue, Method, Request, Response, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{get, options, post},
    Json, Router,
};
use futures_util::{stream, Stream, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    errors::GatewayError,
    health::{HealthRegistry, HealthUpdate},
    runtime_catalog::RuntimeCatalog,
    selector::{is_auto_route, resolve_auto_candidates, resolve_model},
    sync::{self, SyncRegistry},
    types::{AppConfig, Endpoint, ModelConfig, SelectedModel},
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub health: HealthRegistry,
    pub catalog: RuntimeCatalog,
    pub client: reqwest::Client,
    pub sync: SyncRegistry,
}

impl AppState {
    pub fn new(config: AppConfig, client: reqwest::Client) -> Self {
        let config = Arc::new(config);
        let health = HealthRegistry::new(&config);
        let catalog = RuntimeCatalog::new(config.clone(), health.clone());
        let sync = SyncRegistry::new(&config);
        Self {
            config,
            health,
            catalog,
            client,
            sync,
        }
    }

    pub fn start_background_tasks(&self) {
        self.health
            .start((*self.config).clone(), self.client.clone());
        if self.config.openrouter_sync.enabled {
            let state = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                loop {
                    let _ = sync::sync_openrouter(
                        &state.config,
                        &state.catalog,
                        &state.sync,
                        &state.client,
                    )
                    .await;
                    tokio::time::sleep(Duration::from_secs(
                        state.config.openrouter_sync.interval_seconds,
                    ))
                    .await;
                }
            });
        }
        if self.config.deepseek_sync.enabled {
            let state = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                loop {
                    let _ = sync::sync_deepseek(
                        &state.config,
                        &state.catalog,
                        &state.sync,
                        &state.client,
                    )
                    .await;
                    tokio::time::sleep(Duration::from_secs(
                        state.config.deepseek_sync.interval_seconds,
                    ))
                    .await;
                }
            });
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/models", get(models_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .route("/v1/embeddings", post(embeddings_handler))
        .route("/v1/audio/speech", post(audio_speech_handler))
        .route(
            "/v1/audio/transcriptions",
            post(audio_transcriptions_handler),
        )
        .route("/v1/translations", post(translations_handler))
        .route("/tailgate/health", get(health_handler))
        .route("/tailgate/config", get(config_handler))
        .route("/tailgate/sync/openrouter", post(sync_openrouter_handler))
        .route("/tailgate/sync/deepseek", post(sync_deepseek_handler))
        .route("/*path", options(options_handler))
        .fallback(not_found_handler)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, cors_middleware))
}

async fn cors_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> axum::response::Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let cors_origin =
        allowed_cors_origin(&state.config.server.cors_allowed_origins, origin.as_deref());
    if req.method() == Method::OPTIONS {
        let mut response = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .expect("valid CORS response");
        apply_cors_headers(response.headers_mut(), cors_origin);
        return response.into_response();
    }
    let mut response = next.run(req).await;
    apply_cors_headers(response.headers_mut(), cors_origin);
    response
}

async fn options_handler() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn not_found_handler() -> impl IntoResponse {
    GatewayError::new("Not found", 404, "not_found")
}

async fn models_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, GatewayError> {
    require_router_auth(&headers)?;
    Ok(Json(models_response(&state)))
}

async fn health_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, GatewayError> {
    require_router_auth(&headers)?;
    let model_refs = state
        .catalog
        .get_model_entries()
        .into_iter()
        .map(|(id, model, _)| (id, model.provider, model.endpoint))
        .collect();
    Ok(Json(json!({
        "object": "tailgate.health",
        "data": state.health.snapshot(model_refs),
        "openrouter_sync": state.sync.openrouter_status(),
        "deepseek_sync": state.sync.deepseek_status(),
    })))
}

async fn config_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, GatewayError> {
    require_router_auth(&headers)?;
    let mut config = serde_json::to_value(&*state.config)
        .map_err(|error| GatewayError::new(error.to_string(), 500, "internal_error"))?;
    if let Some(object) = config.as_object_mut() {
        object.insert(
            "runtime_overlay".to_string(),
            serde_json::to_value(state.catalog.get_runtime_metadata_summary())
                .map_err(|error| GatewayError::new(error.to_string(), 500, "internal_error"))?,
        );
        object.insert(
            "virtual_model_ids".to_string(),
            serde_json::to_value(state.catalog.get_virtual_model_ids())
                .map_err(|error| GatewayError::new(error.to_string(), 500, "internal_error"))?,
        );
    }
    Ok(Json(config))
}

async fn sync_openrouter_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, GatewayError> {
    require_router_auth(&headers)?;
    let summary = sync::sync_openrouter(&state.config, &state.catalog, &state.sync, &state.client)
        .await
        .map_err(|error| GatewayError::new(error.to_string(), 502, "openrouter_sync_failed"))?;
    Ok(Json(summary))
}

async fn sync_deepseek_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, GatewayError> {
    require_router_auth(&headers)?;
    let summary = sync::sync_deepseek(&state.config, &state.catalog, &state.sync, &state.client)
        .await
        .map_err(|error| GatewayError::new(error.to_string(), 502, "deepseek_sync_failed"))?;
    Ok(Json(summary))
}

async fn chat_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, GatewayError> {
    proxy_json(state, headers, body, EndpointSpec::chat()).await
}

async fn embeddings_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, GatewayError> {
    proxy_json(state, headers, body, EndpointSpec::embeddings()).await
}

async fn audio_speech_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, GatewayError> {
    proxy_json(state, headers, body, EndpointSpec::audio_speech()).await
}

async fn translations_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, GatewayError> {
    proxy_json(state, headers, body, EndpointSpec::translations()).await
}

async fn audio_transcriptions_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<axum::response::Response, GatewayError> {
    require_router_auth(&headers)?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.contains("multipart/form-data") {
        return Err(GatewayError::new(
            "Request must be multipart/form-data",
            400,
            "invalid_content_type",
        ));
    }
    let mut multipart = multipart.map_err(|_| {
        GatewayError::new(
            "Request must be multipart/form-data",
            400,
            "invalid_content_type",
        )
    })?;
    let mut fields = Vec::new();
    let mut requested_model = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| GatewayError::new(error.to_string(), 400, "invalid_multipart"))?
    {
        let name = field
            .name()
            .ok_or_else(|| {
                GatewayError::new(
                    "Multipart field must include a name",
                    400,
                    "invalid_multipart",
                )
            })?
            .to_string();
        let file_name = field.file_name().map(ToOwned::to_owned);
        let content_type = field.content_type().map(ToOwned::to_owned);
        let bytes = field
            .bytes()
            .await
            .map_err(|error| GatewayError::new(error.to_string(), 400, "invalid_multipart"))?;
        if name == "model" {
            requested_model = Some(
                std::str::from_utf8(&bytes)
                    .map_err(|_| {
                        GatewayError::new(
                            "Request body must include a model",
                            400,
                            "model_required",
                        )
                    })?
                    .to_string(),
            );
        }
        fields.push(MultipartField {
            name,
            file_name,
            content_type,
            bytes,
        });
    }
    let requested_model = requested_model
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            GatewayError::new("Request body must include a model", 400, "model_required")
        })?;
    let request_body = ParsedRequestBody::Multipart { fields };
    proxy_request(
        state,
        headers,
        EndpointSpec::audio_transcriptions(),
        request_body,
        requested_model,
        false,
        None,
    )
    .await
}

async fn proxy_json(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
    spec: EndpointSpec,
) -> Result<axum::response::Response, GatewayError> {
    require_router_auth(&headers)?;
    let parsed: Value = serde_json::from_slice(&body)
        .map_err(|_| GatewayError::new("Invalid JSON request body", 400, "invalid_json"))?;
    if !parsed.is_object() {
        return Err(GatewayError::new(
            "Request body must be a JSON object",
            400,
            "invalid_request",
        ));
    }
    let requested_model = parsed
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            GatewayError::new("Request body must include a model", 400, "model_required")
        })?
        .to_string();
    let stream =
        spec.supports_stream && parsed.get("stream").and_then(Value::as_bool) == Some(true);
    proxy_request(
        state,
        headers,
        spec,
        ParsedRequestBody::Json(parsed.clone()),
        requested_model,
        stream,
        Some(parsed),
    )
    .await
}

async fn proxy_request(
    state: AppState,
    headers: HeaderMap,
    spec: EndpointSpec,
    request_body: ParsedRequestBody,
    requested_model: String,
    stream: bool,
    json_body: Option<Value>,
) -> Result<axum::response::Response, GatewayError> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let candidates = candidate_models(
        &state,
        &requested_model,
        spec.endpoint.clone(),
        json_body.as_ref(),
    )?;
    let mut last_error = None;
    for (index, selected) in candidates.iter().enumerate() {
        match forward_to_selected(
            &state,
            selected,
            &spec,
            &request_body,
            stream,
            &requested_model,
            index > 0,
        )
        .await
        {
            Ok(response) => {
                info!(
                    request_id = request_id,
                    endpoint = ?spec.endpoint,
                    requested_model = requested_model,
                    selected_model = selected.name,
                    provider = selected.config.provider,
                    upstream_model = selected.config.upstream_model,
                    stream = stream,
                    status = response.status().as_u16(),
                    fallback_count = index,
                    "request completed"
                );
                return Ok(response);
            }
            Err(error) => {
                let can_fallback =
                    is_auto_route(&state.config, &requested_model) && index + 1 < candidates.len();
                if !can_fallback {
                    return Err(error);
                }
                warn!(
                    request_id = request_id,
                    requested_model = requested_model,
                    selected_model = selected.name,
                    provider = selected.config.provider,
                    upstream_model = selected.config.upstream_model,
                    endpoint = ?spec.endpoint,
                    fallback_count = index + 1,
                    error_code = error.code,
                    "fallback to next candidate"
                );
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| GatewayError::upstream("No model candidate succeeded")))
}

fn candidate_models(
    state: &AppState,
    requested_model: &str,
    endpoint: Endpoint,
    body: Option<&Value>,
) -> Result<Vec<SelectedModel>, GatewayError> {
    if is_auto_route(&state.config, requested_model) {
        return resolve_auto_candidates(
            &state.config,
            &state.health,
            requested_model,
            endpoint,
            body,
            Some(&state.catalog),
        )
        .map_err(|error| {
            let message = error.to_string();
            if message.starts_with("No healthy model") {
                GatewayError::new(message, 503, "no_healthy_model")
            } else if message.contains("does not support endpoint") {
                GatewayError::new(message, 400, "endpoint_mismatch")
            } else {
                GatewayError::new(message, 404, "model_not_found")
            }
        });
    }

    let selected = resolve_model(
        &state.config,
        &state.health,
        requested_model,
        Some(&state.catalog),
    )
    .map_err(|error| GatewayError::new(error.to_string(), 404, "model_not_found"))?;
    if selected.config.endpoint != endpoint {
        return Err(GatewayError::new(
            format!(
                "Model {requested_model} does not support endpoint {:?}",
                endpoint
            ),
            400,
            "endpoint_mismatch",
        ));
    }
    Ok(vec![selected])
}

async fn forward_to_selected(
    state: &AppState,
    selected: &SelectedModel,
    spec: &EndpointSpec,
    request_body: &ParsedRequestBody,
    stream: bool,
    requested_model: &str,
    is_fallback: bool,
) -> Result<axum::response::Response, GatewayError> {
    let api_key = env::var(&selected.config.api_key_env).map_err(|_| {
        GatewayError::new(
            format!(
                "Provider API key is not configured for model: {}",
                selected.name
            ),
            500,
            "provider_key_not_configured",
        )
    })?;
    let started = std::time::Instant::now();
    state.health.begin_request(&selected.name);
    let request = build_upstream_request(state, selected, spec, request_body, &api_key)?;
    let response = tokio::time::timeout(
        Duration::from_millis(state.config.server.request_timeout_ms),
        request.send(),
    )
    .await
    .map_err(|error| {
        finish_request_error(state, &selected.name, started, &error.to_string());
        GatewayError::upstream(error.to_string())
    })?
    .map_err(|error| {
        finish_request_error(state, &selected.name, started, &error.to_string());
        GatewayError::upstream(error.to_string())
    })?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        finish_request_error(
            state,
            &selected.name,
            started,
            &format!("upstream returned {status}"),
        );
        return Err(GatewayError::new(
            format!("Upstream {} returned {status}", selected.name),
            if status >= 500 { 502 } else { status },
            "upstream_error",
        ));
    }

    let status = response.status();
    let mut builder = Response::builder().status(status);
    {
        let headers = builder
            .headers_mut()
            .expect("response builder headers exist");
        let filtered = filtered_upstream_headers(response.headers());
        for (key, value) in filtered.iter() {
            headers.insert(key.clone(), value.clone());
        }
        headers.insert(
            "X-Tailgate-Model",
            HeaderValue::from_str(&selected.name).unwrap(),
        );
        headers.insert(
            "X-Tailgate-Provider",
            HeaderValue::from_str(&selected.config.provider).unwrap(),
        );
        headers.insert(
            "X-Tailgate-Route",
            HeaderValue::from_str(requested_model).unwrap(),
        );
        headers.insert(
            "X-Tailgate-Fallback",
            HeaderValue::from_static(if is_fallback { "true" } else { "false" }),
        );
    }

    let body = measured_body(
        response.bytes_stream(),
        state.health.clone(),
        selected.name.clone(),
        started,
        stream,
    );
    builder
        .body(Body::from_stream(body))
        .map_err(|error| GatewayError::new(error.to_string(), 500, "internal_error"))
}

fn build_upstream_request(
    state: &AppState,
    selected: &SelectedModel,
    spec: &EndpointSpec,
    request_body: &ParsedRequestBody,
    api_key: &str,
) -> Result<reqwest::RequestBuilder, GatewayError> {
    let url = format!("{}{}", selected.config.base_url, spec.upstream_path);
    let builder = state.client.post(url).bearer_auth(api_key);
    match request_body {
        ParsedRequestBody::Json(json_body) => {
            let mut body = json_body.clone();
            body.as_object_mut()
                .expect("JSON request body object")
                .insert(
                    "model".to_string(),
                    Value::String(selected.config.upstream_model.clone()),
                );
            Ok(builder
                .header(header::CONTENT_TYPE, "application/json")
                .json(&body))
        }
        ParsedRequestBody::Multipart { fields } => {
            let mut form = reqwest::multipart::Form::new();
            for field in fields {
                if field.name == "model" {
                    form = form.part(
                        field.name.clone(),
                        reqwest::multipart::Part::text(selected.config.upstream_model.clone()),
                    );
                    continue;
                }
                let bytes = field.bytes.clone();
                let mut part = reqwest::multipart::Part::bytes(bytes.to_vec());
                if let Some(file_name) = &field.file_name {
                    part = part.file_name(file_name.clone());
                }
                if let Some(content_type) = &field.content_type {
                    part = part.mime_str(content_type).map_err(|error| {
                        GatewayError::new(error.to_string(), 400, "invalid_multipart")
                    })?;
                }
                form = form.part(field.name.clone(), part);
            }
            Ok(builder.multipart(form))
        }
    }
}

fn measured_body(
    upstream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    health: HealthRegistry,
    model_name: String,
    started: std::time::Instant,
    measure_first_token: bool,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let stream: Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>> =
        Box::pin(upstream);
    stream::unfold(
        (
            stream,
            false,
            false,
            health,
            model_name,
            started,
            measure_first_token,
        ),
        |(mut stream, measured, finished, health, model_name, started, measure_first_token)| async move {
            if finished {
                return None;
            }
            match stream.as_mut().next().await {
                Some(Ok(bytes)) => {
                    let measured_now = if !measured && measure_first_token {
                        let latency = started.elapsed().as_millis() as u64;
                        health.record_first_token(&model_name, latency);
                        true
                    } else {
                        measured
                    };
                    Some((
                        Ok(bytes),
                        (
                            stream,
                            measured_now,
                            false,
                            health,
                            model_name,
                            started,
                            measure_first_token,
                        ),
                    ))
                }
                Some(Err(error)) => {
                    health.finish_request(
                        &model_name,
                        HealthUpdate {
                            ok: false,
                            total_latency_ms: started.elapsed().as_millis() as u64,
                            first_token_latency_ms: None,
                            error: Some(error.to_string()),
                        },
                    );
                    Some((
                        Err(std::io::Error::other(error.to_string())),
                        (
                            stream,
                            measured,
                            true,
                            health,
                            model_name,
                            started,
                            measure_first_token,
                        ),
                    ))
                }
                None => {
                    health.finish_request(
                        &model_name,
                        HealthUpdate {
                            ok: true,
                            total_latency_ms: started.elapsed().as_millis() as u64,
                            first_token_latency_ms: None,
                            error: None,
                        },
                    );
                    None
                }
            }
        },
    )
}

fn finish_request_error(
    state: &AppState,
    model_name: &str,
    started: std::time::Instant,
    error: &str,
) {
    state.health.finish_request(
        model_name,
        HealthUpdate {
            ok: false,
            total_latency_ms: started.elapsed().as_millis() as u64,
            first_token_latency_ms: None,
            error: Some(error.to_string()),
        },
    );
}

fn require_router_auth(headers: &HeaderMap) -> Result<(), GatewayError> {
    let expected = env::var("ROUTER_API_KEY").map_err(|_| {
        GatewayError::new(
            "ROUTER_API_KEY is not configured",
            500,
            "router_key_not_configured",
        )
    })?;
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let Some(authorization) = authorization else {
        return Err(GatewayError::new(
            "Missing or invalid Authorization header",
            401,
            "unauthorized",
        ));
    };
    let Some(token) = authorization.strip_prefix("Bearer ") else {
        return Err(GatewayError::new(
            "Missing or invalid Authorization header",
            401,
            "unauthorized",
        ));
    };
    if token != expected {
        return Err(GatewayError::new("Invalid API key", 401, "unauthorized"));
    }
    Ok(())
}

pub fn filtered_upstream_headers(headers: &HeaderMap) -> HeaderMap {
    let blocked = [
        "connection",
        "content-encoding",
        "content-length",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];
    let mut result = HeaderMap::new();
    for (key, value) in headers {
        if !blocked.contains(&key.as_str().to_ascii_lowercase().as_str()) {
            result.insert(key.clone(), value.clone());
        }
    }
    result
}

fn models_response(state: &AppState) -> Value {
    let model_entries = state.catalog.get_model_entries();
    let mut ids: Vec<_> = model_entries
        .iter()
        .map(|(id, _, _)| id.clone())
        .chain(state.config.routes.keys().cloned())
        .collect();
    ids.sort();
    let data: Vec<_> = ids
        .into_iter()
        .map(|id| {
            let model = model_entries
                .iter()
                .find(|(model_id, _, _)| model_id == &id);
            let route = state.config.routes.get(&id);
            let mut value = json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": "tailgate",
            });
            let object = value.as_object_mut().expect("object");
            if let Some((model_id, model, _)) = model {
                object.insert("provider".to_string(), json!(model.provider));
                object.insert("upstream_model".to_string(), json!(model.upstream_model));
                object.insert("context_window".to_string(), json!(model.context_window));
                object.insert(
                    "cost_tier".to_string(),
                    json!(display_cost_tier(model_id, model)),
                );
                object.insert("price_rank".to_string(), json!(model.price_rank));
            }
            if let Some(route) = route {
                object.insert("endpoint".to_string(), json!(route.endpoint));
                object.insert("cost_tier".to_string(), json!(route.cost_tier));
                object.insert("route".to_string(), json!(true));
            }
            value
        })
        .collect();
    json!({ "object": "list", "data": data })
}

fn display_cost_tier(model_name: &str, model: &ModelConfig) -> String {
    if let Some(tier) = &model.cost_tier {
        return tier.clone();
    }
    if model.provider == "local" {
        return "free".to_string();
    }
    if model.provider == "deepseek" {
        return "standard".to_string();
    }
    if model_name.ends_with("/premium") {
        return "premium".to_string();
    }
    "standard".to_string()
}

fn allowed_cors_origin(allowed_origins: &[String], origin: Option<&str>) -> Option<String> {
    let origin = origin?;
    if allowed_origins.iter().any(|allowed| allowed == "*")
        || allowed_origins.iter().any(|allowed| allowed == origin)
    {
        return Some(origin.to_string());
    }
    None
}

fn apply_cors_headers(headers: &mut HeaderMap, origin: Option<String>) {
    let Some(origin) = origin else {
        return;
    };
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(&origin).unwrap(),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    headers.insert(
        "Access-Control-Allow-Private-Network",
        HeaderValue::from_static("true"),
    );
}

#[derive(Clone)]
struct EndpointSpec {
    endpoint: Endpoint,
    upstream_path: &'static str,
    supports_stream: bool,
}

impl EndpointSpec {
    fn chat() -> Self {
        Self {
            endpoint: Endpoint::Chat,
            upstream_path: "/chat/completions",
            supports_stream: true,
        }
    }

    fn embeddings() -> Self {
        Self {
            endpoint: Endpoint::Embeddings,
            upstream_path: "/embeddings",
            supports_stream: false,
        }
    }

    fn audio_speech() -> Self {
        Self {
            endpoint: Endpoint::AudioSpeech,
            upstream_path: "/audio/speech",
            supports_stream: false,
        }
    }

    fn audio_transcriptions() -> Self {
        Self {
            endpoint: Endpoint::AudioTranscriptions,
            upstream_path: "/audio/transcriptions",
            supports_stream: false,
        }
    }

    fn translations() -> Self {
        Self {
            endpoint: Endpoint::Translations,
            upstream_path: "/translations",
            supports_stream: false,
        }
    }
}

#[derive(Clone)]
enum ParsedRequestBody {
    Json(Value),
    Multipart { fields: Vec<MultipartField> },
}

#[derive(Clone)]
struct MultipartField {
    name: String,
    file_name: Option<String>,
    content_type: Option<String>,
    bytes: Bytes,
}

#[allow(dead_code)]
async fn collect_body_text(body: Body) -> Result<String, Infallible> {
    let bytes = body
        .collect()
        .await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();
    Ok(String::from_utf8_lossy(&bytes).to_string())
}
