use axum::{
    body::Body,
    extract::{Multipart, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use http::{header, Request};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tailgate::app::{build_router, AppState};
use tailgate::config::load_config_from_value;
use tailgate::health::HealthUpdate;
use tower::ServiceExt;

#[tokio::test]
async fn cors_preflight_and_auth_match_node_behavior() {
    let state = test_state("http://127.0.0.1:1/v1");
    let app = build_router(state);

    let options = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/chat/completions")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "authorization,content-type",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(options.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        options
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "http://127.0.0.1:5173"
    );
    assert_eq!(
        options
            .headers()
            .get("access-control-allow-private-network")
            .unwrap(),
        "true"
    );

    let unauthorized = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/translations")
                .header(header::ORIGIN, "http://127.0.0.1:5173")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"model":"free/translation"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unauthorized
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "http://127.0.0.1:5173"
    );
}

#[tokio::test]
async fn translation_proxy_rewrites_model_and_sets_tailgate_headers() {
    std::env::set_var("ROUTER_API_KEY", "test-router-key");
    std::env::set_var("LOCAL_API_KEY", "local-test-key");

    let upstream = spawn_json_upstream().await;
    let state = test_state(&upstream.base_url);
    mark_healthy(&state, "local/translation");
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/translations")
                .header(header::AUTHORIZATION, "Bearer test-router-key")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"model":"free/translation","source_language":"en","target_language":"zh","text":"hello"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-tailgate-model").unwrap(),
        "local/translation"
    );
    assert_eq!(
        response.headers().get("x-tailgate-provider").unwrap(),
        "local"
    );
    assert_eq!(
        response.headers().get("x-tailgate-route").unwrap(),
        "free/translation"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"object":"translation","translation":"你好","source_language":"en","target_language":"zh"})
    );

    let call = upstream.calls.lock().await.remove(0);
    assert_eq!(call.path, "/v1/translations");
    assert_eq!(call.authorization, Some("Bearer local-test-key".into()));
    assert_eq!(
        call.body,
        json!({"model":"qwen-local-translation","source_language":"en","target_language":"zh","text":"hello"})
    );
}

#[tokio::test]
async fn streaming_chat_forwards_chunks_and_fallback_tries_next_candidate() {
    std::env::set_var("ROUTER_API_KEY", "test-router-key");
    std::env::set_var("DEEPSEEK_API_KEY", "deepseek-key");
    std::env::set_var("OPENROUTER_API_KEY", "openrouter-key");

    let upstream = spawn_fallback_stream_upstream().await;
    let state = fallback_state(&upstream.base_url);
    mark_healthy(&state, "deepseek/chat");
    mark_healthy(&state, "openrouter/auto");
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header(header::AUTHORIZATION, "Bearer test-router-key")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"model":"standard/chat","stream":true,"messages":[]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-tailgate-model").unwrap(),
        "openrouter/auto"
    );
    assert_eq!(
        response.headers().get("x-tailgate-fallback").unwrap(),
        "true"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"data: one\n\ndata: [DONE]\n\n");

    let calls = upstream.calls.lock().await.clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].body["model"], "deepseek-v4-flash");
    assert_eq!(calls[1].body["model"], "openrouter/auto");
}

#[tokio::test]
async fn multipart_transcription_preserves_file_fields_and_rewrites_model() {
    std::env::set_var("ROUTER_API_KEY", "test-router-key");
    std::env::set_var("LOCAL_API_KEY", "local-test-key");

    let upstream = spawn_multipart_upstream().await;
    let state = test_state(&upstream.base_url);
    mark_healthy(&state, "local/asr");
    let tailgate = spawn_app(build_router(state)).await;

    let form = reqwest::multipart::Form::new()
        .text("model", "local/asr")
        .text("language", "en")
        .part(
            "file",
            reqwest::multipart::Part::bytes(b"audio-bytes".to_vec())
                .file_name("sample.wav")
                .mime_str("audio/wav")
                .unwrap(),
        );

    let response = reqwest::Client::new()
        .post(format!("{}/audio/transcriptions", tailgate.base_url))
        .bearer_auth("test-router-key")
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("x-tailgate-model").unwrap(),
        "local/asr"
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(body, json!({"text":"hello"}));

    let call = upstream.multipart_calls.lock().await.remove(0);
    assert_eq!(call.model, "local-asr");
    assert_eq!(call.language, "en");
    assert_eq!(call.file_name, Some("sample.wav".into()));
    assert_eq!(call.content_type, Some("audio/wav".into()));
    assert_eq!(call.file_bytes, b"audio-bytes");
}

