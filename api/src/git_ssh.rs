//! Validation for a user's git SSH key and its `known_hosts` (#52).
//!
//! Delivery is the config bundle's `[git]` table, alongside the HTTPS token: aoe
//! writes both into the workspace, so nothing here touches the filesystem. What
//! this module owns is refusing a value that would fail silently inside the
//! container, where the user cannot see why.
//!
//! Two things are refused rather than stored:
//!
//! - **A passphrase-protected key.** Nothing in a workspace can prompt for one,
//!   so it would turn every clone into a hang or an opaque failure. Detected by
//!   shape, not by trying to decrypt it: an unencrypted key is what the three
//!   private key encodings in use all say plainly.
//! - **A key with no `known_hosts`.** Shipping a key while trusting whatever
//!   answers on port 22 trades a credential problem for a machine-in-the-middle
//!   one. Requiring the host keys up front is what makes
//!   `StrictHostKeyChecking=yes` in the workspace mean something.
//!
//! Deliberately not validated: the key's algorithm, its length, and the key
//! types named in `known_hosts`. Those are OpenSSH's to accept or reject, and a
//! list of them here would rot as providers rotate and add algorithms.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

use crate::error::AppError;

/// Longest key accepted. An RSA-4096 key in OpenSSH format is about 3 KiB.
const MAX_KEY_LEN: usize = 16 * 1024;
/// Longest `known_hosts` accepted. GitHub's four host keys are under 1 KiB.
const MAX_KNOWN_HOSTS_LEN: usize = 16 * 1024;

/// The magic every OpenSSH private key starts with once base64-decoded.
const OPENSSH_MAGIC: &[u8] = b"openssh-key-v1\0";

/// Normalize and check a submitted private key.
///
/// Returns the key with CRLF line endings collapsed and exactly one trailing
/// newline, which is what OpenSSH expects of a key file and what a paste out of
/// a browser or a Windows editor does not always provide.
pub fn validate_key(key: &str) -> Result<String, AppError> {
    let key = normalize(key);
    if key.trim().is_empty() {
        return Err(AppError::BadRequest("a private key is required"));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(AppError::BadRequest("private key is too long"));
    }
    if key.contains('\0') {
        return Err(AppError::BadRequest(
            "a private key cannot contain a NUL byte",
        ));
    }

    let label = pem_label(&key).ok_or(AppError::BadRequest(
        "that does not look like a private key. Paste the whole file, including its BEGIN and END lines, and make sure it is the private half and not the .pub",
    ))?;
    if !key.contains(&format!("-----END {label}-----")) {
        return Err(AppError::BadRequest(
            "the key's END line is missing or does not match its BEGIN line",
        ));
    }

    if is_encrypted(&key, &label)? {
        return Err(AppError::BadRequest(
            "that key is protected by a passphrase, and nothing in a workspace can prompt for one. Store a key with no passphrase, kept for this purpose only, or strip it with `ssh-keygen -p`",
        ));
    }

    Ok(key)
}

/// Normalize and check submitted `known_hosts` lines.
///
/// Every meaningful line needs at least the three fields OpenSSH reads, a
/// pattern, a key type, and the key itself, because the common mistake is
/// pasting a fingerprint or a bare public key instead of `ssh-keyscan` output.
pub fn validate_known_hosts(known_hosts: &str) -> Result<String, AppError> {
    let known_hosts = normalize(known_hosts);
    if known_hosts.trim().is_empty() {
        return Err(AppError::BadRequest(
            "known_hosts is required, so the workspace can verify the host it connects to. Get it with `ssh-keyscan <host>` on a machine you trust",
        ));
    }
    if known_hosts.len() > MAX_KNOWN_HOSTS_LEN {
        return Err(AppError::BadRequest("known_hosts is too long"));
    }
    if known_hosts.contains('\0') {
        return Err(AppError::BadRequest(
            "known_hosts cannot contain a NUL byte",
        ));
    }

    let mut lines = 0;
    for line in known_hosts.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.split_whitespace().count() < 3 {
            return Err(AppError::BadRequest(
                "every known_hosts line needs a host pattern, a key type, and a key. Use the output of `ssh-keyscan <host>` as it comes, not a fingerprint",
            ));
        }
        lines += 1;
    }
    if lines == 0 {
        return Err(AppError::BadRequest(
            "known_hosts holds only comments, so no host could be verified",
        ));
    }

    Ok(known_hosts)
}

/// CRLF collapsed, trailing whitespace gone, exactly one final newline.
fn normalize(raw: &str) -> String {
    let body = raw.replace("\r\n", "\n").replace('\r', "\n");
    let body = body.trim_end();
    if body.is_empty() {
        return String::new();
    }
    format!("{body}\n")
}

/// The label out of the `-----BEGIN <label>-----` line, if the key has one and it
/// names a private key.
fn pem_label(key: &str) -> Option<String> {
    let line = key.lines().find(|l| l.starts_with("-----BEGIN "))?;
    let label = line
        .trim()
        .strip_prefix("-----BEGIN ")?
        .strip_suffix("-----")?
        .trim();
    // A public key or a certificate pasted by mistake would otherwise pass every
    // later check, and then fail inside the container.
    label.ends_with("PRIVATE KEY").then(|| label.to_string())
}

