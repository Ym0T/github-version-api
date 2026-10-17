//! End-to-end tests against a fake GitHub API.
use std::io::Read;
use std::time::Duration;

use axum::http::StatusCode;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::AppConfig;
use crate::services::github::GitHubClient;
use crate::services::signing::{Signer, generate_keypair, verify};
use crate::state::AppState;
use crate::test_support::{COMMIT_A, COMMIT_B, FakeFile, FakeGitHub, Scenario, TestApp};

struct Setup {
    app: TestApp,
    fake: FakeGitHub,
    public_key: String,
}

async fn setup(scenario: Scenario, extra: &[(&str, &str)]) -> Setup {
    let fake = FakeGitHub::start(scenario).await;
    let (private_key, public_key) = generate_keypair().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let app = app_with(&fake, &dir, Some(private_key.as_str()), extra).await;
    Setup {
        app: TestApp::new(app, dir),
        fake,
        public_key,
    }
}

async fn app_with(
    fake: &FakeGitHub,
    dir: &tempfile::TempDir,
    private_key: Option<&str>,
    extra: &[(&str, &str)],
) -> std::sync::Arc<AppState> {
    let data_dir = dir.path().to_string_lossy().into_owned();
    let mut vars = vec![
        ("DATA_DIR", data_dir.as_str()),
        ("DEFAULT_OWNER", "o"),
        ("DEFAULT_REPO", "r"),
        ("ALLOWED_REPOS", "o/r#42"),
        ("API_BASE_URL", "http://localhost:3000"),
        ("RATE_LIMIT_BURST", "10000"),
    ];
    if let Some(key) = private_key {
        vars.push(("SIGNING_PRIVATE_KEY", key));
    }
    // Later entries win.
    for (name, value) in extra {
        vars.retain(|(existing, _)| existing != name);
        vars.push((name, value));
    }
    let config = AppConfig::for_tests(&vars).unwrap();
    let signer = config
        .signing_key
        .as_ref()
        .map(|key| Signer::from_base64(key.expose()).unwrap());
    let github =
        GitHubClient::with_endpoints(Duration::from_secs(5), None, fake.endpoints()).unwrap();
    AppState::with_github(config, signer, github).await.unwrap()
}

fn update_files() -> Vec<FakeFile> {
    let mut script = FakeFile::new("modules/a.sh", "added", "#!/bin/sh\necho hi\n");
    script.mode = "100755";
    let mut renamed = FakeFile::new("new.txt", "renamed", "same\n");
    renamed.previous = Some("prev.txt".into());
    vec![
        script,
        FakeFile::new("README.md", "modified", "readme v2\n"),
        FakeFile::new("old.txt", "removed", ""),
        renamed,
    ]
}

fn zip_scenario() -> Scenario {
    Scenario {
        files: update_files(),
        ..Scenario::default()
    }
}

fn manifest_field<'a>(manifest: &'a str, key: &str) -> Vec<&'a str> {
    manifest
        .lines()
        .filter_map(|line| line.strip_prefix(&format!("{key}=")))
        .collect()
}

