//! Configuration from environment variables.
//!
//! Invalid or out-of-range values are rejected at startup instead of
//! silently falling back to defaults, so a typo can never quietly weaken the
//! configuration.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use axum::http::HeaderValue;
use reqwest::Url;
use tracing::warn;
use zeroize::Zeroizing;

use crate::error::ApiError;
use crate::net::IpNet;
use crate::validation::{is_valid_owner, is_valid_repo};

const MIB: u64 = 1024 * 1024;
const TEN_YEARS_SECS: u64 = 10 * 365 * 24 * 3600;
/// Secret files (token, signing key) are tiny; anything bigger is a mistake.
const MAX_SECRET_FILE_BYTES: u64 = 4096;

/// A secret value that is zeroed on drop and never printed.
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// A GitHub repository (`owner/name`) whose parts passed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    /// Lowercased `owner/name`, GitHub names are case-insensitive.
    pub fn cache_key(&self) -> String {
        format!("{}/{}", self.owner, self.name).to_ascii_lowercase()
    }

    pub fn matches(&self, owner: &str, name: &str) -> bool {
        self.owner.eq_ignore_ascii_case(owner) && self.name.eq_ignore_ascii_case(name)
    }
}

impl FromStr for RepoRef {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let (owner, name) = spec
            .trim()
            .split_once('/')
            .ok_or_else(|| format!("'{spec}' is not in the form owner/repo"))?;
        if !is_valid_owner(owner) || !is_valid_repo(name) {
            return Err(format!("'{spec}' is not a valid GitHub repository name"));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }
}

impl fmt::Display for RepoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// A repository that passed validation and the access policy.
#[derive(Debug, Clone)]
pub struct ResolvedRepo {
    pub repo: RepoRef,
    /// Explicitly listed in `ALLOWED_REPOS`. Only trusted repositories get
    /// (signed) ZIP archives and are exempt from the public lookup budget.
    pub trusted: bool,
    /// Expected GitHub repository id (`owner/repo#id` in `ALLOWED_REPOS`).
    pub pinned_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrustedRepo {
    repo: RepoRef,
    id: Option<u64>,
}

/// Which repositories the API serves.
#[derive(Debug, Clone)]
pub struct RepoPolicy {
    trusted: Vec<TrustedRepo>,
    allow_any_public: bool,
}

impl RepoPolicy {
    /// Parses `ALLOWED_REPOS`: a comma-separated list of `owner/repo` or
    /// `owner/repo#id` entries, optionally including `*` to also serve
    /// read-only data for any public repository. Defaults to the default
    /// repository only.
    pub fn parse(spec: Option<&str>, default_repo: &RepoRef) -> anyhow::Result<Self> {
        let Some(spec) = spec else {
            return Ok(Self {
                trusted: vec![TrustedRepo {
                    repo: default_repo.clone(),
                    id: None,
                }],
                allow_any_public: false,
            });
        };

        let mut policy = Self {
            trusted: Vec::new(),
            allow_any_public: false,
        };
        for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
            if entry == "*" {
                policy.allow_any_public = true;
                continue;
            }
            let (name, id) = match entry.split_once('#') {
                Some((name, id)) => {
                    let id = id.parse::<u64>().map_err(|_| {
                        anyhow!("ALLOWED_REPOS: '{entry}' has an invalid repository id")
                    })?;
                    (name, Some(id))
                }
                None => (entry, None),
            };
            let repo: RepoRef = name.parse().map_err(|e| anyhow!("ALLOWED_REPOS: {e}"))?;
            if policy
                .trusted
                .iter()
                .any(|t| t.repo.matches(&repo.owner, &repo.name))
            {
                bail!("ALLOWED_REPOS: {repo} is listed more than once");
            }
            policy.trusted.push(TrustedRepo { repo, id });
        }
        if policy.trusted.is_empty() && !policy.allow_any_public {
            bail!("ALLOWED_REPOS does not contain any repository");
        }
        if policy
            .resolve(&default_repo.owner, &default_repo.name)
            .is_err()
        {
            bail!("DEFAULT_OWNER/DEFAULT_REPO ({default_repo}) must be covered by ALLOWED_REPOS");
        }
        Ok(policy)
    }

