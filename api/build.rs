//! Builds the web frontend into `web/dist` so a plain `cargo build` / `cargo
//! run` produces a binary that can serve the SPA.
//!
//! Escape hatch: set `SKIP_FRONTEND_BUILD=1` when the bundle is produced
//! elsewhere (docker multi-stage build, CI artifact, backend-only iteration).

use std::path::Path;
use std::process::Command;

fn main() {
    // The crate lives in `api/`; the frontend is its sibling `web/`.
    let web = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("api crate has a parent workspace dir")
        .join("web");

    // Only rerun when frontend inputs change, not on every backend edit.
    for input in [
        "src",
        "public",
        "index.html",
        "package.json",
        "package-lock.json",
        "vite.config.ts",
        "tsconfig.json",
    ] {
        println!("cargo:rerun-if-changed={}", web.join(input).display());
    }
    println!("cargo:rerun-if-env-changed=SKIP_FRONTEND_BUILD");

    if std::env::var_os("SKIP_FRONTEND_BUILD").is_some() {
        println!("cargo:warning=SKIP_FRONTEND_BUILD set; not building web/dist");
        return;
    }

    let dist_exists = web.join("dist").join("index.html").exists();

    // `npm` is `npm.cmd` on Windows.
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if Command::new(npm).arg("--version").output().is_err() {
        if dist_exists {
            println!("cargo:warning=npm not found; serving existing web/dist");
            return;
        }
        panic!(
            "npm not found and web/dist is missing. Install Node.js, or build the \
             frontend elsewhere and set SKIP_FRONTEND_BUILD=1."
        );
    }

    if !web.join("node_modules").exists() {
        run(
            Command::new(npm).arg("install").current_dir(&web),
            "npm install",
        );
    }
    run(
        Command::new(npm).args(["run", "build"]).current_dir(&web),
        "npm run build",
    );
}

fn run(cmd: &mut Command, label: &str) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `{label}`: {e}"));
    assert!(status.success(), "`{label}` failed with {status}");
}
