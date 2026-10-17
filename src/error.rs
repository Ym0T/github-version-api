//! Typed API errors.
//!
//! Every error that reaches a client goes through [`ApiError`]. Messages are
//! static or built only from validated input, so internal details (upstream
//! responses, file paths, I/O errors) never leak into responses. Details are
//! kept in a private field and logged together with the request id.

use std::borrow::Cow;
use std::fmt;

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tracing::{error, warn};

use crate::models::responses::ErrorResponse;

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: Cow<'static, str>,
    retry_after: Option<u64>,
    /// Internal detail for the log, never sent to clients.
    detail: Option<String>,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            retry_after: None,
            detail: None,
        }
    }

    pub fn bad_request(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn forbidden(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::FORBIDDEN, code, message)
    }

    pub fn not_found(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn unprocessable(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    pub fn bad_gateway(code: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, code, message)
    }

    pub fn unavailable(
        code: &'static str,
        message: impl Into<Cow<'static, str>>,
        retry_after: Option<u64>,
    ) -> Self {
        Self {
            retry_after,
            ..Self::new(StatusCode::SERVICE_UNAVAILABLE, code, message)
        }
    }

    /// An unexpected failure. The client only gets a generic message.
    pub fn internal(context: &str, err: impl fmt::Display) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An internal error occurred",
        )
        .with_detail(format!("{context}: {err}"))
    }

    pub fn rate_limited(retry_after: u64) -> Self {
        Self {
            retry_after: Some(retry_after.max(1)),
            ..Self::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many requests, please slow down",
            )
        }
    }

    /// Attaches a detail that is logged but not returned.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Same error with a more specific code.
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = code;
        self
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.code, self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<PathRejection> for ApiError {
    fn from(rejection: PathRejection) -> Self {
        Self::bad_request("invalid_path", "The request path is invalid")
            .with_detail(rejection.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> Self {
        Self::bad_request("invalid_query", "The query string is invalid")
            .with_detail(rejection.body_text())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorResponse::new(self.code, self.message.into_owned());
        if self.status.is_server_error() {
            match &self.detail {
                Some(detail) => error!(request_id = %body.request_id, code = self.code, "{detail}"),
                None => {
                    warn!(request_id = %body.request_id, code = self.code, status = %self.status, "request failed");
                }
            }
        } else if let Some(detail) = &self.detail {
            tracing::debug!(request_id = %body.request_id, code = self.code, "{detail}");
        }

        let mut response = (self.status, Json(body)).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if let Some(seconds) = self.retry_after {
            headers.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}