    /// Validates `owner`/`repo` and checks them against the policy.
    pub fn resolve(&self, owner: &str, repo: &str) -> Result<ResolvedRepo, ApiError> {
        if !is_valid_owner(owner) {
            return Err(ApiError::bad_request(
                "invalid_owner",
                "Owner must be 1-39 characters: letters, digits and single hyphens",
            ));
        }
        if !is_valid_repo(repo) {
            return Err(ApiError::bad_request(
                "invalid_repo",
                "Repository must be 1-100 characters: letters, digits, '-', '_' and '.'",
            ));
        }

        if let Some(trusted) = self.trusted.iter().find(|t| t.repo.matches(owner, repo)) {
            return Ok(ResolvedRepo {
                repo: trusted.repo.clone(),
                trusted: true,
                pinned_id: trusted.id,
            });
        }
        if self.allow_any_public {
            return Ok(ResolvedRepo {
                repo: RepoRef {
                    owner: owner.to_ascii_lowercase(),
                    name: repo.to_ascii_lowercase(),
                },
                trusted: false,
                pinned_id: None,
            });
        }
        Err(ApiError::forbidden(
            "repository_not_allowed",
            "This repository is not served by this API instance",
        ))
    }

    pub fn trusted_names(&self) -> Vec<String> {
        self.trusted
            .iter()
            .map(|t| match t.id {
                Some(id) => format!("{}#{id}", t.repo),
                None => t.repo.to_string(),
            })
            .collect()
    }

    pub fn allows_any_public(&self) -> bool {
        self.allow_any_public
    }
}

#[derive(Debug, Clone)]
pub enum CorsOrigins {
    Any,
    List(Vec<HeaderValue>),
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Time to replenish one request token.
    pub period: Duration,
    pub burst: u32,
}

#[derive(Debug, Clone)]
pub struct ZipConfig {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub storage_quota_bytes: u64,
    pub retention: Duration,
    pub max_concurrent_builds: usize,
    pub max_builds_per_hour: u32,
    /// New archives a single client may trigger per hour.
    pub max_builds_per_client_per_hour: u32,
    pub allow_unsigned: bool,
    /// Also emit the legacy signature (hash only, not bound to a request).
    pub legacy_signatures: bool,
}

impl ZipConfig {
    /// Existing archives are only handed out while clients still have at
    /// least a sixth of the retention time to download them.
    pub fn reuse_window(&self) -> Duration {
        self.retention.saturating_sub(self.retention / 6)
    }
}

#[derive(Debug)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub max_connections: usize,
    /// Per client address (IPv6: per /64); trusted proxies are exempt.
    pub max_connections_per_ip: usize,
    pub default_repo: RepoRef,
    pub repo_policy: RepoPolicy,
    pub github_token: Option<Secret>,
    /// Base64 Ed25519 seed, if configured.
    pub signing_key: Option<Secret>,
    pub http_timeout: Duration,
    pub request_timeout: Duration,
    pub version_cache_ttl: Duration,
    pub diff_cache_ttl: Duration,
    /// GitHub lookups per hour that updates to the latest version don't need:
    /// repositories allowed only via `*` and diffs to older versions.
    pub lookup_budget_per_hour: u32,
    pub diffs_dir: PathBuf,
    pub stats_file: PathBuf,
    pub diff_cache_file: PathBuf,
    pub stats_save_interval: Duration,
    /// Public URL for download links. Required when archives are enabled.
    pub public_base_url: Option<String>,
    pub cors: CorsOrigins,
    pub trusted_proxies: Vec<IpNet>,
    pub rate_limit: RateLimitConfig,
    pub zip: ZipConfig,
}

