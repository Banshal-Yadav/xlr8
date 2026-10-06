mod cli;
mod diff;
mod github;
mod mirror;
mod server;

use anyhow::{bail, Result};
use clap::Parser;
use cli::Cli;

#[derive(Clone)]
pub struct AppState {
    pub gh: github::GitHubClient,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let state = AppState {
        gh: github::GitHubClient::new(),
    };

    let repos_to_sync: Vec<mirror::RepoRef> = match &cli.repo {
        Some(input) => vec![mirror::RepoRef::parse(input)?],
        None if cli.serve => list_cached_repos()?,
        None => {
            let cached = list_cached_repos()?;
            if cached.is_empty() {
                bail!(
                    "no repositories yet\n\nusage:\n  xlr8 owner/repo          open a repository\n  xlr8 https://github.com/owner/repo\n  xlr8 --serve             serve previously opened repos"
                );
            }
            cached
        }
    };

    for repo in &repos_to_sync {
        let r = repo.clone();
        let handle = tokio::task::spawn_blocking(move || mirror::sync(&r));
        match handle.await? {
            Ok(p) => tracing::info!("ready: {}", p.display()),
            Err(e) => tracing::warn!("sync failed for {}: {e}", repo.canonical()),
        }
    }

    let addr = server::serve(state, cli.port).await?;
    let url = format!("http://{addr}");
    tracing::info!("xlr8 serving at {url}");
    if !cli.no_open {
        let _ = open::that_detached(&url);
    }

    // park forever
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

fn list_cached_repos() -> Result<Vec<mirror::RepoRef>> {
    let mirrors = mirror::cache_dir().join("mirrors");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&mirrors) {
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if let Some(slug) = file.strip_suffix(".git") {
                let parts: Vec<&str> = slug.splitn(2, '_').collect();
                if parts.len() == 2 {
                    out.push(mirror::RepoRef {
                        owner: parts[0].to_string(),
                        name: parts[1].to_string(),
                    });
                }
            }
        }
    }
    Ok(out)
}
