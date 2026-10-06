use crate::{diff, github, mirror, AppState};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/repos", get(list_repos))
        .route("/api/sync", post(sync_repo))
        .route("/api/{owner}/{repo}/refs", get(refs))
        .route("/api/{owner}/{repo}/commits", get(commits))
        .route("/api/{owner}/{repo}/diff", get(diff_summary))
        .route("/api/{owner}/{repo}/file", get(file_diff))
        .route("/api/{owner}/{repo}/pulls", get(pulls))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../assets/index.html"))
}

async fn list_repos() -> Json<serde_json::Value> {
    let mut repos = Vec::new();
    let mirrors = mirror::cache_dir().join("mirrors");
    if let Ok(rd) = std::fs::read_dir(&mirrors) {
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if let Some(slug) = file.strip_suffix(".git") {
                let parts: Vec<&str> = slug.splitn(2, '_').collect();
                if parts.len() == 2 {
                    repos.push(format!("{}/{}", parts[0], parts[1]));
                }
            }
        }
    }
    Json(json!({ "repos": repos }))
}

#[derive(Deserialize)]
struct SyncReq {
    repo: String,
}

async fn sync_repo(Json(req): Json<SyncReq>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let repo = mirror::RepoRef::parse(&req.repo).map_err(internal)?;
    tokio::task::spawn_blocking(move || mirror::sync(&repo))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })))
}

async fn refs(Path((owner, repo)): Path<(String, String)>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let r = mirror::RepoRef {
        owner,
        name: repo,
    };
    let path = mirror::mirror_path(&r);
    let refs = tokio::task::spawn_blocking(move || diff::refs(&path))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(Json(json!({ "refs": refs })))
}

#[derive(Deserialize)]
struct CommitsQuery {
    limit: Option<usize>,
    r#ref: Option<String>,
}

async fn commits(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<CommitsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let limit = q.limit.unwrap_or(50).min(500);
    let refspec = q.r#ref.clone();
    let list = tokio::task::spawn_blocking(move || diff::commits(&path, limit, refspec.as_deref()))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(Json(json!({ "commits": list })))
}

#[derive(Deserialize)]
struct RangeQuery {
    base: String,
    head: String,
}

async fn diff_summary(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<RangeQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let (base, head) = (q.base.clone(), q.head.clone());
    let summary = tokio::task::spawn_blocking(move || diff::compare(&path, &base, &head))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(Json(serde_json::to_value(summary).map_err(internal)?))
}

#[derive(Deserialize)]
struct FileQuery {
    base: String,
    head: String,
    path: String,
}

async fn file_diff(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let (base, head, p) = (q.base.clone(), q.head.clone(), q.path.clone());
    let fd = tokio::task::spawn_blocking(move || diff::file_diff(&path, &base, &head, &p))
        .await
        .map_err(internal)?
        .map_err(internal)?;
    Ok(Json(serde_json::to_value(fd).map_err(internal)?))
}

async fn pulls(
    Path((owner, repo)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let list = state.gh.pulls(&owner, &repo).await.map_err(internal)?;
    Ok(Json(json!({ "pulls": list })))
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn serve(state: AppState, port: Option<u16>) -> anyhow::Result<std::net::SocketAddr> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });
    Ok(local)
}