/// Reads configuration values from a lookup function (the process
/// environment in production, a map in tests).
struct Env<F: Fn(&str) -> Option<String>>(F);

impl<F: Fn(&str) -> Option<String>> Env<F> {
    fn opt(&self, name: &str) -> Option<String> {
        (self.0)(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// A positive integer not larger than `max`.
    fn bounded(&self, name: &str, default: u64, max: u64) -> anyhow::Result<u64> {
        let value = match self.opt(name) {
            Some(raw) => raw
                .parse::<u64>()
                .map_err(|_| anyhow!("{name} has an invalid value: '{raw}'"))?,
            None => default,
        };
        if value == 0 || value > max {
            bail!("{name} must be between 1 and {max}");
        }
        Ok(value)
    }

    fn seconds(&self, name: &str, default: u64, max: u64) -> anyhow::Result<Duration> {
        self.bounded(name, default, max).map(Duration::from_secs)
    }

    fn flag(&self, name: &str, default: bool) -> anyhow::Result<bool> {
        match self.opt(name).map(|v| v.to_ascii_lowercase()) {
            None => Ok(default),
            Some(v) if matches!(v.as_str(), "1" | "true" | "yes" | "on") => Ok(true),
            Some(v) if matches!(v.as_str(), "0" | "false" | "no" | "off") => Ok(false),
            Some(v) => bail!("{name} must be true or false, got '{v}'"),
        }
    }

    /// A secret from `<NAME>_FILE` (preferred, keeps it out of the process
    /// environment) or `<NAME>`.
    fn secret(&self, name: &str) -> anyhow::Result<Option<Secret>> {
        let file_var = format!("{name}_FILE");
        match (self.opt(&file_var), (self.0)(name).map(Zeroizing::new)) {
            (Some(_), Some(inline)) if !inline.trim().is_empty() => {
                bail!("set either {name} or {file_var}, not both")
            }
            (Some(path), _) => read_secret_file(Path::new(&path))
                .with_context(|| format!("cannot read {file_var}"))
                .map(Some),
            (None, Some(inline)) => {
                let value = inline.trim();
                Ok((!value.is_empty()).then(|| Secret::new(value.to_string())))
            }
            (None, None) => Ok(None),
        }
    }
}

fn read_secret_file(path: &Path) -> anyhow::Result<Secret> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_SECRET_FILE_BYTES {
        bail!("{} is too large for a secret", path.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            warn!(
                "{} is accessible by other users, restrict it with chmod 600",
                path.display()
            );
        }
    }
    let content = Zeroizing::new(std::fs::read_to_string(path)?);
    let value = content.trim();
    if value.is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok(Secret::new(value.to_string()))
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let env = Env(lookup);

        let port_raw = env.opt("SERVER_PORT").or_else(|| env.opt("PORT"));
        let port = match port_raw {
            Some(raw) => raw
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| anyhow!("SERVER_PORT/PORT has an invalid value: '{raw}'"))?,
            None => 3000,
        };

        let default_repo: RepoRef = format!(
            "{}/{}",
            env.opt("DEFAULT_OWNER").unwrap_or_else(|| "Ym0T".into()),
            env.opt("DEFAULT_REPO")
                .unwrap_or_else(|| "pterodactyl-nginx-egg".into())
        )
        .parse()
        .map_err(|e| anyhow!("DEFAULT_OWNER/DEFAULT_REPO: {e}"))?;
        let repo_policy = RepoPolicy::parse(env.opt("ALLOWED_REPOS").as_deref(), &default_repo)?;

        let github_token = env.secret("GITHUB_TOKEN")?;
        if let Some(token) = &github_token
            && !token.expose().bytes().all(|b| b.is_ascii_graphic())
        {
            bail!("GITHUB_TOKEN contains invalid characters");
        }
        let signing_key = env.secret("SIGNING_PRIVATE_KEY")?;

