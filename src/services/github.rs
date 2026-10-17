//! Minimal GitHub client.
//!
//! - Request URLs are assembled from validated parts with
//!   `path_segments_mut`, never with string formatting, so input can never
//!   change the path, query or host of a request.
//! - The token is only attached to API requests.
//! - Requests and redirects are limited to GitHub origins over HTTPS.
//! - All response bodies are size-limited and concurrent requests are capped.
//! - TLS uses the bundled Mozilla root store, independent of the host system.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use reqwest::header::{self, HeaderValue};
use reqwest::{Client, Response, StatusCode, Url, redirect};
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use tokio::sync::{Semaphore, SemaphorePermit};
use zeroize::Zeroizing;

use crate::config::{RepoRef, Secret};
use crate::error::ApiError;
use crate::models::github::{GhCompare, GhRelease, GhRepository, GhTag, GhTree};

const MAX_API_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TEXT_FILE_BYTES: u64 = 64 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_CONCURRENT_REQUESTS: usize = 8;
/// Larger JSON bodies are parsed on a blocking thread.
const INLINE_PARSE_LIMIT: usize = 256 * 1024;
const REQUEST_SLOT_WAIT: Duration = Duration::from_secs(30);
const TAGS_PER_PAGE: usize = 100;

#[derive(Debug)]
pub enum GitHubError {
    /// 404/410/451: missing, private or blocked.
    NotFound,
    /// 422: GitHub could not process the request (e.g. invalid ref).
    Unprocessable,
    RateLimited {
        retry_after: Option<u64>,
    },
    TooLarge,
    /// Downloaded content doesn't match the hash GitHub reported.
    ContentMismatch,
    /// Too many concurrent requests to GitHub from this instance.
    Busy,
    /// Anything else. The detail is logged, never returned to clients.
    Upstream(String),
}

impl From<GitHubError> for ApiError {
    fn from(err: GitHubError) -> Self {
        match err {
            GitHubError::NotFound => ApiError::not_found(
                "not_found",
                "The requested resource was not found on GitHub",
            ),
            GitHubError::Unprocessable => ApiError::unprocessable(
                "invalid_reference",
                "GitHub could not process the requested versions",
            ),
            GitHubError::RateLimited { retry_after } => ApiError::unavailable(
                "github_rate_limited",
                "The GitHub API rate limit is exhausted, please try again later",
                retry_after,
            ),
            GitHubError::TooLarge => ApiError::unprocessable(
                "upstream_too_large",
                "The response from GitHub exceeds the configured size limits",
            ),
            GitHubError::ContentMismatch => ApiError::bad_gateway(
                "content_verification_failed",
                "A downloaded file does not match the content hash reported by GitHub",
            ),
            GitHubError::Busy => ApiError::unavailable(
                "server_busy",
                "Too many requests to GitHub are in progress, please retry shortly",
                Some(5),
            ),
            GitHubError::Upstream(detail) => ApiError::bad_gateway(
                "github_unavailable",
                "The GitHub API is unavailable or returned an error",
            )
            .with_detail(format!("GitHub request failed: {detail}")),
        }
    }
}

/// Where the client sends requests. Production uses GitHub; tests point it
/// at a local fake.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub api: Url,
    pub raw: Url,
    /// Origins (`scheme://host[:port]`) requests and redirects may go to.
    pub allowed_origins: Vec<String>,
    pub https_only: bool,
}

impl Endpoints {
    pub fn github() -> Self {
        Self {
            api: Url::parse("https://api.github.com/").expect("static URL"),
            raw: Url::parse("https://raw.githubusercontent.com/").expect("static URL"),
            allowed_origins: [
                "https://api.github.com",
                "https://github.com",
                "https://raw.githubusercontent.com",
                "https://codeload.github.com",
            ]
            .map(String::from)
            .to_vec(),
            https_only: true,
        }
    }
}

pub struct GitHubClient {
    http: Client,
    authorization: Option<HeaderValue>,
    endpoints: Endpoints,
    allowed_origins: Arc<[String]>,
    slots: Semaphore,
}

impl GitHubClient {
    pub fn new(timeout: Duration, token: Option<&Secret>) -> anyhow::Result<Self> {
        Self::with_endpoints(timeout, token, Endpoints::github())
    }