#[tokio::test]
async fn openrouter_and_deepseek_sync_update_runtime_catalog() {
    std::env::set_var("ROUTER_API_KEY", "test-router-key");
    std::env::set_var("OPENROUTER_API_KEY", "openrouter-key");
    std::env::set_var("DEEPSEEK_API_KEY", "deepseek-key");

    let upstream = spawn_sync_upstream().await;
    let state = sync_state(&upstream.base_url);
    let app = build_router(state.clone());

    let openrouter = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tailgate/sync/openrouter")
                .header(header::AUTHORIZATION, "Bearer test-router-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(openrouter.status(), StatusCode::OK);
    let openrouter_body = openrouter.into_body().collect().await.unwrap().to_bytes();
    let openrouter_json: Value = serde_json::from_slice(&openrouter_body).unwrap();
    assert_eq!(openrouter_json["updated_configured"], 1);
    assert_eq!(openrouter_json["added_virtual"], 1);

    let deepseek = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tailgate/sync/deepseek")
                .header(header::AUTHORIZATION, "Bearer test-router-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deepseek.status(), StatusCode::OK);
    let deepseek_body = deepseek.into_body().collect().await.unwrap().to_bytes();
    let deepseek_json: Value = serde_json::from_slice(&deepseek_body).unwrap();
    assert_eq!(deepseek_json["updated_configured"], 2);

    let config = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/tailgate/config")
                .header(header::AUTHORIZATION, "Bearer test-router-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let config_body = config.into_body().collect().await.unwrap().to_bytes();
    let config_json: Value = serde_json::from_slice(&config_body).unwrap();
    assert_eq!(
        config_json["virtual_model_ids"],
        json!(["openrouter/new-model"])
    );
    assert_eq!(
        config_json["runtime_overlay"]["deepseek/premium"]["dynamic_cost_tier"],
        "premium"
    );
}

#[derive(Clone)]
struct Upstream {
    base_url: String,
    calls: std::sync::Arc<tokio::sync::Mutex<Vec<Call>>>,
    multipart_calls: std::sync::Arc<tokio::sync::Mutex<Vec<MultipartCall>>>,
}

#[derive(Clone, Debug)]
struct Call {
    path: String,
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone, Debug, Default)]
struct MultipartCall {
    model: String,
    language: String,
    file_name: Option<String>,
    content_type: Option<String>,
    file_bytes: Vec<u8>,
}

async fn spawn_json_upstream() -> Upstream {
    let calls = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<Call>::new()));
    let app = Router::new()
        .route(
            "/v1/translations",
            post(
                move |State(calls): State<std::sync::Arc<tokio::sync::Mutex<Vec<Call>>>>,
                      headers: http::HeaderMap,
                      Json(body): Json<Value>| async move {
                    calls.lock().await.push(Call {
                        path: "/v1/translations".into(),
                        authorization: headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .map(ToOwned::to_owned),
                        body,
                    });
                    Json(json!({
                        "object": "translation",
                        "translation": "你好",
                        "source_language": "en",
                        "target_language": "zh"
                    }))
                },
            ),
        )
        .with_state(calls.clone());
    spawn_router(app, calls).await
}