        let data_dir = PathBuf::from(
            env.opt("DATA_DIR")
                .unwrap_or_else(|| "/home/container/data".into()),
        );
        let stats_file = env
            .opt("STATS_FILE_PATH")
            .map_or_else(|| data_dir.join("stats.json"), PathBuf::from);
        let diff_cache_file = env
            .opt("DIFF_CACHE_FILE")
            .map_or_else(|| data_dir.join("diff_cache.json"), PathBuf::from);

        let public_base_url = env
            .opt("API_BASE_URL")
            .map(|raw| parse_public_base_url(&raw))
            .transpose()?;
        let cors = parse_cors_origins(env.opt("CORS_ALLOWED_ORIGINS").as_deref().unwrap_or(""))?;
        let trusted_proxies = parse_trusted_proxies(env.opt("TRUSTED_PROXIES").as_deref())?;

        let per_minute = env.bounded("RATE_LIMIT_PER_MINUTE", 120, 60_000)?;
        let rate_limit = RateLimitConfig {
            period: Duration::from_millis(60_000 / per_minute),
            burst: u32::try_from(env.bounded("RATE_LIMIT_BURST", 30, 100_000)?)?,
        };

        let zip = zip_config(&env)?;
        if zip.legacy_signatures && signing_key.is_none() {
            bail!("LEGACY_SIGNATURES requires a signing key");
        }
        if (signing_key.is_some() || zip.allow_unsigned) && public_base_url.is_none() {
            bail!(
                "API_BASE_URL is required when ZIP archives are enabled (signing key or ALLOW_UNSIGNED_ZIPS)"
            );
        }

        Ok(Self {
            host: env.opt("HOST").unwrap_or_else(|| "0.0.0.0".into()),
            port,
            max_connections: usize::try_from(env.bounded("MAX_CONNECTIONS", 1024, 100_000)?)?,
            max_connections_per_ip: usize::try_from(env.bounded(
                "MAX_CONNECTIONS_PER_IP",
                32,
                100_000,
            )?)?,
            default_repo,
            repo_policy,
            github_token,
            signing_key,
            http_timeout: env.seconds("HTTP_TIMEOUT_SECONDS", 30, 600)?,
            request_timeout: env.seconds("REQUEST_TIMEOUT_SECONDS", 120, 3600)?,
            version_cache_ttl: env.seconds("CACHE_TTL_SECONDS", 600, TEN_YEARS_SECS)?,
            diff_cache_ttl: env.seconds("DIFF_CACHE_TTL_SECONDS", 7 * 24 * 3600, TEN_YEARS_SECS)?,
            lookup_budget_per_hour: u32::try_from(env.bounded(
                "LOOKUP_BUDGET_PER_HOUR",
                500,
                1_000_000,
            )?)?,
            diffs_dir: data_dir.join("diffs"),
            stats_file,
            diff_cache_file,
            stats_save_interval: env.seconds("STATS_SAVE_INTERVAL", 300, 86_400)?,
            public_base_url,
            cors,
            trusted_proxies,
            rate_limit,
            zip,
        })
    }

    #[cfg(test)]
    pub fn for_tests(vars: &[(&str, &str)]) -> anyhow::Result<Self> {
        let vars: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        Self::from_lookup(move |name| vars.get(name).cloned())
    }

    /// Whether `/diff?zip=true` can produce archives at all.
    pub fn zip_enabled(&self) -> bool {
        self.signing_key.is_some() || self.zip.allow_unsigned
    }
}

