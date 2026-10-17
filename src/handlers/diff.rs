//! Diff endpoints.
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::error::ApiError;
use crate::models::diff::DiffResponse;
use crate::net::ClientAddr;
use crate::services::diff::get_diff;
use crate::state::AppState;
use crate::validation::is_valid_version;

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    zip: Option<String>,
    format: Option<String>,
}

/// `GET /diff/{owner}/{repo}/{from_version}/{to_version}[?zip=true]`
pub async fn get_version_diff(
    State(state): State<Arc<AppState>>,
    client: ClientAddr,
    path: Result<Path<(String, String, String, String)>, PathRejection>,
    query: Result<Query<DiffQuery>, QueryRejection>,
) -> Result<Json<DiffResponse>, ApiError> {
    let Path((owner, repo, from, to)) = path?;
    let Query(query) = query?;
    respond(&state, client, &owner, &repo, &from, &to, &query).await
}

/// `GET /diff/{repo}/{from_version}/{to_version}` with the default owner.
pub async fn get_version_diff_default_owner(
    State(state): State<Arc<AppState>>,
    client: ClientAddr,
    path: Result<Path<(String, String, String)>, PathRejection>,
    query: Result<Query<DiffQuery>, QueryRejection>,
) -> Result<Json<DiffResponse>, ApiError> {
    let Path((repo, from, to)) = path?;
    let Query(query) = query?;
    let owner = state.config.default_repo.owner.clone();
    respond(&state, client, &owner, &repo, &from, &to, &query).await
}

async fn respond(
    state: &Arc<AppState>,
    client: ClientAddr,
    owner: &str,
    repo: &str,
    from: &str,
    to: &str,
    query: &DiffQuery,
) -> Result<Json<DiffResponse>, ApiError> {
    state.stats.record_request();

    let want_zip = match query.zip.as_deref() {
        None | Some("false" | "0") => false,
        Some("true" | "1") => true,
        Some(_) => {
            return Err(ApiError::bad_request(
                "invalid_zip_flag",
                "zip must be true or false",
            ));
        }
    };
    if query.format.as_deref().is_some_and(|f| f != "json") {
        return Err(ApiError::bad_request(
            "invalid_format",
            "Only format=json is supported",
        ));
    }

    let target = state.config.repo_policy.resolve(owner, repo)?;
    if !is_valid_version(from) || !is_valid_version(to) {
        return Err(ApiError::bad_request(
            "invalid_version",
            "Versions must be 1-100 characters, start with a letter or digit and only contain letters, digits, '-', '_', '.' and '+'",
        ));
    }
    if from == to {
        return Err(ApiError::bad_request(
            "identical_versions",
            "from_version and to_version must be different",
        ));
    }

    get_diff(state, &target, client.0, from, to, want_zip)
        .await
        .map(Json)
}
