//! GitHub Version API: latest versions of GitHub repositories and signed
//! diff archives for auto-updaters.
mod app;
mod config;
mod error;
mod handlers;
mod models;
mod net;
mod server;
mod services;
mod state;
mod utils;
mod validation;

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod test_support;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::{AppConfig, CorsOrigins};
use crate::server::ServerLimits;
use crate::services::signing::{Signer, generate_keypair};
use crate::state::AppState;

const USAGE: &str = "\
Usage: version-api [COMMAND]

Without a command the HTTP server is started. Configuration is read from
environment variables (and a .env file in the working directory), see README.md.

Commands:
  generate-keypair   Print a new Ed25519 key pair for signing update archives
  -h, --help         Show this help
  -V, --version      Show the version
";

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("generate-keypair") => {
            let (private_key, public_key) = generate_keypair()?;
            println!("# Keep the private key secret (e.g. in a file with chmod 600).");
            println!("SIGNING_PRIVATE_KEY={}", private_key.as_str());
            println!("# Embed the public key in the auto-updater.");
            println!("PUBLIC_KEY_BASE64={public_key}");
            return Ok(());
        }
        Some("-h" | "--help") => {
            print!("{USAGE}");
            return Ok(());
        }
        Some("-V" | "--version") => {
            println!("version-api {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(other) => bail!("unknown argument '{other}', see --help"),
    }

    // Modifies the process environment, so it must run before any other
    // threads (including the Tokio runtime) exist.
    let _ = dotenvy::dotenv();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = AppConfig::from_env().context("invalid configuration")?;
    let signer = config
        .signing_key
        .as_ref()
        .map(|key| Signer::from_base64(key.expose()))
        .transpose()
        .context("invalid signing key")?;
    log_startup(&config, signer.as_ref());

    let state = AppState::new(config, signer).await?;
    services::cleanup::spawn(state.config.diffs_dir.clone(), state.config.zip.retention);
    utils::persistent_stats::spawn_saver(
        Arc::clone(&state),
        state.config.stats_save_interval,
        state.config.stats_file.clone(),
    );
    utils::persistent_diff_cache::DiffCacheStore::spawn_flusher(Arc::clone(&state));

    let router = app::build_router(&state)?;
    let address = (state.config.host.as_str(), state.config.port);
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("cannot listen on {}:{}", address.0, address.1))?;
    info!("listening on {}", listener.local_addr()?);

    let limits = ServerLimits {
        // Clients must send request headers within this time (slowloris).
        first_request_timeout: Duration::from_secs(10),
        header_read_timeout: Duration::from_secs(10),
        max_connection_age: Duration::from_secs(600),
        max_connections: state.config.max_connections,
        max_connections_per_ip: state.config.max_connections_per_ip,
        trusted_proxies: state.config.trusted_proxies.clone(),
        shutdown_grace: Duration::from_secs(20),
    };
    // State is saved as soon as the signal arrives, before waiting for
    // open connections, and once more at the end.
    server::serve(listener, router, limits, shutdown_signal(), async {
        info!("shutdown signal received, saving state");
        state.persist().await;
    })
    .await;
    state.persist().await;
    info!("stopped");
    Ok(())
}

fn log_startup(config: &AppConfig, signer: Option<&Signer>) {
    info!("GitHub Version API v{}", env!("CARGO_PKG_VERSION"));
    info!(
        "repositories: {}{}",
        config.repo_policy.trusted_names().join(", "),
        if config.repo_policy.allows_any_public() {
            " (+ read-only access to any public repository)"
        } else {
            ""
        }
    );
    match signer {
        Some(signer) => info!(
            "archive signing enabled, public key: {}",
            signer.public_key_b64()
        ),
        None if config.zip.allow_unsigned => {
            warn!("ALLOW_UNSIGNED_ZIPS is set: ZIP archives are NOT signed");
        }
        None => warn!("no signing key configured: ZIP archives are disabled"),
    }
    if config.zip.legacy_signatures {
        warn!(
            "LEGACY_SIGNATURES is enabled: legacy signatures are not bound to repository or versions, \
             disable them once all clients verify manifests"
        );
    }
    if config.zip_enabled() {
        let unpinned: Vec<String> = config
            .repo_policy
            .trusted_names()
            .into_iter()
            .filter(|name| !name.contains('#'))
            .collect();
        if !unpinned.is_empty() {
            warn!(
                "no archives for {} until the repository id is pinned in ALLOWED_REPOS (owner/repo#id, the id is logged on first use)",
                unpinned.join(", ")
            );
        }
    }
    if config.github_token.is_some() {
        info!("GitHub token configured (5000 requests/hour)");
    } else {
        warn!("no GITHUB_TOKEN set: GitHub allows only 60 requests/hour");
    }
    if config.trusted_proxies.is_empty() {
        warn!(
            "TRUSTED_PROXIES is not set: rate limiting uses the TCP peer address. \
             Behind a reverse proxy, set it to the proxy address, otherwise all clients share one limit"
        );
    } else if config.trusted_proxies.iter().any(net::IpNet::is_broad) {
        warn!(
            "TRUSTED_PROXIES contains broad networks: every host in them can spoof client addresses. \
             Prefer the exact proxy address"
        );
    }
    if matches!(config.cors, CorsOrigins::Any) {
        info!("CORS: any origin");
    }
    info!(
        "rate limit: {} requests/minute per client (burst {})",
        60_000 / config.rate_limit.period.as_millis().max(1),
        config.rate_limit.burst
    );
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!("cannot listen for Ctrl+C: {e}");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => {
                warn!("cannot listen for SIGTERM: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
