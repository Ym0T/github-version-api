//! Router, middleware and rate limiting.
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::{GovernorError, GovernorLayer};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::CorsOrigins;
use crate::error::ApiError;
use crate::handlers;
use crate::net::ClientIpKeyExtractor;
use crate::state::AppState;

/// Every request needs a `ConnectInfo<SocketAddr>` extension (the server
/// adds it), rate limiting is keyed by the client address.
pub fn build_router(state: &Arc<AppState>) -> anyhow::Result<Router> {
    let config = &state.config;

    let governor = GovernorConfigBuilder::default()
        .period(config.rate_limit.period)
        .burst_size(config.rate_limit.burst)
        .key_extractor(ClientIpKeyExtractor::new(&config.trusted_proxies))
        .finish()
        .context("invalid rate limit configuration")?;
    let limiter = governor.limiter().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.retain_recent();
        }
    });
    let rate_limit = GovernorLayer::new(governor).error_handler(|error| match error {
        GovernorError::TooManyRequests { wait_time, headers } => {
            let mut response = ApiError::rate_limited(wait_time).into_response();
            if let Some(headers) = headers {
                response.headers_mut().extend(headers);
            }
            response
        }
        GovernorError::UnableToExtractKey => {
            ApiError::internal("rate limiting", "cannot determine the client address")
                .into_response()
        }
        GovernorError::Other { code, .. } => {
            ApiError::new(code, "request_rejected", "The request was rejected").into_response()
        }
    });

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
        .allow_headers([header::ACCEPT, header::CONTENT_TYPE])
        .max_age(Duration::from_secs(3600));
    let cors = match &config.cors {
        CorsOrigins::Any => cors.allow_origin(AllowOrigin::any()),
        CorsOrigins::List(origins) => cors.allow_origin(AllowOrigin::list(origins.clone())),
    };

    Ok(Router::new()
        .route("/", get(handlers::health::root))
        .route("/health", get(handlers::health::health_check))
        .route("/stats", get(handlers::stats::get_stats))
        .route("/version", get(handlers::version::get_version))
        .route(
            "/version/{repo}",
            get(handlers::version::get_version_by_repo),
        )
        .route(
            "/version/{owner}/{repo}",
            get(handlers::version::get_version_by_owner_repo),
        )
        .route(
            "/diff/{owner}/{repo}/{from_version}/{to_version}",
            get(handlers::diff::get_version_diff),
        )
        .route(
            "/diff/{repo}/{from_version}/{to_version}",
            get(handlers::diff::get_version_diff_default_owner),
        )
        .route(
            "/download/diff/{filename}",
            get(handlers::download::download_diff_file),
        )
        .fallback(handlers::not_found)
        .method_not_allowed_fallback(handlers::method_not_allowed)
        .with_state(Arc::clone(state))
        // Outermost layer last: headers are added to every response, including
        // rate limit rejections, timeouts and CORS preflights.
        .layer(rate_limit)
        .layer(middleware::from_fn_with_state(
            config.request_timeout,
            request_timeout,
        ))
        .layer(cors)
        .layer(middleware::from_fn(security_headers)))
}

/// Limits the time until the response starts (not the body transfer).
async fn request_timeout(State(limit): State<Duration>, request: Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_) => ApiError::unavailable(
            "request_timeout",
            "The request took too long, please retry shortly",
            Some(5),
        )
        .into_response(),
    }
}

