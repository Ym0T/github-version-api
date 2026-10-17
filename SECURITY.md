# Security

## Reporting a problem

Please don't open a public issue for security problems. Use GitHub's private
reporting instead: **Security** tab → **Report a vulnerability**.

Tell me which version or commit you looked at, what the problem is and, if
you can, how to reproduce it. I'll get back to you as soon as I can.

Only the latest release gets security fixes.

## Running your own instance

- Keep the signing key out of the repository and readable only by the service
  user (`SIGNING_PRIVATE_KEY_FILE`, `chmod 600`).
- Use a fine-grained `GITHUB_TOKEN` without access to private repositories,
  ideally through `GITHUB_TOKEN_FILE`.
- Pin your repository in `ALLOWED_REPOS` (`owner/repo#id`). Archives need it.
- Only the service should be able to write to `DATA_DIR`.
- Run it behind a reverse proxy with TLS, set `TRUSTED_PROXIES` to the exact
  address of the proxy and don't expose the service port directly.
- Your updater has to embed the public key and check the manifest signature,
  repository, versions, hash and age, and refuse anything unsigned or
  unverifiable. [SIGNING.md](SIGNING.md) has the details.
- Only turn on `LEGACY_SIGNATURES` while you still have updaters that don't
  read manifests.
