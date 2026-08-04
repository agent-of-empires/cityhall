//! Symmetric encryption for secrets stored in the database: the SMTP password,
//! the OIDC client secret, git credentials (HTTPS token and SSH key), and agent
//! credentials. Also the transient admin workspace-access token, which is not
//! stored anywhere.
//!
//! AES-256-GCM under `CITYHALL_SECRET_KEY` (base64, 32 bytes), plus any number of
//! decrypt-only keys in `CITYHALL_SECRET_KEY_PREVIOUS` so the current key can be
//! rotated (see `crate::secrets`).
//!
//! # The envelope
//!
//! A stored value is `v2.` followed by base64 of `nonce (12 bytes) || ciphertext`,
//! and is authenticated against an [`Aad`] naming its purpose and its owning row.
//! The prefix makes the format versioned: base64's alphabet has no `.`, so a
//! prefixed value can never be read as an unprefixed one, or the reverse.
//!
//! Values written before the envelope existed are bare base64 with no associated
//! data. They stay readable, because refusing them would brick every deployment
//! that upgrades. They are never written again, and `cityhall secrets rotate`
//! upgrades them. Until a row is rotated it carries nothing to bind to, so moving
//! it between rows still succeeds; that is the whole reason rotation reports a
//! legacy count and startup warns about one.
//!
//! # Why the key is not identified in the envelope
//!
//! Decryption tries the current key, then each previous key, and stops at the
//! first that authenticates; a wrong key is a clean failure because GCM checks a
//! tag. Which key matched is reported as a [`Provenance`], which is everything a
//! stored key id would have been for. Storing one instead would need either an id
//! allocation scheme or a key fingerprint, and a fingerprint in the column groups
//! rows by key epoch for anyone holding a database backup.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::error::AppError;

const KEY_ENV: &str = "CITYHALL_SECRET_KEY";
const PREVIOUS_KEY_ENV: &str = "CITYHALL_SECRET_KEY_PREVIOUS";
const NONCE_LEN: usize = 12;
/// GCM's authentication tag, appended to the ciphertext. Only used to reject a
/// blob too short to be one, before asking the cipher about it.
const TAG_LEN: usize = 16;
/// Marks the versioned envelope. Always `STANDARD` base64 after this: no base64
/// alphabet contains `.`, so the prefix is unambiguous, but a future path that
/// switched alphabets mid-format would still be a mistake.
const V2_PREFIX: &str = "v2.";

/// What a secret is bound to. Authenticated with the ciphertext, so a value
/// moved to another row, or to another store, no longer decrypts.
///
/// A sealed enum rather than a string, because a string parameter is advisory: a
/// call site could pass a literal, or spell an owner interpolation almost
/// correctly, and reintroduce exactly the swappability this exists to stop.
/// Naming a new purpose means adding a variant here, and the byte encoding lives
/// in one place, which matters because two of the stores encrypt and decrypt in
/// different modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Aad {
    /// `smtp_settings`, a singleton row.
    SmtpPassword,
    /// `oidc_settings`, a singleton row.
    OidcClientSecret,
    GitCredential {
        user_id: i32,
    },
    /// `git_ssh_keys`. Distinct from [`Aad::GitCredential`] even though both are
    /// keyed by user, so the HTTPS token and the SSH key cannot be swapped for
    /// each other.
    GitSshKey {
        user_id: i32,
    },
    AgentCredential {
        user_id: i32,
        env_var: String,
    },
    /// Not a stored secret: the admin access token in `crate::proxy`.
    WorkspaceAccessToken {
        purpose: &'static str,
    },
}

impl Aad {
    /// The authenticated bytes. `:` separates the fields, which is unambiguous
    /// because the only interpolated name comes from the closed credential
    /// catalog and cannot contain one (asserted by a test over `CATALOG`).
    fn bytes(&self) -> Vec<u8> {
        match self {
            // The trailing id is the singleton row these live in, so the binding
            // says which row and not merely which store.
            Self::SmtpPassword => b"smtp-password:1".to_vec(),
            Self::OidcClientSecret => b"oidc-client-secret:1".to_vec(),
            Self::GitCredential { user_id } => format!("git-credential:{user_id}").into_bytes(),
            Self::GitSshKey { user_id } => format!("git-ssh-key:{user_id}").into_bytes(),
            Self::AgentCredential { user_id, env_var } => {
                format!("agent-credential:{user_id}:{env_var}").into_bytes()
            }
            Self::WorkspaceAccessToken { purpose } => {
                format!("ws-access-token:{purpose}").into_bytes()
            }
        }
    }
}

/// Which key opened a value, so rotation can tell what still needs rewriting and
/// an operator can tell whether dropping the previous keys is safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// Predates the envelope: no associated data, so not bound to its row.
    Legacy,
    CurrentKey,
    /// Readable, but only until `CITYHALL_SECRET_KEY_PREVIOUS` goes away.
    PreviousKey,
}