const SECURITY_HEADERS: [(HeaderName, &str); 6] = [
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (header::X_FRAME_OPTIONS, "DENY"),
    (header::REFERRER_POLICY, "no-referrer"),
    (
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    ),
    (header::STRICT_TRANSPORT_SECURITY, "max-age=31536000"),
    (
        HeaderName::from_static("permissions-policy"),
        "geolocation=(), microphone=(), camera=()",
    ),
];

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in SECURITY_HEADERS {
        headers
            .entry(name)
            .or_insert(HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::config::AppConfig;
    use crate::test_support::{TestApp, error_code, offline_github};

    async fn app(extra: &[(&str, &str)]) -> TestApp {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_string_lossy().into_owned();
        let mut vars = vec![
            ("DATA_DIR", data_dir.as_str()),
            ("RATE_LIMIT_PER_MINUTE", "1"),
            ("RATE_LIMIT_BURST", "5"),
        ];
        vars.extend_from_slice(extra);
        let config = AppConfig::for_tests(&vars).unwrap();
        let state = AppState::with_github(config, None, offline_github())
            .await
            .unwrap();
        TestApp::new(state, dir)
    }

    #[tokio::test]
    async fn responses_carry_security_headers() {
        let app = app(&[]).await;
        let (status, headers, _) = app.send("/health", "192.0.2.1:1", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
        assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY));

        let (status, headers, body) = app.send("/nope", "192.0.2.1:1", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error_code(&body), "not_found");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/health")
            .body(axum::body::Body::empty())
            .unwrap();
        let (status, _, body) = app.request(request, "192.0.2.1:1").await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            error_code(&String::from_utf8(body).unwrap()),
            "method_not_allowed"
        );
    }

    #[tokio::test]
    async fn rejects_path_manipulation_before_calling_github() {
        let app = app(&[]).await;
        for (uri, code) in [
            ("/version?owner=x&repo=..%2F..%2Fuser%3F", "invalid_repo"),
            ("/version?owner=..%2F..&repo=x", "invalid_owner"),
            ("/version?owner=a&owner=b", "invalid_query"),
            ("/version/Ym0T/..%2F..%2Fuser", "invalid_repo"),
            ("/version/Ym0T/%FF", "invalid_path"),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/%252e%252e/v1",
                "invalid_version",
            ),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/v1%2F..%2F..%2Fuser%3F/v2",
                "invalid_version",
            ),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/v1..v2/v3",
                "invalid_version",
            ),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/v1%3Brm/v3",
                "invalid_version",
            ),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/v1/v1",
                "identical_versions",
            ),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/v1/v2?zip=yes",
                "invalid_zip_flag",
            ),
            (
                "/diff/Ym0T/pterodactyl-nginx-egg/v1/v2?format=xml",
                "invalid_format",
            ),
        ] {
            let (status, body) = app.get(uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(error_code(&body), code, "{uri}");
            assert!(!body.contains("user"), "input must not be echoed: {body}");
        }
    }

    #[tokio::test]
    async fn only_allowed_repositories_are_served() {
        let app = app(&[]).await;
        for uri in [
            "/version/someone/else",
            "/version?owner=someone&repo=else",
            "/version/other-repo",
            "/diff/attacker/evil/v1.0/v1.1?zip=true",
            "/diff/attacker/evil/v1.0/v1.1",
        ] {
            let (status, body) = app.get(uri).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
            assert_eq!(error_code(&body), "repository_not_allowed", "{uri}");
        }
    }

    #[tokio::test]
    async fn zip_requires_a_signing_key() {
        let app = app(&[]).await;
        let (status, body) = app
            .get("/diff/Ym0T/pterodactyl-nginx-egg/v1.0/v1.1?zip=true")
            .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_code(&body), "signing_unavailable");
    }

    #[tokio::test]
    async fn downloads_are_confined_to_generated_archives() {
        let app = app(&[]).await;
        let dir = &app.state.config.diffs_dir;

        for uri in [
            "/download/diff/..%2F..%2Fetc%2Fpasswd",
            "/download/diff/diff-Ym0T-repo-v1-v2-abcd1234.zip",
            "/download/diff/DIFF-0123456789abcdef0123456789abcdef.zip",
        ] {
            let (status, body) = app.get(uri).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(error_code(&body), "invalid_filename");
        }

        let missing = "/download/diff/diff-0123456789abcdef0123456789abcdef.zip";
        assert_eq!(app.get(missing).await.0, StatusCode::NOT_FOUND);

        let name = "diff-fedcba9876543210fedcba9876543210.zip";
        std::fs::write(dir.join(name), b"PK-archive").unwrap();
        let (status, headers, body) = app
            .send(&format!("/download/diff/{name}"), "192.0.2.1:1", None)
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "application/zip");
        assert_eq!(body, "PK-archive");

        #[cfg(unix)]
        {
            let secret = app.dir.path().join("secret");
            std::fs::write(&secret, b"secret").unwrap();
            let link = "diff-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.zip";
            std::os::unix::fs::symlink(&secret, dir.join(link)).unwrap();
            assert_eq!(
                app.get(&format!("/download/diff/{link}")).await.0,
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn rate_limit_is_per_client_ignores_spoofed_headers_and_answers_json() {
        let app = app(&[]).await;
        for _ in 0..5 {
            let (status, _, _) = app
                .send("/health", "198.51.100.1:1", Some("10.0.0.1"))
                .await;
            assert_eq!(status, StatusCode::OK);
        }
        // A different X-Forwarded-For does not create a new bucket because
        // the peer is not a trusted proxy.
        let (status, headers, body) = app
            .send("/health", "198.51.100.1:2", Some("10.9.9.9"))
            .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error_code(&body), "rate_limited");
        assert!(headers.contains_key(header::RETRY_AFTER));
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

        let (status, _, _) = app.send("/health", "198.51.100.2:1", None).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn forwarded_header_is_used_behind_a_trusted_proxy() {
        let app = app(&[("TRUSTED_PROXIES", "10.0.0.1")]).await;
        for _ in 0..5 {
            let (status, _, _) = app.send("/health", "10.0.0.1:1", Some("203.0.113.1")).await;
            assert_eq!(status, StatusCode::OK);
        }
        assert_eq!(
            app.send("/health", "10.0.0.1:1", Some("203.0.113.1"))
                .await
                .0,
            StatusCode::TOO_MANY_REQUESTS
        );
        // Another client behind the same proxy has its own bucket.
        assert_eq!(
            app.send("/health", "10.0.0.1:1", Some("203.0.113.2"))
                .await
                .0,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn cors_is_off_by_default_and_configurable() {
        let app = app(&[]).await;
        let (_, headers, _) = app
            .send_with_origin("/health", "https://evil.example")
            .await;
        assert!(!headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));

        let app = self::app(&[("CORS_ALLOWED_ORIGINS", "https://ok.example")]).await;
        let (_, headers, _) = app.send_with_origin("/health", "https://ok.example").await;
        assert_eq!(
            headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://ok.example"
        );
    }
}
