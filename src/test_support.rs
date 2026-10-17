//! Test helpers: a wrapper around the router and a fake GitHub API.
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::app::build_router;
use crate::services::github::{Endpoints, GitHubClient, git_blob_sha1};
use crate::state::AppState;

pub struct TestApp {
    pub router: Router,
    pub state: Arc<AppState>,
    pub dir: tempfile::TempDir,
}

impl TestApp {
    pub fn new(state: Arc<AppState>, dir: tempfile::TempDir) -> Self {
        Self {
            router: build_router(&state).unwrap(),
            state,
            dir,
        }
    }

    pub async fn request(
        &self,
        request: Request<Body>,
        peer: &str,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request = request;
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    pub async fn send(
        &self,
        uri: &str,
        peer: &str,
        forwarded_for: Option<&str>,
    ) -> (StatusCode, HeaderMap, String) {
        let mut request = Request::builder().uri(uri);
        if let Some(value) = forwarded_for {
            request = request.header("x-forwarded-for", value);
        }
        let (status, headers, body) = self
            .request(request.body(Body::empty()).unwrap(), peer)
            .await;
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    pub async fn send_with_origin(
        &self,
        uri: &str,
        origin: &str,
    ) -> (StatusCode, HeaderMap, String) {
        let request = Request::builder()
            .uri(uri)
            .header("origin", origin)
            .body(Body::empty())
            .unwrap();
        let (status, headers, body) = self.request(request, &next_peer()).await;
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    /// Sends from a new client address each time, so tests don't hit the rate limit.
    pub async fn get(&self, uri: &str) -> (StatusCode, String) {
        let (status, _, body) = self.send(uri, &next_peer(), None).await;
        (status, body)
    }

    pub async fn get_json(&self, uri: &str) -> (StatusCode, Value) {
        let (status, body) = self.get(uri).await;
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    pub async fn get_bytes(&self, uri: &str) -> (StatusCode, Vec<u8>) {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let (status, _, body) = self.request(request, &next_peer()).await;
        (status, body)
    }
}

fn next_peer() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{}:1234", std::net::Ipv4Addr::from(0xC000_0000 + n))
}

/// A GitHub client that can't reach anything (connections to a closed local
/// port fail), so tests never touch the network by accident.
pub fn offline_github() -> GitHubClient {
    let endpoints = Endpoints {
        api: "http://127.0.0.1:9/".parse().unwrap(),
        raw: "http://127.0.0.1:9/raw/".parse().unwrap(),
        allowed_origins: vec!["http://127.0.0.1:9".into()],
        https_only: false,
    };
    GitHubClient::with_endpoints(std::time::Duration::from_secs(2), None, endpoints).unwrap()
}

pub fn error_code(body: &str) -> String {
    serde_json::from_str::<Value>(body).unwrap()["error"]
        .as_str()
        .unwrap()
        .to_string()
}

pub const COMMIT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const COMMIT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Debug, Clone)]
pub struct FakeFile {
    pub path: String,
    pub status: &'static str,
    pub content: Vec<u8>,
    pub mode: &'static str,
    pub previous: Option<String>,
}

impl FakeFile {
    pub fn new(path: &str, status: &'static str, content: &str) -> Self {
        Self {
            path: path.into(),
            status,
            content: content.as_bytes().to_vec(),
            mode: "100644",
            previous: None,
        }
    }

    fn blob_sha(&self) -> String {
        git_blob_sha1(&self.content)
    }
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub repo_status: u16,
    pub private: bool,
    pub repo_id: u64,
    /// Name GitHub reports; defaults to the requested one.
    pub full_name: Option<String>,
    pub latest_release: Option<String>,
    pub tags: Vec<(String, String)>,
    pub compare_status: String,
    /// Merge base reported by compare; defaults to the base commit.
    pub merge_base: Option<String>,
    pub files: Vec<FakeFile>,
    pub tamper_downloads: bool,
    /// Content of `VERSION` on the default branch.
    pub version_file: Option<String>,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            repo_status: 200,
            private: false,
            repo_id: 42,
            full_name: None,
            latest_release: Some("v2".into()),
            tags: vec![
                ("v2".into(), COMMIT_B.into()),
                ("v1".into(), COMMIT_A.into()),
            ],
            compare_status: "ahead".into(),
            merge_base: None,
            files: Vec::new(),
            tamper_downloads: false,
            version_file: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Calls {
    pub repo: AtomicUsize,
    pub tags: AtomicUsize,
    pub compare: AtomicUsize,
    pub tree: AtomicUsize,
    pub raw: AtomicUsize,
}

#[derive(Clone)]
struct Fake {
    base: String,
    scenario: Arc<Mutex<Scenario>>,
    calls: Arc<Calls>,
}

pub struct FakeGitHub {
    base: String,
    pub scenario: Arc<Mutex<Scenario>>,
    pub calls: Arc<Calls>,
}

impl FakeGitHub {
    pub async fn start(scenario: Scenario) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let fake = Fake {
            base: base.clone(),
            scenario: Arc::new(Mutex::new(scenario)),
            calls: Arc::new(Calls::default()),
        };
        let app = Router::new()
            .route("/repos/{owner}/{repo}", get(repo))
            .route("/repos/{owner}/{repo}/releases/latest", get(release))
            .route("/repos/{owner}/{repo}/tags", get(tags))
            .route("/repos/{owner}/{repo}/compare/{range}", get(compare))
            .route("/repos/{owner}/{repo}/git/trees/{sha}", get(tree))
            .route("/raw/{sha}", get(raw))
            .route("/rawfiles/{owner}/{repo}/{branch}/{file}", get(raw_file))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base,
            scenario: fake.scenario,
            calls: fake.calls,
        }
    }