async fn spawn_fallback_stream_upstream() -> Upstream {
    let calls = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<Call>::new()));
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                move |State(calls): State<std::sync::Arc<tokio::sync::Mutex<Vec<Call>>>>,
                      headers: http::HeaderMap,
                      Json(body): Json<Value>| async move {
                    let status = if body["model"] == "deepseek-v4-flash" {
                        StatusCode::BAD_GATEWAY
                    } else {
                        StatusCode::OK
                    };
                    calls.lock().await.push(Call {
                        path: "/v1/chat/completions".into(),
                        authorization: headers
                            .get(header::AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .map(ToOwned::to_owned),
                        body,
                    });
                    if status == StatusCode::OK {
                        (
                            status,
                            [(header::CONTENT_TYPE, "text/event-stream")],
                            "data: one\n\ndata: [DONE]\n\n",
                        )
                            .into_response()
                    } else {
                        (status, "upstream failed").into_response()
                    }
                },
            ),
        )
        .with_state(calls.clone());
    spawn_router(app, calls).await
}

async fn spawn_router(
    app: Router,
    calls: std::sync::Arc<tokio::sync::Mutex<Vec<Call>>>,
) -> Upstream {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Upstream {
        base_url: format!("http://{addr}/v1"),
        calls,
        multipart_calls: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
    }
}

async fn spawn_app(app: Router) -> Upstream {
    spawn_router(
        app,
        std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
    )
    .await
}

async fn spawn_multipart_upstream() -> Upstream {
    let calls = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<Call>::new()));
    let multipart_calls = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<MultipartCall>::new()));
    let app = Router::new()
        .route(
            "/v1/audio/transcriptions",
            post(
                move |State(multipart_calls): State<
                    std::sync::Arc<tokio::sync::Mutex<Vec<MultipartCall>>>,
                >,
                      mut multipart: Multipart| async move {
                    let mut call = MultipartCall::default();
                    while let Some(field) = multipart.next_field().await.unwrap() {
                        let name = field.name().unwrap().to_string();
                        let file_name = field.file_name().map(ToOwned::to_owned);
                        let content_type = field.content_type().map(ToOwned::to_owned);
                        let bytes = field.bytes().await.unwrap();
                        match name.as_str() {
                            "model" => call.model = String::from_utf8(bytes.to_vec()).unwrap(),
                            "language" => {
                                call.language = String::from_utf8(bytes.to_vec()).unwrap()
                            }
                            "file" => {
                                call.file_name = file_name;
                                call.content_type = content_type;
                                call.file_bytes = bytes.to_vec();
                            }
                            _ => {}
                        }
                    }
                    multipart_calls.lock().await.push(call);
                    Json(json!({"text":"hello"}))
                },
            ),
        )
        .with_state(multipart_calls.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Upstream {
        base_url: format!("http://{addr}/v1"),
        calls,
        multipart_calls,
    }
}

async fn spawn_sync_upstream() -> Upstream {
    let calls = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<Call>::new()));
    let app = Router::new()
        .route(
            "/v1/models",
            get(|| async {
                Json(json!({
                    "data": [
                        {
                            "id": "openrouter/auto",
                            "name": "OpenRouter Auto",
                            "context_length": 128000,
                            "pricing": {"prompt": "0.000001", "completion": "0.000002"},
                            "supported_parameters": ["temperature"],
                            "created": 1
                        },
                        {
                            "id": "new/model",
                            "name": "New Model",
                            "context_length": 64000,
                            "pricing": {"prompt": "0", "completion": "0"},
                            "supported_parameters": ["top_p"],
                            "created": 2
                        }
                    ]
                }))
            }),
        )
        .route(
            "/v1/deepseek-pricing",
            get(|| async {
                "CONTEXT LENGTH 64K deepseek-v4-flash deepseek-v4-pro 1M INPUT TOKENS (CACHE MISS) $0.10 $1.00 1M OUTPUT TOKENS $0.20 $2.00"
            }),
        );
    spawn_router(app, calls).await
}