fn zip_config<F: Fn(&str) -> Option<String>>(env: &Env<F>) -> anyhow::Result<ZipConfig> {
    // Files are held in memory while an archive is built (a few per build),
    // so file and archive sizes are capped.
    let mib = |name: &str, default: u64, max: u64| -> anyhow::Result<u64> {
        Ok(env.bounded(name, default, max)? * MIB)
    };
    let zip = ZipConfig {
        max_file_bytes: mib("ZIP_MAX_FILE_MB", 50, 1024)?,
        max_total_bytes: mib("ZIP_MAX_TOTAL_MB", 250, 10 * 1024)?,
        storage_quota_bytes: mib("ZIP_STORAGE_QUOTA_MB", 2048, 1024 * 1024)?,
        retention: Duration::from_secs(env.bounded("ZIP_RETENTION_HOURS", 24, 24 * 365)? * 3600),
        max_concurrent_builds: usize::try_from(env.bounded("ZIP_MAX_CONCURRENT", 2, 64)?)?,
        max_builds_per_hour: u32::try_from(env.bounded("ZIP_MAX_PER_HOUR", 60, 100_000)?)?,
        max_builds_per_client_per_hour: u32::try_from(env.bounded(
            "ZIP_MAX_PER_CLIENT_PER_HOUR",
            10,
            100_000,
        )?)?,
        allow_unsigned: env.flag("ALLOW_UNSIGNED_ZIPS", false)?,
        legacy_signatures: env.flag("LEGACY_SIGNATURES", false)?,
    };
    if zip.max_file_bytes > zip.max_total_bytes {
        bail!("ZIP_MAX_FILE_MB must not exceed ZIP_MAX_TOTAL_MB");
    }
    if zip.max_total_bytes > zip.storage_quota_bytes {
        bail!("ZIP_MAX_TOTAL_MB must not exceed ZIP_STORAGE_QUOTA_MB");
    }
    Ok(zip)
}

fn parse_trusted_proxies(raw: Option<&str>) -> anyhow::Result<Vec<IpNet>> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<IpNet>()
                .map_err(|e| anyhow!("TRUSTED_PROXIES: {e}"))
        })
        .collect()
}