#[tokio::test]
async fn version_lookup_is_cached_and_validated() {
    let s = setup(Scenario::default(), &[]).await;
    let (status, first) = s.app.get_json("/version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["version"], "v2");
    assert_eq!(first["cached"], false);
    let (_, second) = s.app.get_json("/version/o/r").await;
    assert_eq!(second["cached"], true);
    assert_eq!(s.fake.count(|c| &c.repo), 1);

    // A release tag that is unsafe for shell scripts is skipped.
    let s = setup(
        Scenario {
            latest_release: Some("v3;rm -rf /".into()),
            ..Scenario::default()
        },
        &[],
    )
    .await;
    let (_, body) = s.app.get_json("/version").await;
    assert_eq!(body["version"], "v2", "falls back to the newest valid tag");
}

#[tokio::test]
async fn github_errors_are_not_cached_as_missing() {
    let s = setup(
        Scenario {
            repo_status: 403,
            ..Scenario::default()
        },
        &[],
    )
    .await;
    let (status, body) = s.app.get_json("/version").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "github_unavailable");

    s.fake.update(|sc| sc.repo_status = 200);
    let (status, _) = s.app.get_json("/version").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn private_renamed_or_unpinned_repositories_are_refused() {
    for (scenario, extra) in [
        (
            Scenario {
                private: true,
                ..Scenario::default()
            },
            None,
        ),
        (
            Scenario {
                full_name: Some("someone-else/r".into()),
                ..Scenario::default()
            },
            None,
        ),
        (Scenario::default(), Some("o/r#7")),
    ] {
        let extra: Vec<(&str, &str)> = extra.map(|v| ("ALLOWED_REPOS", v)).into_iter().collect();
        let s = setup(scenario, &extra).await;
        let (status, body) = s.app.get_json("/version").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "repository_not_found");
    }

    let s = setup(Scenario::default(), &[("ALLOWED_REPOS", "o/r#42")]).await;
    assert_eq!(s.app.get_json("/version").await.0, StatusCode::OK);
}

#[tokio::test]
async fn diffs_only_between_known_tags_without_extra_github_calls() {
    let s = setup(zip_scenario(), &[]).await;
    for uri in [
        "/diff/o/r/v9/v2",
        "/diff/o/r/v1/main",
        "/diff/o/r/0123abcd/v2",
    ] {
        let (status, body) = s.app.get_json(uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(body["error"], "version_not_found");
    }
    assert_eq!(s.fake.count(|c| &c.compare), 0);

    let (status, body) = s.app.get_json("/diff/o/r/v1/v2").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["from_commit"], COMMIT_A);
    assert_eq!(body["to_commit"], COMMIT_B);
    assert_eq!(body["comparison_status"], "ahead");
    assert_eq!(body["files_changed"].as_array().unwrap().len(), 4);
    assert_eq!(body["download_url"], Value::Null);
    assert_eq!(body["manifest"], Value::Null);
}

#[tokio::test]
async fn signed_archive_end_to_end() {
    let s = setup(zip_scenario(), &[]).await;
    let (status, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Manifest signature with the pinned key.
    let manifest = body["manifest"].as_str().unwrap();
    let signature = body["manifest_signature"].as_str().unwrap();
    assert!(verify(manifest.as_bytes(), signature, &s.public_key));
    assert_eq!(manifest_field(manifest, "repository"), ["o/r"]);
    assert_eq!(manifest_field(manifest, "from"), ["v1"]);
    assert_eq!(manifest_field(manifest, "to"), ["v2"]);
    assert_eq!(manifest_field(manifest, "from_commit"), [COMMIT_A]);
    assert_eq!(manifest_field(manifest, "to_commit"), [COMMIT_B]);
    assert_eq!(manifest_field(manifest, "delete"), ["old.txt", "prev.txt"]);
    // Legacy signatures are opt-in.
    assert_eq!(body["signature"], Value::Null);

    // The archive matches the manifest.
    let download = body["download_url"].as_str().unwrap();
    let path = download.strip_prefix("http://localhost:3000").unwrap();
    let (status, archive) = s.app.get_bytes(path).await;
    assert_eq!(status, StatusCode::OK);
    let digest = hex::encode(Sha256::digest(&archive));
    assert_eq!(manifest_field(manifest, "sha256"), [digest.as_str()]);
    assert_eq!(body["checksum"], digest);

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).unwrap();
    let mut names: Vec<String> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["README.md", "modules/a.sh", "new.txt"],
        "only repository files"
    );
    let mut script = String::new();
    zip.by_name("modules/a.sh")
        .unwrap()
        .read_to_string(&mut script)
        .unwrap();
    assert_eq!(script, "#!/bin/sh\necho hi\n");
    assert_eq!(
        zip.by_name("modules/a.sh").unwrap().unix_mode().unwrap() & 0o777,
        0o755
    );
    assert_eq!(
        zip.by_name("README.md").unwrap().unix_mode().unwrap() & 0o777,
        0o644
    );
}