    pub fn with_endpoints(
        timeout: Duration,
        token: Option<&Secret>,
        endpoints: Endpoints,
    ) -> anyhow::Result<Self> {
        // reqwest is built without a bundled crypto provider; use ring. This
        // fails harmlessly if the provider is already installed.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|der| reqwest::Certificate::from_der(der.as_ref()))
            .collect::<Result<Vec<_>, _>>()
            .context("cannot load bundled root certificates")?;

        let allowed_origins: Arc<[String]> = endpoints.allowed_origins.clone().into();
        let redirect_origins = Arc::clone(&allowed_origins);
        let http = Client::builder()
            .user_agent(concat!("version-api/", env!("CARGO_PKG_VERSION")))
            .https_only(endpoints.https_only)
            .tls_certs_only(roots)
            .redirect(redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if is_allowed_url(attempt.url(), &redirect_origins) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .connect_timeout(Duration::from_secs(10))
            .timeout(timeout)
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(8)
            .build()
            .context("cannot build HTTP client")?;

        let authorization = token
            .map(|token| {
                let value = Zeroizing::new(format!("Bearer {}", token.expose()));
                let mut header = HeaderValue::from_str(&value)
                    .context("GITHUB_TOKEN is not a valid header value")?;
                header.set_sensitive(true);
                anyhow::Ok(header)
            })
            .transpose()?;

        Ok(Self {
            http,
            authorization,
            endpoints,
            allowed_origins,
            slots: Semaphore::new(MAX_CONCURRENT_REQUESTS),
        })
    }

    /// Builds an API URL. Each segment is percent-encoded and can't
    /// introduce additional path segments, a query or a fragment.
    pub fn api_url(&self, segments: &[&str]) -> Url {
        let mut url = self.endpoints.api.clone();
        url.path_segments_mut()
            .expect("API base URL has a path")
            .pop_if_empty()
            .extend(segments);
        url
    }

    pub async fn repository(&self, repo: &RepoRef) -> Result<GhRepository, GitHubError> {
        self.get_json(self.api_url(&["repos", &repo.owner, &repo.name]))
            .await
    }

    pub async fn latest_release(&self, repo: &RepoRef) -> Result<Option<GhRelease>, GitHubError> {
        let url = self.api_url(&["repos", &repo.owner, &repo.name, "releases", "latest"]);
        not_found_as_none(self.get_json(url).await)
    }

    /// Tags with the commits they point to, up to `max_pages` pages of 100.
    pub async fn tags(&self, repo: &RepoRef, max_pages: u32) -> Result<Vec<GhTag>, GitHubError> {
        let mut tags = Vec::new();
        for page in 1..=max_pages {
            let mut url = self.api_url(&["repos", &repo.owner, &repo.name, "tags"]);
            url.query_pairs_mut()
                .append_pair("per_page", &TAGS_PER_PAGE.to_string())
                .append_pair("page", &page.to_string());
            let batch: Vec<GhTag> = match self.get_json(url).await {
                Ok(batch) => batch,
                Err(GitHubError::NotFound) => break,
                Err(err) => return Err(err),
            };
            let last_page = batch.len() < TAGS_PER_PAGE;
            tags.extend(batch);
            if last_page {
                break;
            }
        }
        Ok(tags)
    }

    pub async fn compare(
        &self,
        repo: &RepoRef,
        base: &str,
        head: &str,
    ) -> Result<GhCompare, GitHubError> {
        let range = format!("{base}...{head}");
        self.get_json(self.api_url(&["repos", &repo.owner, &repo.name, "compare", &range]))
            .await
    }

    /// The full file tree of a commit (paths, modes and blob SHAs).
    pub async fn tree(&self, repo: &RepoRef, commit: &str) -> Result<GhTree, GitHubError> {
        let mut url = self.api_url(&["repos", &repo.owner, &repo.name, "git", "trees", commit]);
        url.query_pairs_mut().append_pair("recursive", "1");
        self.get_json(url).await
    }

    /// Reads a small text file from a branch. Returns `None` if it doesn't exist.
    pub async fn raw_text(
        &self,
        repo: &RepoRef,
        branch: &str,
        file: &str,
    ) -> Result<Option<String>, GitHubError> {
        let mut url = self.endpoints.raw.clone();
        url.path_segments_mut()
            .expect("raw base URL has a path")
            .pop_if_empty()
            .push(&repo.owner)
            .push(&repo.name)
            .extend(branch.split('/'))
            .push(file);

        let _slot = self.slot().await?;
        let response = match check_status(self.send(self.http.get(url)).await?) {
            Ok(response) => response,
            Err(GitHubError::NotFound) => return Ok(None),
            Err(err) => return Err(err),
        };
        let body = read_limited(response, MAX_TEXT_FILE_BYTES).await?;
        Ok(Some(String::from_utf8_lossy(&body).into_owned()))
    }

    /// Downloads a file from a GitHub `raw_url` and verifies that its content
    /// matches the git blob SHA reported by GitHub.
    pub async fn download_blob(
        &self,
        raw_url: &str,
        expected_sha: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, GitHubError> {
        let url = Url::parse(raw_url)
            .ok()
            .filter(|url| is_allowed_url(url, &self.allowed_origins))
            .ok_or_else(|| {
                GitHubError::Upstream("compare API returned an unexpected download URL".into())
            })?;
        let _slot = self.slot().await?;
        let response = check_status(self.send(self.http.get(url)).await?)?;
        let body = read_limited(response, max_bytes).await?;

        // Hashing up to ZIP_MAX_FILE_MB is CPU work; keep it off the runtime.
        let (body, digest) = tokio::task::spawn_blocking(move || {
            let digest = git_blob_sha1(&body);
            (body, digest)
        })
        .await
        .map_err(|e| GitHubError::Upstream(format!("hashing failed: {e}")))?;
        if !digest.eq_ignore_ascii_case(expected_sha) {
            return Err(GitHubError::ContentMismatch);
        }
        Ok(body)
    }

    async fn get_json<T: DeserializeOwned + Send + 'static>(
        &self,
        url: Url,
    ) -> Result<T, GitHubError> {
        let mut request = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(authorization) = &self.authorization {
            request = request.header(header::AUTHORIZATION, authorization.clone());
        }
        let slot = self.slot().await?;
        let response = check_status(self.send(request).await?)?;
        let body = read_limited(response, MAX_API_RESPONSE_BYTES).await?;
        drop(slot);
        let small = body.len() <= INLINE_PARSE_LIMIT;
        let parse = move || {
            serde_json::from_slice(&body)
                .map_err(|e| GitHubError::Upstream(format!("unexpected JSON from GitHub: {e}")))
        };
        if small {
            parse()
        } else {
            tokio::task::spawn_blocking(parse)
                .await
                .map_err(|e| GitHubError::Upstream(format!("parsing failed: {e}")))?
        }
    }

    async fn slot(&self) -> Result<SemaphorePermit<'_>, GitHubError> {
        match tokio::time::timeout(REQUEST_SLOT_WAIT, self.slots.acquire()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) | Err(_) => Err(GitHubError::Busy),
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Response, GitHubError> {
        request
            .send()
            .await
            .map_err(|e| GitHubError::Upstream(format!("request failed: {e}")))
    }
}

fn is_allowed_url(url: &Url, allowed_origins: &[String]) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && allowed_origins.contains(&url.origin().ascii_serialization())
}