/// The keys in trial order: current first, then the decrypt-only ones.
struct Keyring {
    current: Aes256Gcm,
    previous: Vec<Aes256Gcm>,
}

fn parse_key(
    raw: &str,
    invalid: &'static str,
    wrong_len: &'static str,
) -> Result<Aes256Gcm, AppError> {
    let bytes = B64
        .decode(raw.trim())
        .map_err(|_| AppError::Internal(invalid))?;
    let key =
        Key::<Aes256Gcm>::try_from(bytes.as_slice()).map_err(|_| AppError::Internal(wrong_len))?;
    Ok(Aes256Gcm::new(&key))
}

/// Load and validate the current key.
fn current_key() -> Result<Aes256Gcm, AppError> {
    let raw = std::env::var(KEY_ENV).map_err(|_| {
        AppError::BadRequest("CITYHALL_SECRET_KEY is not set; it is required to store secrets")
    })?;
    parse_key(
        &raw,
        "CITYHALL_SECRET_KEY is not valid base64",
        "CITYHALL_SECRET_KEY must decode to 32 bytes",
    )
}

/// Load the whole ring. Read from the environment on every call rather than
/// cached: this is a base64 decode on paths that run at an SMTP send, an OIDC
/// handshake, a workspace boot, and a token verify, and caching it would mean
/// reworking how every test in the crate supplies a key.
fn keyring() -> Result<Keyring, AppError> {
    let current = current_key()?;
    let previous = match std::env::var(PREVIOUS_KEY_ENV) {
        Ok(raw) => raw
            .split(',')
            .map(str::trim)
            // A trailing comma, or a value left as `,`, is a formatting artifact
            // rather than a key. A non-empty entry that will not parse is an
            // error below, because skipping it is how a rotation looks like it
            // worked and loses data.
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                parse_key(
                    entry,
                    "CITYHALL_SECRET_KEY_PREVIOUS has an entry that is not valid base64",
                    "CITYHALL_SECRET_KEY_PREVIOUS has an entry that does not decode to 32 bytes",
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
        Err(_) => Vec::new(),
    };
    Ok(Keyring { current, previous })
}

/// Parse the whole ring, so a malformed `CITYHALL_SECRET_KEY_PREVIOUS` fails at
/// startup instead of inside an unrelated request much later.
///
/// No key at all stays legal: a deployment that stores no secrets runs without
/// one, and every write path already reports that it is required.
pub fn validate_keyring() -> Result<(), AppError> {
    if std::env::var(KEY_ENV).is_err() {
        return Ok(());
    }
    keyring().map(|_| ())
}

/// Whether a usable secret key is configured. Used to gate flows that need to
/// read or write encrypted secrets before attempting them.
pub fn key_available() -> bool {
    current_key().is_ok()
}

/// Whether a stored value predates the versioned envelope, and so is not bound
/// to the row holding it. A prefix test: needs no key and cannot fail.
pub fn is_legacy(encoded: &str) -> bool {
    !encoded.starts_with(V2_PREFIX)
}

/// A decrypted secret, wrapped so it cannot be printed by accident.
///
/// Plaintext secrets travel through `WorkspaceSpec`, which derives `Debug` and
/// is a natural thing for a future `tracing` call to log. A bare `String` there
/// would make that a credential leak; this type renders as `[REDACTED]` and has
/// no `Display`, so reaching the value takes an explicit [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The plaintext. Named to make a review notice every call site.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Encrypt under the current key, bound to `aad`. Always writes the versioned
/// envelope.
pub fn encrypt(plaintext: &str, aad: &Aad) -> Result<String, AppError> {
    let cipher = current_key()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes).map_err(|_| AppError::Internal("secure RNG failure"))?;
    let nonce: Nonce<_> = nonce_bytes.into();
    let aad_bytes = aad.bytes();
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad_bytes,
            },
        )
        .map_err(|_| AppError::Internal("failed to encrypt secret"))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(format!("{V2_PREFIX}{}", B64.encode(blob)))
}

/// Decrypt a value that must belong to `aad`.
pub fn decrypt(encoded: &str, aad: &Aad) -> Result<String, AppError> {
    open(encoded, aad).map(|(plaintext, _)| plaintext)
}

