use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub struct GatewayError {
    pub message: String,
    pub status: u16,
    pub code: &'static str,
}

impl GatewayError {
    pub fn new(message: impl Into<String>, status: u16, code: &'static str) -> Self {
        Self {
            message: message.into(),
            status,
            code,
        }
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Self::new(message, 502, "upstream_error")
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (
            status,
            Json(json!({
                "error": {
                    "message": self.message,
                    "type": "gateway_error",
                    "code": self.code,
                }
            })),
        )
            .into_response()
    }
}