fn test_state(local_base_url: &str) -> AppState {
    let config = load_config_from_value(json!({
        "server": {
            "host": "127.0.0.1",
            "port": 11435,
            "request_timeout_ms": 60000,
            "fallback_max_attempts": 2,
            "cors_allowed_origins": ["http://127.0.0.1:5173"]
        },
        "local": {
            "base_url": local_base_url,
            "api_key_env": "LOCAL_API_KEY",
            "translation_model": "qwen-local-translation",
            "max_concurrency": 1
        }
    }))
    .unwrap();
    AppState::new(config, reqwest::Client::new())
}

fn fallback_state(base_url: &str) -> AppState {
    let config = load_config_from_value(json!({
        "server": {
            "host": "127.0.0.1",
            "port": 11435,
            "request_timeout_ms": 60000,
            "fallback_max_attempts": 2,
            "cors_allowed_origins": ["*"]
        },
        "models": {
            "deepseek/chat": {
                "provider": "deepseek",
                "upstream_model": "deepseek-v4-flash",
                "base_url": base_url,
                "api_key_env": "DEEPSEEK_API_KEY",
                "endpoint": "chat",
                "cost_tier": "standard",
                "price_rank": 30
            },
            "openrouter/auto": {
                "provider": "openrouter",
                "upstream_model": "openrouter/auto",
                "base_url": base_url,
                "api_key_env": "OPENROUTER_API_KEY",
                "endpoint": "chat",
                "cost_tier": "standard",
                "price_rank": 30
            }
        }
    }))
    .unwrap();
    AppState::new(config, reqwest::Client::new())
}

fn sync_state(base_url: &str) -> AppState {
    let config = load_config_from_value(json!({
        "server": {
            "host": "127.0.0.1",
            "port": 11435,
            "request_timeout_ms": 60000,
            "fallback_max_attempts": 2,
            "cors_allowed_origins": ["*"]
        },
        "openrouter_sync": {
            "enabled": true,
            "interval_seconds": 21600,
            "update_config_file": false,
            "source_url": format!("{base_url}/models"),
            "include_unconfigured_models": false,
            "allowlist": ["openrouter/auto", "new/model"],
            "cost_tiers": {
                "free_max_usd_per_1m_tokens": 0,
                "standard_max_usd_per_1m_tokens": 2,
                "premium_max_usd_per_1m_tokens": 9999
            }
        },
        "deepseek_sync": {
            "enabled": true,
            "interval_seconds": 21600,
            "source_url": format!("{base_url}/deepseek-pricing"),
            "cost_tiers": {
                "free_max_usd_per_1m_tokens": 0,
                "standard_max_usd_per_1m_tokens": 2,
                "premium_max_usd_per_1m_tokens": 9999
            }
        },
        "models": {
            "openrouter/auto": {
                "provider": "openrouter",
                "upstream_model": "openrouter/auto",
                "base_url": base_url,
                "api_key_env": "OPENROUTER_API_KEY",
                "endpoint": "chat"
            },
            "deepseek/chat": {
                "provider": "deepseek",
                "upstream_model": "deepseek-v4-flash",
                "base_url": base_url,
                "api_key_env": "DEEPSEEK_API_KEY",
                "endpoint": "chat"
            },
            "deepseek/premium": {
                "provider": "deepseek",
                "upstream_model": "deepseek-v4-pro",
                "base_url": base_url,
                "api_key_env": "DEEPSEEK_API_KEY",
                "endpoint": "chat",
                "cost_tier": "premium"
            }
        }
    }))
    .unwrap();
    AppState::new(config, reqwest::Client::new())
}

fn mark_healthy(state: &AppState, model: &str) {
    state.health.finish_request(
        model,
        HealthUpdate {
            ok: true,
            total_latency_ms: 100,
            first_token_latency_ms: None,
            error: None,
        },
    );
    state.health.update_probe(model, true, 100, None);
}
