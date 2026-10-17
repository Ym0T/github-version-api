# Signing

Diff archives (`/diff/...?zip=true`) are signed with Ed25519. The point is
that an updater can check two things before it installs anything: the archive
was built by your API instance, and it was built for exactly the update the
updater asked for.

## When the API builds an archive

Only if all of these hold:

- The repository is listed in `ALLOWED_REPOS` with its id
  (`owner/repo#id`, the id is logged on first use). Never through `*`. If
  someone registers the name after a rename, it's a different id and nothing
  gets signed.
- `from` and `to` are existing **tags**. The diff is computed between the
  commits they point to.
- `to` is a descendant of `from` (GitHub status `ahead`), so applying the diff
  turns `from` into exactly `to`.
- `to` is the latest version that `/version` reports. Downgrades are never
  signed.
- Every path is safe (printable ASCII, no `..`, no absolute paths, no `.git`),
  no file turns into a directory or back, and every file is a regular file
  (no symlinks or submodules) that matches its git blob hash.

If any of this fails, there's no archive at all. Without a signing key,
archives are off unless you set `ALLOW_UNSIGNED_ZIPS=true`.

## What gets signed

| Field | Signature over | |
|---|---|---|
| `manifest` + `manifest_signature` | the exact bytes of `manifest` | This is what updaters should check. It binds the archive to repository, versions, commits, hash and deletions. |
| `signature` | the raw 32-byte SHA-256 of the archive (`checksum` is the hex form) | Only with `LEGACY_SIGNATURES=true`. Not bound to anything, see [below](#updaters-that-only-check-the-hash). |

The manifest is plain text. One `key=value` per line, every line ends with
`\n`, followed by one `delete=` line for each file the update removes (deleted
and renamed files, sorted):

```text
format=version-api-update-manifest/1
repository=Ym0T/pterodactyl-nginx-egg
from=v3.2.3
to=v3.2.5
from_commit=da308f62891fa3855cd2851194cac7bcbe7782d6
to_commit=5d5b3f9b522f65f6845a69e558090827b2c88a4e
sha256=920fc17ac13aef4cfaa567af209fd3107c7d01d5b8b408cf6dd7eb5cc4ee01d8
size=25637
created_at=1792165817
delete=nginx/sites-available/default
```

`repository` is spelled as in `ALLOWED_REPOS` (GitHub names are
case-insensitive). `created_at` is a Unix timestamp. When parsing, split lines
**only** at `\n` (not at other Unicode line breaks) and each line at the first
`=`. Values never contain `\n`, keys never contain `=`.

The archive contains exactly the changed files of `to`, nothing else.
Executable files keep their mode from the git tree.

## Setup

### 1. Create a key pair

```bash
version-api generate-keypair
```

This prints `SIGNING_PRIVATE_KEY=...` and `PUBLIC_KEY_BASE64=...`.

### 2. Give the private key to the API

Put the base64 private key into a file that only the service user can read and
point `SIGNING_PRIVATE_KEY_FILE` to it. That keeps the key out of the process
environment.

```bash
umask 077
printf '%s\n' 'BASE64_PRIVATE_KEY' > /path/outside/backups/signing.key
export SIGNING_PRIVATE_KEY_FILE=/path/outside/backups/signing.key
```

`SIGNING_PRIVATE_KEY` works too, but not both at once. An invalid key stops
the service at startup, and the public key is logged when it starts.
`API_BASE_URL` is required as soon as archives are enabled.

On Pterodactyl, everything under `/home/container` shows up in the file
manager for subusers with file access and ends up in backups. If that matters
to you, pass the key as a startup variable that isn't user-viewable instead.

The diff cache in `DATA_DIR/diff_cache.json` decides what gets signed, so it
is authenticated with a key derived from the signing key. A modified file is
simply ignored. Still, keep `DATA_DIR` writable only by the service: someone
who can change the archives in there can't forge a signature, but they can
make updates fail.

### 3. Put the public key into the updater

Embed `PUBLIC_KEY_BASE64` in your updater. **Never** verify against the
`public_key` field of an API response. Whoever can tamper with the response
can replace that field as well.

## Checking an archive in the updater

Accept an archive only if **all** of these hold, otherwise refuse the update.
Don't fall back to installing unsigned or unverified archives.

1. The manifest signature verifies with your embedded public key.
2. `format` is `version-api-update-manifest/1`, `repository` is your
   repository (compare case-insensitively), and `from`/`to` are the installed
   and the requested version.
3. The SHA-256 of the downloaded file equals `sha256` in the manifest.
4. `created_at` is recent. Archives live for `ZIP_RETENTION_HOURS` (24 h by
   default), so something like 48 h is a good limit. Without this check,
   someone who controls the network could keep replaying an old but genuinely
   signed manifest and hold a client back on an old version.

Then apply the update:

1. Extract into an empty staging directory and check every path again
   (relative, no `..`, no `.git`).
2. Delete exactly the `delete=` paths from the manifest, **before** copying,
   so renames that only change the case work on case-insensitive file
   systems. Check each path again, delete only regular files
   (`rm -f -- "$root/$path"`, never `-r`) and skip paths below a symlinked
   directory. Never take deletions from the unsigned `files_changed` list.
3. Copy the staged files into the installation, ideally after a backup so a
   failed copy can be rolled back, and record the new version.

Here's a reference implementation for a POSIX shell with OpenSSL 3 and `jq`:

```sh
#!/bin/sh
# Usage: verify-update.sh RESPONSE_JSON ARCHIVE REPOSITORY FROM TO
# Prints the paths to delete on success; prints nothing and exits non-zero
# otherwise.
set -eu
PUBLIC_KEY_BASE64="your_base64_public_key_here"
response=$1 archive=$2 repository=$3 from=$4 to=$5

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Pinned public key -> PEM (DER prefix for a raw Ed25519 public key)
{ printf '\060\052\060\005\006\003\053\145\160\003\041\000'; printf '%s' "$PUBLIC_KEY_BASE64" | base64 -d; } > "$tmp/pub.der"
openssl pkey -pubin -inform DER -in "$tmp/pub.der" -out "$tmp/pub.pem"

jq -j '.manifest' "$response" > "$tmp/manifest"
jq -r '.manifest_signature' "$response" | base64 -d > "$tmp/manifest.sig"

openssl pkeyutl -verify -pubin -inkey "$tmp/pub.pem" -rawin \
    -in "$tmp/manifest" -sigfile "$tmp/manifest.sig" >/dev/null

field() { sed -n "s/^$1=//p" "$tmp/manifest"; }
lower() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]'; }
[ "$(field format)" = "version-api-update-manifest/1" ]
[ "$(lower "$(field repository)")" = "$(lower "$repository")" ]
[ "$(field from)" = "$from" ]
[ "$(field to)" = "$to" ]
[ "$(sha256sum "$archive" | cut -d' ' -f1)" = "$(field sha256)" ]
created=$(field created_at)
case "$created" in "" | *[!0-9]*) exit 1 ;; esac
[ $(( $(date +%s) - created )) -lt 172800 ]   # at most 48 hours old

# Signed deletions: check all of them first, print only if all are safe.
field delete > "$tmp/delete"
while IFS= read -r path; do
    case "/$path/" in
        *"/../"* | *"/./"* | *"//"* | *"/.git/"*) echo "unsafe path: $path" >&2; exit 1 ;;
    esac
done < "$tmp/delete"
cat "$tmp/delete"
```

`jq -j` writes the manifest without adding a newline, so the bytes are exactly
the ones that were signed.

## Updaters that only check the hash

Some updaters only check `signature` against the SHA-256 of the archive. That
signature says "this key signed an archive with this hash" and nothing more.
It doesn't name a repository or a version, so such an updater accepts **any**
archive that was ever signed with the key, including old ones.

`LEGACY_SIGNATURES=true` adds this signature so those updaters keep working
while you ship one that checks the manifest. The same key can be used for
both: a manifest is always longer than 32 bytes, so a hash signature can never
pass as a manifest signature or the other way round. That also means a
switch to manifests doesn't require a new key.

How to switch:

1. Enable `LEGACY_SIGNATURES` and release an updater that verifies the
   manifest as described above and ignores `signature` completely.
2. Old installations pick up the new updater through a normal update.
3. Once (nearly) all installations have it, turn `LEGACY_SIGNATURES` off.
   Installations that still run the old updater then can't update anymore,
   but they don't become less safe either.

## Changing the key

1. Create a new key pair.
2. Release an updater that trusts the new public key. During the transition it
   can trust both.
3. Switch the API to the new private key. The stored diff cache belongs to the
   old key and is discarded, archives are built again when they're requested.

You only have to do this if the private key may have leaked.

## Troubleshooting

| Error | Meaning |
|---|---|
| `signing_unavailable` (503) | No key configured and `ALLOW_UNSIGNED_ZIPS` not set. |
| `zip_not_allowed` (403) | The repository is only served through `*`. Add it to `ALLOWED_REPOS`. |
| `repository_not_pinned` (503) | Add the repository id: `ALLOWED_REPOS=owner/repo#id`. |
| `version_not_found` (404) | `from` or `to` is not a tag of the repository. |
| `versions_not_sequential` (422) | `to` is not a descendant of `from` (downgrade or diverged branches). |
| `zip_target_not_latest` (422) | `to` is not the latest version. |
| `unsupported_file_type` (422) | The update contains a symlink or submodule. |
| `unsupported_change` (422) | A file turns into a directory or the other way round. |
| `unsafe_path` (422) | A path isn't printable ASCII or not a plain relative path. |
| `zip_build_limit_reached` (503), `zip_client_limit_reached` (429) | Build limits reached. Existing archives are still handed out. |
| `diff_too_large` (422) | 300 or more changed files, do a full update instead. |

If the signature doesn't verify, the public key in the updater usually doesn't
match the API's key (compare it with the one logged at startup), or the
manifest wasn't written byte for byte (use `jq -j`, not `jq -r`).