fn not_found_as_none<T>(result: Result<T, GitHubError>) -> Result<Option<T>, GitHubError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(GitHubError::NotFound) => Ok(None),
        Err(err) => Err(err),
    }
}

fn check_status(response: Response) -> Result<Response, GitHubError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }

    let headers = response.headers();
    let exhausted = headers
        .get("x-ratelimit-remaining")
        .is_some_and(|v| v.as_bytes() == b"0");
    if status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (exhausted || headers.contains_key(header::RETRY_AFTER)))
    {
        let retry_after = header_u64(headers.get(header::RETRY_AFTER)).or_else(|| {
            header_u64(headers.get("x-ratelimit-reset")).map(|reset| {
                reset.saturating_sub(u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0))
            })
        });
        return Err(GitHubError::RateLimited {
            retry_after: retry_after.map(|s| s.clamp(1, 3600)),
        });
    }

    match status {
        // Private repositories the token can't see are reported as 404.
        StatusCode::NOT_FOUND | StatusCode::GONE | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS => {
            Err(GitHubError::NotFound)
        }
        StatusCode::UNPROCESSABLE_ENTITY => Err(GitHubError::Unprocessable),
        StatusCode::UNAUTHORIZED => Err(GitHubError::Upstream(
            "GitHub rejected the configured GITHUB_TOKEN".into(),
        )),
        // Includes 403 without rate limit headers (e.g. secondary rate
        // limits): never treat it as "not found", so it isn't cached.
        other => Err(GitHubError::Upstream(format!(
            "GitHub returned HTTP {other}"
        ))),
    }
}

