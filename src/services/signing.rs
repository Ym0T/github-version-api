//! Ed25519 signing of update archives.
//!
//! * `manifest_signature`: signs a text manifest that binds the archive hash
//!   to the repository, both versions and commits, and the files the update
//!   deletes. A client that checks these fields can't be tricked into
//!   installing an archive built for another repository or version pair.
//! * `signature` (legacy, opt-in): signs only the raw 32-byte SHA-256 of the
//!   archive, for updaters that only check the archive hash. It is not bound
//!   to a request and should be disabled once all clients verify manifests.
//!
//! The manifest is always longer than 32 bytes, so a legacy signature can
//! never be mistaken for a manifest signature or vice versa.

use std::fmt;
use std::fmt::Write as _;

use anyhow::anyhow;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const MANIFEST_FORMAT: &str = "version-api-update-manifest/1";

pub struct Signer {
    key: SigningKey,
    public_key_b64: String,
}

impl fmt::Debug for Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signer")
            .field("public_key", &self.public_key_b64)
            .finish_non_exhaustive()
    }
}

impl Signer {
    /// Creates a signer from a base64-encoded 32-byte Ed25519 seed.
    pub fn from_base64(encoded: &str) -> anyhow::Result<Self> {
        let bytes = Zeroizing::new(
            BASE64
                .decode(encoded.trim())
                .map_err(|_| anyhow!("the signing key is not valid base64"))?,
        );
        let seed: Zeroizing<[u8; 32]> =
            Zeroizing::new(bytes.as_slice().try_into().map_err(|_| {
                anyhow!(
                    "the signing key must be exactly 32 bytes, got {}",
                    bytes.len()
                )
            })?);
        let key = SigningKey::from_bytes(&seed);
        let public_key_b64 = BASE64.encode(key.verifying_key().as_bytes());
        Ok(Self {
            key,
            public_key_b64,
        })
    }

    pub fn public_key_b64(&self) -> &str {
        &self.public_key_b64
    }

    /// Signs `message` and returns the base64-encoded signature.
    pub fn sign(&self, message: &[u8]) -> String {
        BASE64.encode(self.key.sign(message).to_bytes())
    }

    /// A secondary key for `context`, derived from the signing seed
    /// (domain-separated SHA-256). Never usable as a signing key.
    pub fn derive_key(&self, context: &str) -> Zeroizing<[u8; 32]> {
        let mut hasher = Sha256::new();
        hasher.update(b"version-api key derivation\0");
        hasher.update(context.as_bytes());
        hasher.update([0]);
        hasher.update(self.key.as_bytes());
        Zeroizing::new(hasher.finalize().into())
    }
}

/// The fields bound by a manifest signature.
pub struct ManifestFields<'a> {
    pub repository: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub from_commit: &'a str,
    pub to_commit: &'a str,
    pub sha256_hex: &'a str,
    pub size: u64,
    pub created_at: i64,
    /// Validated, sorted paths the client must delete.
    pub delete: &'a [String],
}

/// Renders the manifest: one `key=value` per line, each terminated by `\n`,
/// with one `delete=` line per file to delete. Values are validated
/// identifiers, hashes, numbers or validated paths, so they never contain
/// line breaks; keys never contain `=`.
pub fn render_manifest(fields: &ManifestFields<'_>) -> String {
    let mut out = format!(
        "format={MANIFEST_FORMAT}\nrepository={}\nfrom={}\nto={}\nfrom_commit={}\nto_commit={}\nsha256={}\nsize={}\ncreated_at={}\n",
        fields.repository,
        fields.from,
        fields.to,
        fields.from_commit,
        fields.to_commit,
        fields.sha256_hex,
        fields.size,
        fields.created_at
    );
    for path in fields.delete {
        let _ = writeln!(out, "delete={path}");
    }
    out
}