#[tokio::test]
async fn archives_are_built_once_and_reused() {
    let s = std::sync::Arc::new(setup(zip_scenario(), &[]).await);
    let requests = (0..6).map(|_| {
        let s = std::sync::Arc::clone(&s);
        tokio::spawn(async move { s.app.get_json("/diff/o/r/v1/v2?zip=true").await })
    });
    let mut urls = Vec::new();
    for request in requests {
        let (status, body) = request.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        urls.push(body["download_url"].as_str().unwrap().to_string());
    }
    urls.dedup();
    assert_eq!(urls.len(), 1, "all requests got the same archive");
    assert_eq!(s.fake.count(|c| &c.raw), 3, "each file was downloaded once");
    assert_eq!(s.fake.count(|c| &c.compare), 1);

    let (_, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(body["download_url"], urls[0].as_str());
    assert_eq!(s.fake.count(|c| &c.raw), 3);
}

#[tokio::test]
async fn archive_preconditions_are_checked_before_spending_budget() {
    let cases: Vec<(Scenario, &str, &str)> = vec![
        (
            Scenario {
                compare_status: "diverged".into(),
                ..zip_scenario()
            },
            "/diff/o/r/v1/v2?zip=true",
            "versions_not_sequential",
        ),
        (
            Scenario {
                compare_status: "behind".into(),
                ..zip_scenario()
            },
            "/diff/o/r/v2/v1?zip=true",
            "versions_not_sequential",
        ),
        (
            Scenario {
                latest_release: Some("v3".into()),
                tags: vec![
                    ("v3".into(), "c".repeat(40)),
                    ("v2".into(), COMMIT_B.into()),
                    ("v1".into(), COMMIT_A.into()),
                ],
                ..zip_scenario()
            },
            "/diff/o/r/v1/v2?zip=true",
            "zip_target_not_latest",
        ),
    ];
    for (scenario, uri, code) in cases {
        // A budget of one build: failed preconditions must not use it up.
        let s = setup(scenario, &[("ZIP_MAX_PER_HOUR", "1")]).await;
        for _ in 0..3 {
            let (status, body) = s.app.get_json(uri).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
            assert_eq!(body["error"], code);
        }
        assert_eq!(s.fake.count(|c| &c.raw), 0);
    }
}

#[tokio::test]
async fn archives_fail_closed() {
    let mut symlink = FakeFile::new("link", "added", "target");
    symlink.mode = "120000";
    let mut scenario = zip_scenario();
    scenario.files.push(symlink);
    let s = setup(scenario, &[]).await;
    let (status, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "unsupported_file_type");

    let s = setup(
        Scenario {
            tamper_downloads: true,
            ..zip_scenario()
        },
        &[],
    )
    .await;
    let (status, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "content_verification_failed");
    let leftovers = std::fs::read_dir(&s.app.state.config.diffs_dir)
        .unwrap()
        .count();
    assert_eq!(leftovers, 0, "no partial archive is left behind");
}

#[tokio::test]
async fn legacy_signature_is_opt_in() {
    let s = setup(zip_scenario(), &[("LEGACY_SIGNATURES", "true")]).await;
    let (status, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::OK);
    let digest = hex::decode(body["checksum"].as_str().unwrap()).unwrap();
    assert!(verify(
        &digest,
        body["signature"].as_str().unwrap(),
        &s.public_key
    ));
}

#[tokio::test]
async fn restarts_reuse_archives_and_re_sign_when_needed() {
    let fake = FakeGitHub::start(zip_scenario()).await;
    let dir = tempfile::tempdir().unwrap();
    let (key, public_key) = generate_keypair().unwrap();
    let restart = |extra: &'static [(&'static str, &'static str)]| {
        let (fake, dir, key) = (&fake, &dir, key.clone());
        async move {
            TestApp::new(
                app_with(fake, dir, Some(key.as_str()), extra).await,
                tempfile::tempdir().unwrap(),
            )
        }
    };

    let first_app = restart(&[]).await;
    let (_, first) = first_app.get_json("/diff/o/r/v1/v2?zip=true").await;
    first_app.state.persist().await;

    // Same key, legacy signatures switched on: the archive is reused and
    // signed again, nothing is downloaded.
    let second_app = restart(&[("LEGACY_SIGNATURES", "true")]).await;
    let (status, second) = second_app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(
        second["download_url"], first["download_url"],
        "same archive"
    );
    assert_eq!(fake.count(|c| &c.raw), 3, "nothing was downloaded again");
    let digest = hex::decode(second["checksum"].as_str().unwrap()).unwrap();
    assert!(verify(
        &digest,
        second["signature"].as_str().unwrap(),
        &public_key
    ));
}

