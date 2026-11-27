//! Delivery of a workspace's per-workspace bearer to `aoe serve --auth token`.
//!
//! UNDOCUMENTED UPSTREAM CONTRACT. aoe has no flag or env var to choose the
//! bearer it serves under (agent-of-empires issue 4953 is open). What it does
//! do, verified against aoe 1.18.0 through 1.26.0: when
//! `<HOME>/.config/agent-of-empires/serve.token`
//! holds exactly 64 hex characters, has mode 0600 and a fresh mtime, then
//! `aoe serve --auth token` adopts that value as its bearer instead of minting
//! one. Everything that depends on this behavior lives in this module, so a
//! change upstream (or the flag landing) is a one-file edit. The minimum aoe
//! version below is the oldest one the behavior was verified on.
//!
//! The token is never put in argv, labels, annotations, or `state.json`, and
//! never appears in CityHall's own logs, errors or tracing. aoe itself prints
//! the token URL to stdout, so workspace logs (`docker logs`, `kubectl logs`,
//! the process backend's `serve.log`) are secret-bearing. Only
//! [`fingerprint`] (a one-way, domain-separated digest) may be recorded on a
//! runtime object, for drift detection.

use super::{OrchestratorError, TelemetryPolicy};

/// Environment variable the container backends carry the token in. The wrapper
/// script reads it, writes `serve.token`, and unsets it before aoe starts.
pub const ENV_VAR: &str = "CITYHALL_DAEMON_TOKEN";

/// Where aoe looks for the token, relative to `$HOME`.
pub const SERVE_TOKEN_RELATIVE_PATH: &str = ".config/agent-of-empires/serve.token";

/// Oldest aoe `(major, minor)` verified to adopt `serve.token`.
const MIN_TOKEN_AUTH_VERSION: (u32, u32) = (1, 18);

/// Refuse to launch a release older than [`MIN_TOKEN_AUTH_VERSION`].
///
/// Never falls back to `--auth none`: an aoe that cannot adopt the token would
/// serve under a bearer CityHall does not know, so the proxy could not reach it.
/// A version whose first segment is not a number (`latest`, `main`) passes,
/// as those track the newest aoe. A numeric major with a non-numeric minor refuses.
pub fn require_token_auth_support(version: &str) -> Result<(), OrchestratorError> {
    let mut parts = version.trim_start_matches('v').split('.');
    // A first segment that is not a number is a moving tag (`latest`, `main`).
    if let Some(major) = parts.next().and_then(|p| p.parse::<u32>().ok()) {
        // A numeric major with a non-numeric minor (`1.17rc1.0`) is a malformed
        // release tag: refuse rather than guess it is new enough.
        let minor = match parts.next() {
            None => return Ok(()),
            Some(p) => p.parse::<u32>().ok(),
        };
        let Some(minor) = minor else {
            return Err(OrchestratorError::Runtime(format!(
                "aoe version {version} is not a valid major.minor release; \
                 aoe {}.{} or newer is required",
                MIN_TOKEN_AUTH_VERSION.0, MIN_TOKEN_AUTH_VERSION.1
            )));
        };
        if (major, minor) < MIN_TOKEN_AUTH_VERSION {
            return Err(OrchestratorError::Runtime(format!(
                "aoe {version} cannot serve under a per-workspace token; \
                 aoe {}.{} or newer is required",
                MIN_TOKEN_AUTH_VERSION.0, MIN_TOKEN_AUTH_VERSION.1
            )));
        }
    }
    Ok(())
}