fn header_u64(value: Option<&HeaderValue>) -> Option<u64> {
    value?.to_str().ok()?.trim().parse().ok()
}

async fn read_limited(mut response: Response, max_bytes: u64) -> Result<Vec<u8>, GitHubError> {
    if response.content_length().is_some_and(|len| len > max_bytes) {
        return Err(GitHubError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| GitHubError::Upstream(format!("reading response failed: {e}")))?
    {
        if (body.len() + chunk.len()) as u64 > max_bytes {
            return Err(GitHubError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// The SHA-1 git uses for blob objects: `sha1("blob <len>\0" + content)`.
pub fn git_blob_sha1(content: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", content.len()).as_bytes());
    hasher.update(content);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> GitHubClient {
        GitHubClient::new(Duration::from_secs(5), None).unwrap()
    }

    #[test]
    fn api_urls_cannot_be_manipulated() {
        let client = client();
        let url = client.api_url(&["repos", "x", "../../user?", "releases", "latest"]);
        assert_eq!(url.host_str(), Some("api.github.com"));
        assert_eq!(url.query(), None);
        assert_eq!(url.path(), "/repos/x/..%2F..%2Fuser%3F/releases/latest");

        let url = client.api_url(&["repos", "o", "r", "compare", "v1.0...v2.0"]);
        assert_eq!(
            url.as_str(),
            "https://api.github.com/repos/o/r/compare/v1.0...v2.0"
        );

        let url = client.api_url(&["repos", "o", "r", "git", "trees", "a#b"]);
        assert_eq!(url.fragment(), None);
    }

    #[test]
    fn only_github_https_urls_are_allowed() {
        let allowed = Endpoints::github().allowed_origins;
        let ok = |s: &str| is_allowed_url(&Url::parse(s).unwrap(), &allowed);
        assert!(ok("https://github.com/o/r/raw/abc/file"));
        assert!(ok("https://raw.githubusercontent.com/o/r/abc/file"));
        assert!(ok("https://github.com:443/o/r"));
        assert!(!ok("http://raw.githubusercontent.com/o/r/abc/file"));
        assert!(!ok("https://evil.example/o/r"));
        assert!(!ok("https://github.com.evil.example/o/r"));
        assert!(!ok("https://user@github.com/o/r"));
        assert!(!ok("https://github.com:8443/o/r"));
    }

    fn response(status: u16, headers: &[(&str, &str)]) -> Response {
        let mut builder = axum::http::Response::builder().status(status);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        Response::from(builder.body(Vec::<u8>::new()).unwrap())
    }

    #[test]
    fn status_mapping() {
        assert!(matches!(
            check_status(response(404, &[])),
            Err(GitHubError::NotFound)
        ));
        assert!(matches!(
            check_status(response(422, &[])),
            Err(GitHubError::Unprocessable)
        ));
        assert!(matches!(
            check_status(response(401, &[])),
            Err(GitHubError::Upstream(_))
        ));
        // A bare 403 must not look like "not found".
        assert!(matches!(
            check_status(response(403, &[])),
            Err(GitHubError::Upstream(_))
        ));
        assert!(matches!(
            check_status(response(403, &[("x-ratelimit-remaining", "0")])),
            Err(GitHubError::RateLimited { .. })
        ));
        assert!(matches!(
            check_status(response(429, &[("retry-after", "120")])),
            Err(GitHubError::RateLimited {
                retry_after: Some(120)
            })
        ));
        assert!(matches!(
            check_status(response(403, &[("retry-after", "999999")])),
            Err(GitHubError::RateLimited {
                retry_after: Some(3600)
            })
        ));
        assert!(check_status(response(200, &[])).is_ok());
    }

    #[test]
    fn git_blob_hash_matches_git() {
        // `printf 'hello\n' | git hash-object --stdin`
        assert_eq!(
            git_blob_sha1(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        // Empty blob.
        assert_eq!(
            git_blob_sha1(b""),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }
}
