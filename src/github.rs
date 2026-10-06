use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub user: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_sha: String,
    pub base_sha: String,
    pub html_url: String,
    pub updated_at: String,
}

#[derive(Clone)]
pub struct GitHubClient {
    http: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub fn new() -> Self {
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .ok();
        Self {
            http: reqwest::Client::new(),
            token,
        }
    }

    async fn get(&self, url: &str) -> Result<serde_json::Value> {
        let mut req = self.http.get(url).header("Accept", "application/vnd.github+json");
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.context("github request failed")?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("github API {status} for {url}");
        }
        Ok(resp.json().await.context("github decode failed")?)
    }

    pub async fn pulls(&self, owner: &str, repo: &str) -> Result<Vec<PullRequest>> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls?state=open&per_page=30&sort=updated"
        );
        let items = self.get(&url).await?;
        let arr = items.as_array().cloned().unwrap_or_default();
        let mut out = Vec::with_capacity(arr.len());
        for p in arr {
            let number = p["number"].as_u64().unwrap_or(0);
            let title = p["title"].as_str().unwrap_or("").to_string();
            let state = p["state"].as_str().unwrap_or("").to_string();
            let user = p["user"]["login"].as_str().unwrap_or("").to_string();
            let head_ref = p["head"]["ref"].as_str().unwrap_or("").to_string();
            let base_ref = p["base"]["ref"].as_str().unwrap_or("").to_string();
            let head_sha = p["head"]["sha"].as_str().unwrap_or("").to_string();
            let base_sha = p["base"]["sha"].as_str().unwrap_or("").to_string();
            let html_url = p["html_url"].as_str().unwrap_or("").to_string();
            let updated_at = p["updated_at"].as_str().unwrap_or("").to_string();
            out.push(PullRequest {
                number,
                title,
                state,
                user,
                head_ref,
                base_ref,
                head_sha,
                base_sha,
                html_url,
                updated_at,
            });
        }
        Ok(out)
    }
}
