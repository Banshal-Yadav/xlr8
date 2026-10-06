use clap::Parser;

#[derive(Parser)]
#[command(name = "xlr8", version, about = "GitHub in a box — instant local web viewer for repos, diffs and PRs")]
pub struct Cli {
    /// Repository: full URL (https://github.com/owner/repo) or owner/repo
    #[arg(value_name = "REPO")]
    pub repo: Option<String>,

    /// Do not open the browser
    #[arg(long)]
    pub no_open: bool,

    /// Port to serve on (default: random free port)
    #[arg(long)]
    pub port: Option<u16>,

    /// Serve all previously added repositories
    #[arg(long)]
    pub serve: bool,
}