/// Whether the key needs a passphrase, by the encoding's own statement.
///
/// - PKCS#8 says so in the label (`ENCRYPTED PRIVATE KEY`).
/// - Traditional PEM (`RSA PRIVATE KEY` and friends) says so in a `Proc-Type:
///   4,ENCRYPTED` header before the body.
/// - OpenSSH's own format names its cipher inside the body, so that one has to
///   be decoded. `none` is the only unencrypted value.
fn is_encrypted(key: &str, label: &str) -> Result<bool, AppError> {
    if label.contains("ENCRYPTED") {
        return Ok(true);
    }
    if key.contains("Proc-Type:") && key.contains("ENCRYPTED") {
        return Ok(true);
    }
    if label != "OPENSSH PRIVATE KEY" {
        return Ok(false);
    }

    let body: String = key
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN "))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END "))
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .collect();
    let decoded = B64.decode(body.as_bytes()).map_err(|_| {
        AppError::BadRequest("the key's body is not valid base64, so it is damaged or truncated")
    })?;

    let rest = decoded.strip_prefix(OPENSSH_MAGIC).ok_or(AppError::BadRequest(
        "the key says it is an OpenSSH key but does not start like one, so it is damaged or truncated",
    ))?;
    // A uint32 length then that many bytes of cipher name.
    let len: [u8; 4] = rest
        .get(..4)
        .and_then(|b| b.try_into().ok())
        .ok_or(TRUNCATED)?;
    let len = u32::from_be_bytes(len) as usize;
    let name = rest.get(4..4 + len).ok_or(TRUNCATED)?;
    Ok(name != b"none")
}

const TRUNCATED: AppError =
    AppError::BadRequest("the key is truncated: its cipher name does not fit in the body");

#[cfg(test)]
mod tests {
    use super::*;

    /// An `ssh-keygen -t ed25519 -N ""` key, so `ciphername` is `none`.
    const UNENCRYPTED_OPENSSH: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZWQyNTUxOQAAACBH0mVDQeVoLHKUZ1BsvBaBBpydTZLMlomjLnKMHTtRVAAAAJgD/OtiA/zrYgAAAAtzc2gtZWQyNTUxOQAAACBH0mVDQeVoLHKUZ1BsvBaBBpydTZLMlomjLnKMHTtRVAAAAEAKN2Zn1AmoRnLKzOxUmDCLtRUZOe9DYc7oCzhCyDLpEEfSZUNB5WgscpRnUGy8FoEGnJ1NksyWiaMucowdO1FUAAAAAAECAwQF\n-----END OPENSSH PRIVATE KEY-----";

    /// The same shape with `ciphername` = `aes256-ctr` and a kdf, which is what
    /// `ssh-keygen -N somepassphrase` writes.
    fn encrypted_openssh() -> String {
        let mut body = Vec::new();
        body.extend_from_slice(OPENSSH_MAGIC);
        body.extend_from_slice(&(10u32).to_be_bytes());
        body.extend_from_slice(b"aes256-ctr");
        body.extend_from_slice(&(6u32).to_be_bytes());
        body.extend_from_slice(b"bcrypt");
        format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            B64.encode(&body)
        )
    }

    #[test]
    fn an_unencrypted_key_is_accepted_and_normalized() {
        let stored = validate_key(&UNENCRYPTED_OPENSSH.replace('\n', "\r\n")).unwrap();
        assert!(!stored.contains('\r'));
        // OpenSSH wants a key file that ends with a newline, and exactly one.
        assert!(stored.ends_with("PRIVATE KEY-----\n"));
        assert!(!stored.ends_with("\n\n"));
    }

    #[test]
    fn a_passphrase_protected_key_is_refused() {
        // OpenSSH format: the cipher name inside the body is what gives it away.
        let err = validate_key(&encrypted_openssh()).unwrap_err();
        assert!(format!("{err:?}").contains("passphrase"));
        // Traditional PEM says so in a header.
        assert!(validate_key(
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00\n\nAAAA\n-----END RSA PRIVATE KEY-----"
        )
        .is_err());
        // PKCS#8 says so in the label.
        assert!(validate_key(
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----"
        )
        .is_err());
    }

    #[test]
    fn a_public_key_is_refused_rather_than_stored() {
        // The commonest paste mistake, and one that would otherwise only fail
        // inside the container.
        assert!(validate_key("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 me@host").is_err());
        assert!(
            validate_key("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----").is_err()
        );
    }

    #[test]
    fn a_damaged_key_is_named_rather_than_stored() {
        assert!(validate_key("").is_err());
        // BEGIN with no matching END: a paste that stopped short.
        assert!(validate_key("-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n").is_err());
        // Body that is not base64 at all.
        assert!(validate_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n!!!!\n-----END OPENSSH PRIVATE KEY-----"
        )
        .is_err());
        // Valid base64, but not an OpenSSH key.
        assert!(validate_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----"
        )
        .is_err());
        assert!(validate_key(&"x".repeat(MAX_KEY_LEN + 1)).is_err());
    }

    #[test]
    fn known_hosts_must_be_usable_for_verification() {
        let good = "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
        assert_eq!(
            validate_known_hosts(&format!("{good}\r\n")).unwrap(),
            format!("{good}\n")
        );
        // Comments and blanks are ssh-keyscan's own output, so they stay.
        assert!(validate_known_hosts(&format!("# github.com:22 SSH-2.0\n\n{good}\n")).is_ok());

        assert!(validate_known_hosts("").is_err());
        assert!(validate_known_hosts("   \n\n").is_err());
        // Only comments verifies nothing.
        assert!(validate_known_hosts("# github.com:22 SSH-2.0\n").is_err());
        // A fingerprint rather than a key.
        assert!(
            validate_known_hosts("SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU").is_err()
        );
        // A public key with no host pattern in front of it.
        assert!(validate_known_hosts("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5").is_err());
    }
}