/// The first 16 lowercase hex characters of a domain-separated SHA-256 of the
/// token. Safe to record as a label, annotation, or state field.
pub fn fingerprint(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"cityhall-daemon-token:");
    hasher.update(token.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The error for a spec that reaches a backend without a token.
pub fn missing_token() -> OrchestratorError {
    OrchestratorError::Runtime(
        "workspace has no daemon token; refusing to start aoe without --auth token".to_string(),
    )
}

/// Write `token` to `<home>/.config/agent-of-empires/serve.token` for the
/// process backend: mode 0600 from creation, content replaced and mtime
/// refreshed on every call. The error carries only the I/O failure, never the
/// token.
#[cfg(unix)]
pub fn write_serve_token(home: &std::path::Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let path = home.join(SERVE_TOKEN_RELATIVE_PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    // `mode` only applies on creation; an existing file may carry another one.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(token.as_bytes())?;
    file.sync_all()
}

/// The preamble both wrapper scripts share: write the token, then drop it from
/// the environment so nothing started afterwards (telemetry, aoe, agents)
/// inherits it. `set -eu` plus the explicit guard abort the start on any
/// failure, including an unset or empty token; `printf %s` never `echo`.
macro_rules! preamble {
    () => {
        r#"set -eu
umask 077
[ -n "${CITYHALL_DAEMON_TOKEN:-}" ] || { echo "CITYHALL_DAEMON_TOKEN is not set" >&2; exit 1; }
d="$HOME/.config/agent-of-empires"
mkdir -p "$d"
printf %s "$CITYHALL_DAEMON_TOKEN" > "$d/serve.token"
chmod 600 "$d/serve.token"
unset CITYHALL_DAEMON_TOKEN
"#
    };
}

/// Wrapper for every container workspace. A fixed constant with nothing
/// interpolated: the real command arrives through `"$0"` / `"$@"`, so every
/// argument stays one argv element and is never reparsed as shell source.
const SCRIPT: &str = concat!(preamble!(), r#"exec "$0" "$@""#);

/// [`SCRIPT`] for a force-on telemetry policy. `aoe telemetry enable` runs after
/// the token is unset, so that subprocess does not inherit it. `set -e` aborts
/// on an aoe too old for the subcommand rather than starting without the policy.
const SCRIPT_FORCE_ON: &str = concat!(preamble!(), "aoe telemetry enable\n", r#"exec "$0" "$@""#);

/// `command` as the container should run it: `sh -c <script> <command...>`.
/// Wraps rather than replaces, so the image's ENTRYPOINT still runs first (in
/// the reference image that is what relocates each agent's config onto the
/// volume). An image with no shell fails loudly instead of ignoring the token.
pub fn wrap_command(telemetry: TelemetryPolicy, command: Vec<String>) -> Vec<String> {
    let script = if telemetry == TelemetryPolicy::ForceOn {
        SCRIPT_FORCE_ON
    } else {
        SCRIPT
    };
    let mut wrapped = vec!["sh".to_string(), "-c".to_string(), script.to_string()];
    wrapped.extend(command);
    wrapped
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Serializes tests that write an executable and then exec it (ETXTBSY: a
    /// fork in another test thread inherits our write fd until its own exec).
    static FAKE_AOE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cityhall-daemon-token-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mode(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Runs the real wrapper in `sh`. `$0` is `sh -c <probe>`, which reports
    /// whether the token is visible and echoes its argv.
    fn run_wrapper(
        policy: TelemetryPolicy,
        home: &std::path::Path,
        token: Option<&str>,
        path: Option<&std::path::Path>,
    ) -> std::process::Output {
        let probe = r#"echo "tok=${CITYHALL_DAEMON_TOKEN-unset}"; echo "argv=$1|$2""#;
        let argv = wrap_command(
            policy,
            vec![
                "/bin/sh".into(),
                "-c".into(),
                probe.into(),
                "_".into(),
                "a b".into(),
                "$(touch pwned)".into(),
            ],
        );
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).env_clear().env("HOME", home);
        cmd.env(
            "PATH",
            match path {
                Some(p) => format!("{}:/usr/bin:/bin", p.display()),
                None => "/usr/bin:/bin".to_string(),
            },
        );
        if let Some(t) = token {
            cmd.env(ENV_VAR, t);
        }
        cmd.current_dir(home).output().unwrap()
    }

    #[test]
    fn fingerprint_is_16_hex_domain_separated_and_stable() {
        let fp = fingerprint(TOKEN);
        assert_eq!(fp.len(), 16);
        assert!(
            fp.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "{fp}"
        );
        assert_eq!(fp, fingerprint(TOKEN));
        assert_ne!(fp, fingerprint("other"));
        assert!(!TOKEN.contains(&fp));
    }

    #[test]
    fn version_gate_rejects_old_releases_and_passes_moving_tags() {
        for v in [
            "v1.18.0",
            "1.18.0",
            "v1.26.0",
            "v1.27.3",
            "v2.0.0",
            "v1.26.0-rc.1",
            "latest",
            "main",
            "abc1234",
            "1.22.0-rc.1",
            "v1.18.0-rc.1",
        ] {
            assert!(require_token_auth_support(v).is_ok(), "{v}");
        }
        for v in [
            "v1.17rc1.0",
            "1.x.0",
            "v1.-.0",
            "v1.17.9",
            "1.17.0",
            "v0.5.0",
        ] {
            let err = require_token_auth_support(v).unwrap_err().to_string();
            assert!(err.contains(v) && err.contains("1.18"), "{err}");
        }
    }

    #[test]
    fn write_serve_token_is_0600_overwrites_and_refreshes_mtime() {
        let home = scratch("write");
        write_serve_token(&home, TOKEN).unwrap();
        let path = home.join(SERVE_TOKEN_RELATIVE_PATH);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TOKEN);
        assert_eq!(mode(&path), 0o600);

        // An existing, laxer file is tightened and replaced; mtime moves on.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let other = "f".repeat(64);
        write_serve_token(&home, &other).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), other);
        assert_eq!(mode(&path), 0o600);
        assert!(std::fs::metadata(&path).unwrap().modified().unwrap() > old);
    }

    #[test]
    fn wrapper_writes_the_token_hides_it_and_keeps_argv_intact() {
        let home = scratch("wrap");
        let out = run_wrapper(TelemetryPolicy::UserChoice, &home, Some(TOKEN), None);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("tok=unset"), "{stdout}");
        assert!(stdout.contains("argv=a b|$(touch pwned)"), "{stdout}");
        assert!(!home.join("pwned").exists());
        let path = home.join(SERVE_TOKEN_RELATIVE_PATH);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TOKEN);
        assert_eq!(mode(&path), 0o600);
        assert!(!stdout.contains(TOKEN));
    }

    #[test]
    fn wrapper_aborts_without_a_token_or_with_an_unwritable_dir() {
        let home = scratch("abort");
        for token in [None, Some("")] {
            let out = run_wrapper(TelemetryPolicy::UserChoice, &home, token, None);
            assert!(!out.status.success());
            assert!(!String::from_utf8_lossy(&out.stdout).contains("tok="));
            assert!(!home.join(SERVE_TOKEN_RELATIVE_PATH).exists());
        }
        // A HOME under a regular file: mkdir fails even as root.
        let file = home.join("file");
        std::fs::write(&file, "x").unwrap();
        let bad_home = file.join("sub");
        let out = Command::new("sh")
            .arg("-c")
            .arg(SCRIPT)
            .arg("/bin/echo")
            .arg("started")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &bad_home)
            .env(ENV_VAR, TOKEN)
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("started"));
        assert!(!String::from_utf8_lossy(&out.stderr).contains(TOKEN));
    }

    #[test]
    fn force_on_runs_telemetry_enable_after_the_token_is_unset() {
        let _guard = FAKE_AOE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = scratch("force-on");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("aoe");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"$* tok=${CITYHALL_DAEMON_TOKEN-unset}\" > \"$HOME/telemetry.rec\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let out = run_wrapper(TelemetryPolicy::ForceOn, &home, Some(TOKEN), Some(&bin));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let rec = std::fs::read_to_string(home.join("telemetry.rec")).unwrap();
        assert_eq!(rec.trim(), "telemetry enable tok=unset");
        assert!(String::from_utf8_lossy(&out.stdout).contains("argv=a b|"));
        assert_eq!(
            std::fs::read_to_string(home.join(SERVE_TOKEN_RELATIVE_PATH)).unwrap(),
            TOKEN
        );

        // A failing `telemetry enable` aborts before the real command runs.
        std::fs::remove_file(&fake).unwrap();
        std::fs::write(&fake, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let out = run_wrapper(TelemetryPolicy::ForceOn, &home, Some(TOKEN), Some(&bin));
        assert!(!out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("argv="));
    }

    #[test]
    fn wrap_command_keeps_the_command_as_separate_argv_elements() {
        let command: Vec<String> = ["aoe", "serve", "--allowed-host", "ws.example.com"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for policy in [
            TelemetryPolicy::UserChoice,
            TelemetryPolicy::ForceOn,
            TelemetryPolicy::ForceOff,
        ] {
            let wrapped = wrap_command(policy, command.clone());
            assert_eq!(wrapped[..2], ["sh".to_string(), "-c".to_string()]);
            assert_eq!(wrapped[3..], command[..]);
            assert!(wrapped[2].contains(r#"exec "$0" "$@""#));
            assert_eq!(
                wrapped[2].contains("aoe telemetry enable"),
                policy == TelemetryPolicy::ForceOn
            );
        }
    }
}
