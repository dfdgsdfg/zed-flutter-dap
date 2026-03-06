pub fn status_json() -> serde_json::Value {
    serde_json::json!({
        "version": version(),
        "commit": commit(),
        "tag": option_env!("PROXY_GIT_TAG"),
    })
}

fn version() -> &'static str {
    option_env!("PROXY_GIT_TAG")
        .and_then(|tag| tag.strip_prefix("dap-proxy-v"))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn commit() -> Option<String> {
    option_env!("PROXY_GIT_COMMIT").map(|commit| {
        if option_env!("PROXY_GIT_DIRTY") == Some("1") {
            format!("{commit}-dirty")
        } else {
            commit.to_string()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_json_includes_version() {
        let status = status_json();
        assert!(status["version"].as_str().is_some());
    }

    #[test]
    fn version_uses_release_tag_when_present() {
        let value = version();
        assert!(!value.is_empty());
    }
}