    pub fn endpoints(&self) -> Endpoints {
        Endpoints {
            api: format!("{}/", self.base).parse().unwrap(),
            raw: format!("{}/rawfiles/", self.base).parse().unwrap(),
            allowed_origins: vec![self.base.clone()],
            https_only: false,
        }
    }

    pub fn update(&self, change: impl FnOnce(&mut Scenario)) {
        change(&mut self.scenario.lock().unwrap());
    }

    pub fn count(&self, counter: impl Fn(&Calls) -> &AtomicUsize) -> usize {
        counter(&self.calls).load(Ordering::SeqCst)
    }
}

async fn repo(State(fake): State<Fake>, Path((owner, name)): Path<(String, String)>) -> Response {
    fake.calls.repo.fetch_add(1, Ordering::SeqCst);
    let s = fake.scenario.lock().unwrap().clone();
    if s.repo_status != 200 {
        return StatusCode::from_u16(s.repo_status).unwrap().into_response();
    }
    Json(json!({
        "id": s.repo_id,
        "full_name": s.full_name.unwrap_or(format!("{owner}/{name}")),
        "private": s.private,
        "visibility": if s.private { "private" } else { "public" },
        "default_branch": "main",
    }))
    .into_response()
}

async fn release(State(fake): State<Fake>) -> Response {
    let s = fake.scenario.lock().unwrap().clone();
    match s.latest_release {
        Some(tag) => Json(json!({
            "tag_name": tag,
            "name": format!("Release {tag}"),
            "published_at": "2026-01-01T00:00:00Z",
            "body": "notes",
            "assets": [],
        }))
        .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn tags(
    State(fake): State<Fake>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    fake.calls.tags.fetch_add(1, Ordering::SeqCst);
    let s = fake.scenario.lock().unwrap().clone();
    let number = |name: &str, default: usize| {
        query
            .get(name)
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(default)
    };
    let (page, per_page) = (number("page", 1), number("per_page", 30));
    let tags: Vec<Value> = s
        .tags
        .iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .map(|(name, sha)| json!({"name": name, "commit": {"sha": sha}}))
        .collect();
    Json(Value::Array(tags)).into_response()
}

async fn compare(
    State(fake): State<Fake>,
    Path((_, _, range)): Path<(String, String, String)>,
) -> Response {
    fake.calls.compare.fetch_add(1, Ordering::SeqCst);
    let s = fake.scenario.lock().unwrap().clone();
    let Some((base, _head)) = range.split_once("...") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let files: Vec<Value> = s
        .files
        .iter()
        .map(|file| {
            let removed = file.status == "removed";
            json!({
                "filename": file.path,
                "status": file.status,
                "additions": 1,
                "deletions": 0,
                "changes": 1,
                "sha": if removed { Value::Null } else { Value::from(file.blob_sha()) },
                "raw_url": if removed { Value::Null } else { Value::from(format!("{}/raw/{}", fake.base, file.blob_sha())) },
                "previous_filename": file.previous,
            })
        })
        .collect();
    Json(json!({
        "status": s.compare_status,
        "base_commit": {"sha": base},
        "merge_base_commit": {"sha": s.merge_base.unwrap_or_else(|| base.to_string())},
        "files": files,
    }))
    .into_response()
}

async fn tree(State(fake): State<Fake>) -> Response {
    fake.calls.tree.fetch_add(1, Ordering::SeqCst);
    let s = fake.scenario.lock().unwrap().clone();
    let entries: Vec<Value> = s
        .files
        .iter()
        .filter(|f| f.status != "removed")
        .map(|f| json!({"path": f.path, "mode": f.mode, "sha": f.blob_sha(), "type": "blob", "size": f.content.len()}))
        .collect();
    Json(json!({"truncated": false, "tree": entries})).into_response()
}

async fn raw(State(fake): State<Fake>, Path(sha): Path<String>) -> Response {
    fake.calls.raw.fetch_add(1, Ordering::SeqCst);
    let s = fake.scenario.lock().unwrap().clone();
    match s.files.iter().find(|f| f.blob_sha() == sha) {
        Some(file) => {
            let mut content = file.content.clone();
            if s.tamper_downloads {
                content.extend_from_slice(b"\nmalicious();\n");
            }
            content.into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn raw_file(
    State(fake): State<Fake>,
    Path((_, _, _, file)): Path<(String, String, String, String)>,
) -> Response {
    let s = fake.scenario.lock().unwrap().clone();
    match (file.as_str(), s.version_file) {
        ("VERSION", Some(content)) => content.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
