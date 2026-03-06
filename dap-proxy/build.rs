use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/index");

    if let Some(commit) = git_output(&["rev-parse", "--short", "HEAD"]) {
        println!("cargo:rustc-env=PROXY_GIT_COMMIT={commit}");
    }

    if let Some(tag) = git_output(&[
        "describe",
        "--tags",
        "--exact-match",
        "--match",
        "dap-proxy-v*",
    ]) {
        println!("cargo:rustc-env=PROXY_GIT_TAG={tag}");
    }

    if git_is_dirty() {
        println!("cargo:rustc-env=PROXY_GIT_DIRTY=1");
    }
}

fn git_output(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn git_is_dirty() -> bool {
    let output = match Command::new("git").args(["status", "--porcelain"]).output() {
        Ok(output) => output,
        Err(_) => return false,
    };

    output.status.success() && !output.stdout.is_empty()
}