fn parse_public_base_url(raw: &str) -> anyhow::Result<String> {
    let url = Url::parse(raw).map_err(|_| anyhow!("API_BASE_URL is not a valid URL"))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
    if !scheme_ok
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("API_BASE_URL must be an https:// URL without credentials, query or fragment");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn parse_cors_origins(raw: &str) -> anyhow::Result<CorsOrigins> {
    if raw.trim() == "*" {
        return Ok(CorsOrigins::Any);
    }
    let mut origins = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let url = Url::parse(entry)
            .map_err(|_| anyhow!("CORS_ALLOWED_ORIGINS: '{entry}' is not a valid origin"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
        {
            bail!(
                "CORS_ALLOWED_ORIGINS: '{entry}' must be a plain origin like https://example.com"
            );
        }
        let origin = url.origin().ascii_serialization();
        origins
            .push(HeaderValue::from_str(&origin).context("CORS_ALLOWED_ORIGINS: invalid origin")?);
    }
    Ok(CorsOrigins::List(origins))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> anyhow::Result<AppConfig> {
        AppConfig::for_tests(vars)
    }

    #[test]
    fn defaults_are_valid() {
        let cfg = config(&[]).unwrap();
        assert_eq!(cfg.default_repo.to_string(), "Ym0T/pterodactyl-nginx-egg");
        assert_eq!(
            cfg.repo_policy.trusted_names(),
            ["Ym0T/pterodactyl-nginx-egg"]
        );
        assert!(!cfg.repo_policy.allows_any_public());
        assert_eq!(cfg.rate_limit.period, Duration::from_millis(500));
        assert!(!cfg.zip.allow_unsigned && !cfg.zip.legacy_signatures);
        assert!(!cfg.zip_enabled());
        assert!(cfg.public_base_url.is_none());
        assert!(matches!(&cfg.cors, CorsOrigins::List(list) if list.is_empty()));
    }

    #[test]
    fn rejects_invalid_values_instead_of_defaulting() {
        for vars in [
            &[("SERVER_PORT", "abc")][..],
            &[("CACHE_TTL_SECONDS", "0")],
            &[("CACHE_TTL_SECONDS", "999999999999")],
            &[("ZIP_RETENTION_HOURS", "2000000000000000")],
            &[("ZIP_MAX_CONCURRENT", "100000")],
            &[("ALLOW_UNSIGNED_ZIPS", "maybe")],
            &[("API_BASE_URL", "http://example.com")],
            &[("API_BASE_URL", "https://user:pw@example.com")],
            &[("CORS_ALLOWED_ORIGINS", "https://example.com/path")],
            &[("TRUSTED_PROXIES", "10.0.0.0/99")],
            &[("DEFAULT_OWNER", "../x")],
            &[("ALLOWED_REPOS", "someone/else")],
            &[("ALLOWED_REPOS", "not-a-repo")],
            &[("ALLOWED_REPOS", "Ym0T/pterodactyl-nginx-egg#abc")],
            &[(
                "ALLOWED_REPOS",
                "Ym0T/pterodactyl-nginx-egg,ym0t/PTERODACTYL-nginx-egg",
            )],
            &[("ZIP_MAX_FILE_MB", "500")],
            &[("ALLOW_UNSIGNED_ZIPS", "true")],
            &[("LEGACY_SIGNATURES", "true")],
            &[
                ("SIGNING_PRIVATE_KEY", "abc"),
                ("SIGNING_PRIVATE_KEY_FILE", "/x"),
            ],
        ] {
            assert!(config(vars).is_err(), "{vars:?}");
        }
    }

    #[test]
    fn archives_need_a_public_url() {
        assert!(config(&[("SIGNING_PRIVATE_KEY", "key")]).is_err());
        let cfg = config(&[
            ("SIGNING_PRIVATE_KEY", "key"),
            ("API_BASE_URL", "https://api.example/"),
        ])
        .unwrap();
        assert!(cfg.zip_enabled());
        assert_eq!(cfg.public_base_url.as_deref(), Some("https://api.example"));
    }

    #[test]
    fn secrets_from_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, "ghp_example\n").unwrap();
        let path = path.to_string_lossy().into_owned();
        let cfg = config(&[("GITHUB_TOKEN_FILE", &path)]).unwrap();
        assert_eq!(cfg.github_token.unwrap().expose(), "ghp_example");
        assert!(config(&[("GITHUB_TOKEN_FILE", "/does/not/exist")]).is_err());
    }

    #[test]
    fn repo_policy() {
        let default: RepoRef = "Ym0T/pterodactyl-nginx-egg".parse().unwrap();
        let policy = RepoPolicy::parse(None, &default).unwrap();

        let resolved = policy.resolve("ym0t", "PTERODACTYL-nginx-egg").unwrap();
        assert!(resolved.trusted);
        assert_eq!(resolved.repo, default, "uses the configured spelling");

        let err = policy.resolve("someone", "else").unwrap_err();
        assert_eq!(err.code(), "repository_not_allowed");
        assert_eq!(
            policy.resolve("x", "../../user").unwrap_err().code(),
            "invalid_repo"
        );
        assert_eq!(
            policy.resolve("x/y", "repo").unwrap_err().code(),
            "invalid_owner"
        );

        let open = RepoPolicy::parse(Some("Ym0T/pterodactyl-nginx-egg#42, *"), &default).unwrap();
        assert_eq!(
            open.resolve("Ym0T", "pterodactyl-nginx-egg")
                .unwrap()
                .pinned_id,
            Some(42)
        );
        let other = open.resolve("Some-One", "Repo").unwrap();
        assert!(!other.trusted);
        assert_eq!(other.repo.to_string(), "some-one/repo");
    }

    #[test]
    fn cors_origins() {
        assert!(matches!(parse_cors_origins("*").unwrap(), CorsOrigins::Any));
        let CorsOrigins::List(list) =
            parse_cors_origins("https://a.example, http://localhost:3000").unwrap()
        else {
            panic!("expected list");
        };
        assert_eq!(list, vec!["https://a.example", "http://localhost:3000"]);
    }
}