#[tokio::test]
async fn key_rotation_discards_the_persisted_cache() {
    let fake = FakeGitHub::start(zip_scenario()).await;
    let dir = tempfile::tempdir().unwrap();

    let (old_key, _) = generate_keypair().unwrap();
    let old = TestApp::new(
        app_with(&fake, &dir, Some(old_key.as_str()), &[]).await,
        tempfile::tempdir().unwrap(),
    );
    let (_, first) = old.get_json("/diff/o/r/v1/v2?zip=true").await;
    old.state.persist().await;

    // The cache is authenticated with a key derived from the signing key,
    // so it can't be used after a rotation: the archive is built again.
    let (new_key, new_public) = generate_keypair().unwrap();
    let new = TestApp::new(
        app_with(&fake, &dir, Some(new_key.as_str()), &[]).await,
        tempfile::tempdir().unwrap(),
    );
    let (status, second) = new.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_ne!(second["download_url"], first["download_url"]);
    let manifest = second["manifest"].as_str().unwrap();
    assert!(verify(
        manifest.as_bytes(),
        second["manifest_signature"].as_str().unwrap(),
        &new_public
    ));
}

#[tokio::test]
async fn unlisted_repositories_have_a_lookup_budget() {
    let s = setup(
        Scenario::default(),
        &[
            ("ALLOWED_REPOS", "o/r#42,*"),
            ("LOOKUP_BUDGET_PER_HOUR", "2"),
        ],
    )
    .await;
    // Repository and version lookup: two units.
    assert_eq!(s.app.get_json("/version/x/y").await.0, StatusCode::OK);
    let (status, body) = s.app.get_json("/version/x/z").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "lookup_limit_reached");
    // Allowed repositories are not affected.
    assert_eq!(s.app.get_json("/version/o/r").await.0, StatusCode::OK);
    // No archives for repositories that are not explicitly allowed.
    let (status, body) = s.app.get_json("/diff/x/y/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], "zip_not_allowed");
}

const COMMIT_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

fn with_v0(scenario: Scenario) -> Scenario {
    let mut scenario = scenario;
    scenario.tags.push(("v0".into(), COMMIT_C.into()));
    scenario
}

