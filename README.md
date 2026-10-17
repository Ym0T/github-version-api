<div align="center">

# 🔖 GitHub Version API

**Tells your updater which version of a GitHub project is the latest and hands it a signed ZIP with only the files that changed.**

[![Latest Release](https://img.shields.io/github/v/release/Ym0T/github-version-api?style=for-the-badge&logo=github&color=2ea44f)](https://github.com/Ym0T/github-version-api/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/Ym0T/github-version-api/ci.yml?branch=main&style=for-the-badge&logo=githubactions&logoColor=white&label=CI)](https://github.com/Ym0T/github-version-api/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/Ym0T/github-version-api?style=for-the-badge&color=blue)](LICENSE)
[![Stars](https://img.shields.io/github/stars/Ym0T/github-version-api?style=for-the-badge&logo=github&color=yellow)](https://github.com/Ym0T/github-version-api/stargazers)
[![Last Commit](https://img.shields.io/github/last-commit/Ym0T/github-version-api?style=for-the-badge)](https://github.com/Ym0T/github-version-api/commits/main)

[![Rust](https://img.shields.io/badge/Rust-1.88%2B-B7410E?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Signed updates](https://img.shields.io/badge/updates-Ed25519%20signed-6f42c1?style=for-the-badge)](SIGNING.md)
[![Pterodactyl Nginx Egg](https://img.shields.io/badge/powers-Pterodactyl%20Nginx%20Egg-0E4688?style=for-the-badge)](https://github.com/Ym0T/pterodactyl-nginx-egg)

</div>

<br>

I wrote this for the auto-updater of my [Pterodactyl nginx egg](https://github.com/Ym0T/pterodactyl-nginx-egg).
The updater asks the API for the newest version and then downloads a ZIP that
only contains the files that changed since the installed version. The ZIP
comes with a signed manifest, so the updater can check that it really comes
from me and really belongs to that update.

Nothing in here is specific to the egg. If your project uses tags on GitHub,
you can run your own instance for it with your own signing key. My instance
only serves the egg, so your updater needs its own.

## Table of Contents
- [How it works](#how-it-works)
- [Quick start](#quick-start)
- [Endpoints](#endpoints)
- [Configuration](#configuration)
- [Behind a reverse proxy](#behind-a-reverse-proxy)
- [Security](#security)
- [Development](#development)
- [License](#license)

## How it works

`GET /version/{owner}/{repo}` returns the latest version. The API looks at the
latest release first, then at the newest tag, and finally at a `VERSION` file
or `package.json` on the default branch.

`GET /diff/{owner}/{repo}/{from}/{to}` compares two tags. Both tags are
resolved to their commits first, and the response lists every changed file
together with `from_commit`, `to_commit` and GitHub's `comparison_status`.

Add `?zip=true` and you also get a `download_url` for a ZIP with the changed
files, plus a `manifest` and its `manifest_signature`. The manifest names the
repository, both versions and commits, the SHA-256 of the ZIP and the files
the update deletes.

A ZIP is only built for an update to the latest version, where `to` is a
descendant of `from`. Every file is checked against its git blob hash. If
something doesn't fit, for example a symlink, a submodule or an odd path, the
API refuses to build the archive. It never hands out a partial update.

[SIGNING.md](SIGNING.md) describes the manifest and what an updater has to
check before it installs anything.

## Quick start

Every release has a static Linux binary (x86_64) that runs in any container:

```bash
curl -LO https://github.com/Ym0T/github-version-api/releases/latest/download/version-api-x86_64-linux
curl -LO https://github.com/Ym0T/github-version-api/releases/latest/download/version-api-x86_64-linux.sha256
sha256sum -c version-api-x86_64-linux.sha256
chmod +x version-api-x86_64-linux
./version-api-x86_64-linux generate-keypair
```

To build it yourself you need Rust 1.88 or newer. The build needs about 2 GB
of RAM. On small servers add `-j 1`, then it gets by with a bit over 1 GB.

```bash
cargo build --release --locked
./target/release/version-api generate-keypair
```

`generate-keypair` prints a private key for the server and a public key for
your updater. Save the private key in a file that only the service user can
read and create a `.env` next to the binary (all options are in
[.env.example](.env.example)):

```env
DEFAULT_OWNER=your-name
DEFAULT_REPO=your-project
ALLOWED_REPOS=your-name/your-project
SIGNING_PRIVATE_KEY_FILE=/path/to/signing.key
API_BASE_URL=https://updates.example.com
TRUSTED_PROXIES=127.0.0.1
```

Then start the binary. On the first request it logs the
GitHub id of your repository. Add it to `ALLOWED_REPOS`, for example
`your-name/your-project#123456789`, and restart. You can also look the id up
yourself:

```bash
curl -s https://api.github.com/repos/your-name/your-project | grep -m1 '"id"'
```

ZIPs are only built for pinned repositories. If you ever rename or delete the
repository and someone else registers the old name, the API won't sign
anything for it.

```bash
curl https://updates.example.com/version/your-name/your-project
curl "https://updates.example.com/diff/your-name/your-project/v1.2.0/v1.3.0?zip=true"
```

## Endpoints

| Method | Path | |
|---|---|---|
| GET | `/version` | Latest version of the default repository (`?owner=&repo=` work too) |
| GET | `/version/{repo}` | Latest version, default owner |
| GET | `/version/{owner}/{repo}` | Latest version |
| GET | `/diff/{owner}/{repo}/{from}/{to}` | Changed files between two tags, `?zip=true` for an archive |
| GET | `/diff/{repo}/{from}/{to}` | Same with the default owner |
| GET | `/download/diff/{file}` | Download an archive |
| GET | `/health` | Status, version and uptime |
| GET | `/stats` | Request and cache counters |

Errors are always JSON:
`{"error": "<code>", "message": "...", "request_id": "...", "timestamp": "..."}`.

A note on `comparison_status`: only `ahead` describes a complete update. For
`behind` and `diverged`, GitHub lists the changes since the common ancestor,
so a downgrade shows no files at all.

## Configuration

Everything is set through environment variables or a `.env` file in the
working directory. Invalid or out-of-range values stop the service at
startup. It never quietly falls back to a default.

| Variable | Default | |
|---|---|---|
| `HOST` / `SERVER_PORT` (or `PORT`) | `0.0.0.0` / `3000` | Listen address |
| `DEFAULT_OWNER` / `DEFAULT_REPO` | `Ym0T` / `pterodactyl-nginx-egg` | Repository for the short routes |
| `ALLOWED_REPOS` | the default repository | Comma-separated `owner/repo` or `owner/repo#id`. The `#id` pins the GitHub repository id and is required for ZIPs. `*` additionally serves read-only data (no ZIPs) for any public repository |
| `GITHUB_TOKEN_FILE` / `GITHUB_TOKEN` | – | Raises GitHub's limit from 60 to 5000 requests per hour. Use a fine-grained token **without** access to private repositories |
| `SIGNING_PRIVATE_KEY_FILE` / `SIGNING_PRIVATE_KEY` | – | Ed25519 key for archives, see [SIGNING.md](SIGNING.md) |
| `API_BASE_URL` | – | Public URL used in `download_url`. Required as soon as ZIPs are enabled. Must be https (`http://localhost` is fine for testing) |
| `LEGACY_SIGNATURES` | `false` | Also sign the bare archive hash, for updaters that don't read manifests yet |
| `ALLOW_UNSIGNED_ZIPS` | `false` | Build ZIPs without a signing key |
| `TRUSTED_PROXIES` | – | IPs or CIDRs of your reverse proxy. `X-Forwarded-For` is only used when the request comes from one of them |
| `CORS_ALLOWED_ORIGINS` | – (off) | Comma-separated origins, or `*` |
| `RATE_LIMIT_PER_MINUTE` / `RATE_LIMIT_BURST` | `120` / `30` | Per client IP (IPv6: per /64) |
| `MAX_CONNECTIONS` / `MAX_CONNECTIONS_PER_IP` | `1024` / `32` | Open connections in total and per client address (trusted proxies don't count). Anything above is closed right away |
| `LOOKUP_BUDGET_PER_HOUR` | `500` | GitHub lookups per hour that aren't needed for updates to the latest version: repositories served through `*`, diffs to older versions and early tag refreshes |
| `DATA_DIR` | `/home/container/data` | Statistics, diff cache and archives (`diffs/`) |
| `STATS_FILE_PATH` / `DIFF_CACHE_FILE` | inside `DATA_DIR` | Move single files somewhere else |
| `STATS_SAVE_INTERVAL` | `300` | Seconds between saving the statistics |
| `CACHE_TTL_SECONDS` | `600` | How long a version lookup is cached |
| `DIFF_CACHE_TTL_SECONDS` | `604800` | How long diffs are cached. They are keyed by commit, so they never go stale |
| `HTTP_TIMEOUT_SECONDS` / `REQUEST_TIMEOUT_SECONDS` | `30` / `120` | Timeout for GitHub requests / for a whole request to this API |
| `ZIP_MAX_FILE_MB` / `ZIP_MAX_TOTAL_MB` | `50` / `250` | Size limit per file (at most 1024) and per archive (at most 10240). A build keeps up to about four files in memory |
| `ZIP_STORAGE_QUOTA_MB` | `2048` | Disk space for all archives together |
| `ZIP_RETENTION_HOURS` | `24` | Archives are deleted after this time |
| `ZIP_MAX_CONCURRENT` / `ZIP_MAX_PER_HOUR` / `ZIP_MAX_PER_CLIENT_PER_HOUR` | `2` / `60` / `10` | Limits for building new archives. Handing out an existing one doesn't count |
| `RUST_LOG` | `info` | Log filter |

## Behind a reverse proxy

The service doesn't do TLS itself. Put nginx, Caddy or similar in front of it
and set `TRUSTED_PROXIES` to the exact address the proxy connects from.
Without that, every request seems to come from the proxy and all clients share
one rate limit. Don't use broad ranges like a whole Docker network, every host
in there could then fake client addresses. The proxy has to append the client
address to `X-Forwarded-For`, for nginx:

```nginx
proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
```

If the service port can also be reached directly (on Pterodactyl, allocations
are published on the node's address), block it with a firewall or bind `HOST`
to an internal address.

## Security

Some of the things the API does so that it can be exposed to the internet:

- It only serves repositories from `ALLOWED_REPOS`. They must be public, and
  GitHub has to report the requested name, so renamed or transferred
  repositories are refused. ZIPs additionally need the pinned repository id.
- Owner, repository and version names are validated against strict character
  sets. Versions returned by `/version` are safe to use in shell scripts.
- Diffs only exist between tags. Unknown versions are rejected from the cached
  tag list without asking GitHub.
- Archives only contain regular files from the git tree, each one checked
  against its blob hash. Paths are limited to printable ASCII without `..`,
  absolute paths or `.git`.
- The diff cache on disk is authenticated with a key derived from the signing
  key. Write access to `DATA_DIR` alone is not enough to get something signed.
- Rate limits per client, a limit on open connections, request timeouts, size
  limits for everything that comes from GitHub, bounded caches, and hourly,
  per-client and disk limits for building archives. A build only counts once
  it really starts, so failing requests can't use up the budget.
- Outgoing requests are https only, use the bundled Mozilla root certificates,
  follow redirects only within GitHub, and the token is only sent to the API.
- Files are written with owner-only permissions, atomically and never through
  symlinks.

Found a problem? Please see [SECURITY.md](SECURITY.md).

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests run against a small fake of the GitHub API and don't need network
access.

## License

[MIT](LICENSE)