/// Decrypt, and report which key opened it and whether it is still legacy.
///
/// Only rotation and status reporting need the provenance; everything else calls
/// [`decrypt`].
pub fn open(encoded: &str, aad: &Aad) -> Result<(String, Provenance), AppError> {
    let ring = keyring()?;

    let legacy = is_legacy(encoded);
    let body = encoded.strip_prefix(V2_PREFIX).unwrap_or(encoded);
    let blob = B64
        .decode(body)
        .map_err(|_| AppError::Internal("stored secret is not valid base64"))?;
    if blob.len() < NONCE_LEN + TAG_LEN {
        return Err(AppError::Internal("stored secret is malformed"));
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let nonce_arr: [u8; NONCE_LEN] = nonce_bytes
        .try_into()
        .map_err(|_| AppError::Internal("stored secret is malformed"))?;
    let nonce: Nonce<_> = nonce_arr.into();

    // A legacy value was written before there was anything to bind to, so it is
    // authenticated against empty associated data whatever the caller asked for.
    let owned_aad = aad.bytes();
    let aad_bytes: &[u8] = if legacy { &[] } else { &owned_aad };

    for (cipher, key_provenance) in std::iter::once((&ring.current, Provenance::CurrentKey))
        .chain(ring.previous.iter().map(|k| (k, Provenance::PreviousKey)))
    {
        let payload = Payload {
            msg: ciphertext,
            aad: aad_bytes,
        };
        if let Ok(plaintext) = cipher.decrypt(&nonce, payload) {
            // Legacy wins over which key opened it: rotation has to rewrite the
            // row either way, to bind it.
            let provenance = if legacy {
                Provenance::Legacy
            } else {
                key_provenance
            };
            return String::from_utf8(plaintext)
                .map(|plaintext| (plaintext, provenance))
                .map_err(|_| AppError::Internal("decrypted secret is not UTF-8"));
        }
    }

    Err(AppError::Internal(
        "failed to decrypt secret (wrong key, or the value was moved between rows?)",
    ))
}

/// Serializes tests that mutate `CITYHALL_SECRET_KEY`.
///
/// The key is read from the process environment, so a test that sets it and one
/// that clears it will otherwise see each other's value when `cargo test` runs
/// them on different threads. Any test anywhere in the crate that touches the
/// variable takes this lock for its whole body.
#[cfg(test)]
pub(crate) static KEY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The lock, ignoring poisoning: a panicking test leaves the env var in an
/// unknown state, but every holder sets what it needs before reading, so the
/// next test is unaffected and should run rather than fail on the poison.
#[cfg(test)]
pub(crate) fn lock_key_env() -> std::sync::MutexGuard<'static, ()> {
    KEY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Encode a value the way CityHall did before the envelope existed: bare base64
/// of `nonce || ciphertext`, with no associated data.
///
/// Only exists so tests and `crate::secrets`' tests can produce a row that a
/// pre-upgrade CityHall would have written; nothing in the crate writes this
/// format any more.
#[cfg(test)]
pub(crate) fn encrypt_legacy(plaintext: &str) -> Result<String, AppError> {
    let cipher = current_key()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes).map_err(|_| AppError::Internal("secure RNG failure"))?;
    let nonce: Nonce<_> = nonce_bytes.into();
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| AppError::Internal("failed to encrypt secret"))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ciphertext);
    Ok(B64.encode(blob))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: [u8; 32] = [7u8; 32];
    const KEY_B: [u8; 32] = [9u8; 32];

    fn agent(user_id: i32) -> Aad {
        Aad::AgentCredential {
            user_id,
            env_var: "ANTHROPIC_API_KEY".to_string(),
        }
    }

    // One test because these all mutate the same process-wide env vars; running
    // them as separate parallel tests would race on the key ring.
    #[test]
    fn envelope_keyring_and_binding() {
        let _guard = lock_key_env();

        // Missing key: no encryption possible.
        std::env::remove_var(KEY_ENV);
        std::env::remove_var(PREVIOUS_KEY_ENV);
        assert!(!key_available());
        assert!(encrypt("x", &Aad::SmtpPassword).is_err());
        // A ring with no current key is not an error: a deployment storing no
        // secrets runs without one.
        assert!(validate_keyring().is_ok());

        std::env::set_var(KEY_ENV, B64.encode(KEY_A));
        assert!(key_available());

        // Round trip, versioned, and nonces are not reused.
        let secret = "hunter2 \u{1f510} unicode";
        let blob = encrypt(secret, &Aad::SmtpPassword).unwrap();
        assert!(blob.starts_with(V2_PREFIX), "{blob}");
        assert!(!is_legacy(&blob));
        assert_eq!(
            open(&blob, &Aad::SmtpPassword).unwrap(),
            (secret.to_string(), Provenance::CurrentKey)
        );
        assert_ne!(
            encrypt("same", &Aad::SmtpPassword).unwrap(),
            encrypt("same", &Aad::SmtpPassword).unwrap()
        );

        // The point of the issue: the same ciphertext under another row's
        // binding, or another store's, does not decrypt.
        let mine = encrypt("sk-mine", &agent(1)).unwrap();
        assert_eq!(decrypt(&mine, &agent(1)).unwrap(), "sk-mine");
        assert!(decrypt(&mine, &agent(2)).is_err());
        assert!(decrypt(&mine, &Aad::SmtpPassword).is_err());
        assert!(decrypt(&blob, &Aad::OidcClientSecret).is_err());

        // A legacy value still opens, ignores the binding it is handed, and says
        // so, which is what lets rotation find it.
        let legacy = encrypt_legacy("old-password").unwrap();
        assert!(is_legacy(&legacy));
        assert_eq!(
            open(&legacy, &Aad::SmtpPassword).unwrap(),
            ("old-password".to_string(), Provenance::Legacy)
        );
        // The binding is not enforced for it, which is why rotating matters.
        assert_eq!(open(&legacy, &agent(99)).unwrap().1, Provenance::Legacy);

        // Rotation: written under A, read while A is a previous key, gone once A
        // is dropped.
        std::env::set_var(KEY_ENV, B64.encode(KEY_B));
        assert!(decrypt(&mine, &agent(1)).is_err());
        std::env::set_var(PREVIOUS_KEY_ENV, B64.encode(KEY_A));
        assert_eq!(
            open(&mine, &agent(1)).unwrap(),
            ("sk-mine".to_string(), Provenance::PreviousKey)
        );
        // A value written now is under B, so it needs nothing previous.
        let fresh = encrypt("sk-fresh", &agent(1)).unwrap();
        assert_eq!(open(&fresh, &agent(1)).unwrap().1, Provenance::CurrentKey);
        std::env::remove_var(PREVIOUS_KEY_ENV);
        assert!(decrypt(&mine, &agent(1)).is_err());
        assert_eq!(decrypt(&fresh, &agent(1)).unwrap(), "sk-fresh");

        // Ring parsing: whitespace and empty entries are formatting, a malformed
        // entry is an error rather than a silently smaller ring.
        std::env::set_var(PREVIOUS_KEY_ENV, format!(" {} , ,", B64.encode(KEY_A)));
        assert!(validate_keyring().is_ok());
        assert_eq!(open(&mine, &agent(1)).unwrap().1, Provenance::PreviousKey);
        std::env::set_var(PREVIOUS_KEY_ENV, "not base64!!");
        assert!(validate_keyring().is_err());
        assert!(decrypt(&fresh, &agent(1)).is_err());
        std::env::set_var(PREVIOUS_KEY_ENV, B64.encode([1u8; 16]));
        assert!(validate_keyring().is_err());

        std::env::remove_var(KEY_ENV);
        std::env::remove_var(PREVIOUS_KEY_ENV);
    }

    #[test]
    fn malformed_envelopes_are_rejected_without_panicking() {
        let _guard = lock_key_env();
        std::env::set_var(KEY_ENV, B64.encode(KEY_A));
        std::env::remove_var(PREVIOUS_KEY_ENV);

        for bad in [
            "",
            V2_PREFIX,
            "v2.!!!not base64",
            // Valid base64, but too short to hold a nonce and a tag.
            &B64.encode([0u8; NONCE_LEN + TAG_LEN - 1]),
            &format!("{V2_PREFIX}{}", B64.encode([0u8; NONCE_LEN])),
        ] {
            assert!(decrypt(bad, &Aad::SmtpPassword).is_err(), "{bad:?}");
        }

        std::env::remove_var(KEY_ENV);
    }

    /// Every store, and every owner within a store, must authenticate against
    /// different bytes, or a value could be moved between them.
    #[test]
    fn bindings_are_distinct_per_store_and_per_owner() {
        let all = [
            Aad::SmtpPassword,
            Aad::OidcClientSecret,
            Aad::GitCredential { user_id: 1 },
            Aad::GitCredential { user_id: 2 },
            // Same user, other half of their git credentials: an HTTPS token
            // must not decrypt as that user's SSH key.
            Aad::GitSshKey { user_id: 1 },
            Aad::GitSshKey { user_id: 2 },
            agent(1),
            agent(2),
            Aad::AgentCredential {
                user_id: 1,
                env_var: "OPENAI_API_KEY".to_string(),
            },
            Aad::WorkspaceAccessToken {
                purpose: "ws-exchange",
            },
            Aad::WorkspaceAccessToken {
                purpose: "ws-session",
            },
        ];
        let mut seen: Vec<Vec<u8>> = all.iter().map(Aad::bytes).collect();
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count);
    }

    #[test]
    fn secret_never_prints_its_value() {
        let s = Secret::new("sk-do-not-log-me".to_string());
        assert_eq!(format!("{s:?}"), "[REDACTED]");
        assert!(!format!("{:?}", vec![("K".to_string(), s.clone())]).contains("sk-"));
        assert_eq!(s.expose(), "sk-do-not-log-me");
    }
}
