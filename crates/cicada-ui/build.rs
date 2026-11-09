//! Builds the console (web/) with Vite into `$OUT_DIR/web`, which `src/lib.rs` embeds. Needs Node.js and npm. Set `CICADA_UI_SKIP_BUILD` to skip the build (an empty console is embedded then).

use std::path::Path;
use std::process::Command;
use std::{env, fs};

fn main() {
    for path in ["src", "public", "index.html", "vite.config.ts", "tsconfig.json", "package.json", "package-lock.json"]
    {
        println!("cargo:rerun-if-changed=web/{path}");
    }
    println!("cargo:rerun-if-env-changed=CICADA_UI_SKIP_BUILD");
    let web = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("set by cargo")).join("web");
    let out = Path::new(&env::var("OUT_DIR").expect("set by cargo")).join("web");
    fs::create_dir_all(&out).expect("create the console's output folder");
    if env::var_os("CICADA_UI_SKIP_BUILD").is_some() {
        println!("cargo:warning=CICADA_UI_SKIP_BUILD is set: the console is not built");
        return;
    }
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if !web.join("node_modules").exists() {
        run(Command::new(npm).args(["ci", "--no-audit", "--no-fund"]).current_dir(&web), "npm ci");
    }
    run(Command::new(npm).args(["run", "build"]).env("CICADA_UI_OUT_DIR", &out).current_dir(&web), "npm run build");
}

fn run(command: &mut Command, what: &str) {
    let status = command.status().unwrap_or_else(|e| {
        panic!("{what} failed to start ({e}); the console needs Node.js 22+ and npm, or set CICADA_UI_SKIP_BUILD")
    });
    assert!(status.success(), "{what} failed with {status}");
}