async fn get_json_as(app: &TestApp, uri: &str, peer: &str) -> (StatusCode, Value) {
    let (status, _, body) = app.send(uri, peer, None).await;
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn archives_require_a_pinned_repository() {
    let s = setup(zip_scenario(), &[("ALLOWED_REPOS", "o/r")]).await;
    let (status, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "repository_not_pinned");
    assert_eq!(s.fake.count(|c| &c.compare), 0);
}

#[tokio::test]
async fn failed_builds_are_cached_and_do_not_use_up_the_budget() {
    let mut symlink = FakeFile::new("link", "added", "target");
    symlink.mode = "120000";
    let mut scenario = with_v0(zip_scenario());
    scenario.files.push(symlink);
    let s = setup(scenario, &[("ZIP_MAX_PER_HOUR", "1")]).await;
    for _ in 0..3 {
        let (status, body) = s.app.get_json("/diff/o/r/v1/v2?zip=true").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "unsupported_file_type");
    }
    assert_eq!(s.fake.count(|c| &c.tree), 1, "the failure is cached");
    assert_eq!(s.fake.count(|c| &c.raw), 0);

    // The single build of this hour is still available.
    s.fake.update(|sc| sc.files.retain(|f| f.mode != "120000"));
    let (status, body) = s.app.get_json("/diff/o/r/v0/v2?zip=true").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn builds_are_limited_per_client() {
    let s = setup(
        with_v0(zip_scenario()),
        &[("ZIP_MAX_PER_CLIENT_PER_HOUR", "1")],
    )
    .await;
    let peer = "198.51.100.7:1000";
    assert_eq!(
        get_json_as(&s.app, "/diff/o/r/v1/v2?zip=true", peer)
            .await
            .0,
        StatusCode::OK
    );
    // Reusing an existing archive is free.
    assert_eq!(
        get_json_as(&s.app, "/diff/o/r/v1/v2?zip=true", peer)
            .await
            .0,
        StatusCode::OK
    );
    let (status, body) = get_json_as(&s.app, "/diff/o/r/v0/v2?zip=true", peer).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "zip_client_limit_reached");
    // Other clients are not affected.
    assert_eq!(
        s.app.get_json("/diff/o/r/v0/v2?zip=true").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn deterministic_diff_failures_are_cached() {
    let files = (0..300)
        .map(|i| FakeFile::new(&format!("f{i}.txt"), "added", "x"))
        .collect();
    let s = setup(
        Scenario {
            files,
            ..Scenario::default()
        },
        &[],
    )
    .await;
    for _ in 0..3 {
        let (status, body) = s.app.get_json("/diff/o/r/v1/v2").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "diff_too_large");
    }
    assert_eq!(s.fake.count(|c| &c.compare), 1);
}

#[tokio::test]
async fn diffs_to_older_versions_use_the_lookup_budget() {
    let s = setup(with_v0(zip_scenario()), &[("LOOKUP_BUDGET_PER_HOUR", "1")]).await;
    assert_eq!(s.app.get_json("/diff/o/r/v0/v1").await.0, StatusCode::OK);
    let (status, body) = s.app.get_json("/diff/o/r/v1/v0").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "lookup_limit_reached");
    // Updates to the latest version are not limited by it.
    assert_eq!(s.app.get_json("/diff/o/r/v0/v2").await.0, StatusCode::OK);
}

#[tokio::test]
async fn new_tags_are_found_before_the_tag_cache_expires() {
    let s = setup(zip_scenario(), &[]).await;
    assert_eq!(s.app.get_json("/diff/o/r/v1/v2").await.0, StatusCode::OK);
    s.fake.update(|sc| {
        sc.tags.insert(0, ("v3".into(), "d".repeat(40)));
        sc.latest_release = Some("v3".into());
    });
    let (status, body) = s.app.get_json("/diff/o/r/v2/v3").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["to_commit"], "d".repeat(40));
}

#[tokio::test]
async fn inconsistent_merge_base_is_rejected() {
    let s = setup(
        Scenario {
            merge_base: Some("e".repeat(40)),
            ..zip_scenario()
        },
        &[],
    )
    .await;
    let (status, body) = s.app.get_json("/diff/o/r/v1/v2").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"], "github_inconsistent");
}

#[tokio::test]
async fn tags_beyond_the_first_page_are_found() {
    let tags: Vec<(String, String)> = (0..150)
        .rev()
        .map(|i| (format!("t{i}"), format!("{i:040x}")))
        .collect();
    let s = setup(
        Scenario {
            tags,
            latest_release: Some("t149".into()),
            ..zip_scenario()
        },
        &[],
    )
    .await;
    let (status, body) = s.app.get_json("/diff/o/r/t5/t149").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["from_commit"], format!("{:040x}", 5));
}

#[tokio::test]
async fn version_file_fallback() {
    let s = setup(
        Scenario {
            latest_release: None,
            tags: Vec::new(),
            version_file: Some("1.4.2\nignored".into()),
            ..Scenario::default()
        },
        &[],
    )
    .await;
    let (status, body) = s.app.get_json("/version").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["version"], "1.4.2");
    assert_eq!(body["release_notes"], "From VERSION file");
}
