//! Version endpoints.
use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::error::ApiError;
use crate::models::responses::VersionResponse;
use crate::services::version::latest_version;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct VersionQuery {
    owner: Option<String>,
    repo: Option<String>,
}

/// `GET /version?owner=x&repo=y` (both optional).
pub async fn get_version(
    State(state): State<Arc<AppState>>,
    query: Result<Query<VersionQuery>, QueryRejection>,
) -> Result<Json<VersionResponse>, ApiError> {
    let Query(query) = query?;
    let default = &state.config.default_repo;
    let owner = query.owner.as_deref().unwrap_or(&default.owner);
    let repo = query.repo.as_deref().unwrap_or(&default.name);
    respond(&state, owner, repo).await
}

/// `GET /version/{repo}` with the default owner.
pub async fn get_version_by_repo(
    State(state): State<Arc<AppState>>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<VersionResponse>, ApiError> {
    let Path(repo) = path?;
    respond(&state, &state.config.default_repo.owner, &repo).await
}

/// `GET /version/{owner}/{repo}`.
pub async fn get_version_by_owner_repo(
    State(state): State<Arc<AppState>>,
    path: Result<Path<(String, String)>, PathRejection>,
) -> Result<Json<VersionResponse>, ApiError> {
    let Path((owner, repo)) = path?;
    respond(&state, &owner, &repo).await
}

async fn respond(
    state: &AppState,
    owner: &str,
    repo: &str,
) -> Result<Json<VersionResponse>, ApiError> {
    state.stats.record_request();
    let target = state.config.repo_policy.resolve(owner, repo)?;
    latest_version(state, &target).await.map(Json)
}