/// Generates a new key pair: (base64 private seed, base64 public key).
pub fn generate_keypair() -> anyhow::Result<(Zeroizing<String>, String)> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut())
        .map_err(|e| anyhow!("cannot get randomness from the OS: {e}"))?;
    let key = SigningKey::from_bytes(&seed);
    Ok((
        Zeroizing::new(BASE64.encode(seed.as_ref())),
        BASE64.encode(key.verifying_key().as_bytes()),
    ))
}

#[cfg(test)]
pub fn verify(message: &[u8], signature_b64: &str, public_key_b64: &str) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};
    let Ok(signature) = BASE64.decode(signature_b64) else {
        return false;
    };
    let Ok(signature): Result<[u8; 64], _> = signature.try_into() else {
        return false;
    };
    let public_key: [u8; 32] = BASE64.decode(public_key_b64).unwrap().try_into().unwrap();
    VerifyingKey::from_bytes(&public_key)
        .unwrap()
        .verify_strict(message, &Signature::from_bytes(&signature))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fields<'a>(sha: &'a str, delete: &'a [String]) -> ManifestFields<'a> {
        ManifestFields {
            repository: "Ym0T/pterodactyl-nginx-egg",
            from: "v1.0.0",
            to: "v1.1.0",
            from_commit: "1111111111111111111111111111111111111111",
            to_commit: "2222222222222222222222222222222222222222",
            sha256_hex: sha,
            size: 7,
            created_at: 1_700_000_000,
            delete,
        }
    }

    #[test]
    fn signs_and_verifies() {
        let (private_key, public_key) = generate_keypair().unwrap();
        let signer = Signer::from_base64(&private_key).unwrap();
        assert_eq!(signer.public_key_b64(), public_key);

        let digest: [u8; 32] = Sha256::digest(b"archive").into();
        let legacy = signer.sign(&digest);
        assert!(verify(&digest, &legacy, &public_key));

        let hash = hex::encode(digest);
        let delete = vec!["old.txt".to_string()];
        let manifest = render_manifest(&fields(&hash, &delete));
        let manifest_signature = signer.sign(manifest.as_bytes());
        assert!(verify(
            manifest.as_bytes(),
            &manifest_signature,
            &public_key
        ));

        // Signatures are not interchangeable and don't survive tampering.
        assert!(!verify(manifest.as_bytes(), &legacy, &public_key));
        assert!(!verify(&digest, &manifest_signature, &public_key));
        for tampered in [
            manifest.replace("to=v1.1.0", "to=v9.9.9"),
            manifest.replace("delete=old.txt\n", ""),
            format!("{manifest}delete=start.sh\n"),
        ] {
            assert!(!verify(
                tampered.as_bytes(),
                &manifest_signature,
                &public_key
            ));
        }
    }

    #[test]
    fn manifest_format_is_stable() {
        let delete = vec!["a/b.txt".to_string(), "c.txt".to_string()];
        assert_eq!(
            render_manifest(&fields("ab", &delete)),
            "format=version-api-update-manifest/1\n\
             repository=Ym0T/pterodactyl-nginx-egg\n\
             from=v1.0.0\n\
             to=v1.1.0\n\
             from_commit=1111111111111111111111111111111111111111\n\
             to_commit=2222222222222222222222222222222222222222\n\
             sha256=ab\n\
             size=7\n\
             created_at=1700000000\n\
             delete=a/b.txt\n\
             delete=c.txt\n"
        );
    }

    #[test]
    fn rejects_invalid_keys() {
        assert!(Signer::from_base64("not base64!").is_err());
        assert!(Signer::from_base64(&BASE64.encode([0u8; 31])).is_err());
        assert!(Signer::from_base64(&BASE64.encode([0u8; 64])).is_err());
    }

    #[test]
    fn debug_output_does_not_contain_the_private_key() {
        let (private_key, _) = generate_keypair().unwrap();
        let signer = Signer::from_base64(&private_key).unwrap();
        assert!(!format!("{signer:?}").contains(private_key.as_str()));
    }
}
