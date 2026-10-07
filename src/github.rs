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
    token: Option<String>,
}

impl GitHubClient {
    pub fn new() -> Self {
        let token = std::env::var("GITHUB_TOKEN")
            .or_else(|_| std::env::var("GH_TOKEN"))
            .ok();
        Self { token }
    }

    fn get(&self, url: &str) -> Result<serde_json::Value> {
        let mut req = ureq::get(url)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "xlr8");
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        let mut resp = req.call().context("github request failed")?;
        let status = resp.status();
        if !(200..300).contains(&status.as_u16()) {
            anyhow::bail!("github API {status} for {url}");
        }
        resp.body_mut()
            .read_json()
            .context("github decode failed")
    }

    pub fn pulls(&self, owner: &str, repo: &str) -> Result<Vec<PullRequest>> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/pulls?state=open&per_page=30&sort=updated"
        );
        let items = self.get(&url)?;
        let arr = items.as_array().cloned().unwrap_or_default();
        let mut out = Vec::with_capacity(arr.len());
        for p in arr {
            out.push(PullRequest {
                number: p["number"].as_u64().unwrap_or(0),
                title: p["title"].as_str().unwrap_or("").to_string(),
                state: p["state"].as_str().unwrap_or("").to_string(),
                user: p["user"]["login"].as_str().unwrap_or("").to_string(),
                head_ref: p["head"]["ref"].as_str().unwrap_or("").to_string(),
                base_ref: p["base"]["ref"].as_str().unwrap_or("").to_string(),
                head_sha: p["head"]["sha"].as_str().unwrap_or("").to_string(),
                base_sha: p["base"]["sha"].as_str().unwrap_or("").to_string(),
                html_url: p["html_url"].as_str().unwrap_or("").to_string(),
                updated_at: p["updated_at"].as_str().unwrap_or("").to_string(),
            });
        }
        Ok(out)
    }
}
