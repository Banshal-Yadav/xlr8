mod cli;
mod diff;
mod github;
mod log;
mod mirror;
mod repo;
mod rewrite;
mod server;

use anyhow::{bail, Result};

#[derive(Clone)]
pub struct AppState {
    pub gh: github::GitHubClient,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::parse()?;
    let state = AppState {
        gh: github::GitHubClient::new(),
    };

    let repos_to_sync: Vec<mirror::RepoRef> = match &cli.repo {
        Some(input) => vec![mirror::RepoRef::parse(input)?],
        None if cli.serve => mirror::cached_repos(),
        None => {
            let cached = mirror::cached_repos();
            if cached.is_empty() {
                bail!(
                    "no repositories yet\n\nusage:\n  xlr8 owner/repo          open a repository\n  xlr8 https://github.com/owner/repo\n  xlr8 --serve             serve previously opened repos"
                );
            }
            cached
        }
    };

    for r in &repos_to_sync {
        let handle = tokio::task::spawn_blocking({
            let r = r.clone();
            move || mirror::sync(&r)
        });
        match handle.await? {
            Ok(p) => crate::info!("ready: {}", p.display()),
            Err(e) => crate::warn!("sync failed for {}: {e}", r.canonical()),
        }
    }

    let addr = server::serve(state, cli.port).await?;
    let url = format!("http://{addr}");
    crate::info!("xlr8 serving at {url}");
    if !cli.no_open {
        open_url(&url);
    }

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

fn open_url(url: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
