use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FeedSpec {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub listen_addr: String,
    pub metrics_addr: String,
    pub downstream_url: String,
    pub blocklist_path: String,
    /// Optional path to a Wasm plugin (.wasm). If set, enables Stage 3 (dynamic rules).
    pub wasm_plugin_path: Option<String>,
    /// Operator-chosen third-party address feeds, merged with the local blocklist file.
    pub threat_feeds: Vec<FeedSpec>,
    pub threat_feed_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:8899".into(),
            metrics_addr: "0.0.0.0:9090".into(),
            downstream_url: "http://127.0.0.1:8900".into(),
            blocklist_path: "blocklist.txt".into(),
            wasm_plugin_path: None,
            threat_feeds: Vec::new(),
            threat_feed_interval_secs: 300,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("LISTEN_ADDR") { cfg.listen_addr = v; }
        if let Ok(v) = std::env::var("METRICS_ADDR") { cfg.metrics_addr = v; }
        if let Ok(v) = std::env::var("DOWNSTREAM_URL") { cfg.downstream_url = v; }
        if let Ok(v) = std::env::var("BLOCKLIST_PATH") { cfg.blocklist_path = v; }
        if let Ok(v) = std::env::var("WASM_PLUGIN_PATH") { cfg.wasm_plugin_path = Some(v); }
        if let Ok(v) = std::env::var("THREAT_FEEDS") { cfg.threat_feeds = parse_feeds(&v)?; }
        if let Ok(v) = std::env::var("THREAT_FEED_INTERVAL_SECS") {
            cfg.threat_feed_interval_secs = v
                .parse::<u64>()
                .ok()
                .filter(|s| *s > 0)
                .ok_or_else(|| format!("THREAT_FEED_INTERVAL_SECS must be a positive integer, got {v:?}"))?;
        }
        Ok(cfg)
    }
}

/// Parses `name=url,name2=url2`. Names become the `blocklist:<name>` rule id, so they are
/// restricted to a label-safe charset and must be unique.
pub fn parse_feeds(spec: &str) -> Result<Vec<FeedSpec>, String> {
    let mut feeds: Vec<FeedSpec> = Vec::new();
    for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, url) = entry
            .split_once('=')
            .ok_or_else(|| format!("threat feed {entry:?} must be name=url"))?;
        let (name, url) = (name.trim(), url.trim());
        if name.is_empty()
            || name == "file"
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("invalid threat feed name {name:?}: use [A-Za-z0-9_-], not \"file\""));
        }
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(format!("threat feed {name} url must be http(s), got {url:?}"));
        }
        if feeds.iter().any(|f| f.name == name) {
            return Err(format!("duplicate threat feed name {name:?}"));
        }
        feeds.push(FeedSpec { name: name.into(), url: url.into() });
    }
    Ok(feeds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_feed_list() {
        let feeds = parse_feeds(" seal=https://a.example/list.txt , trm_1=http://b:8080/x ,").unwrap();
        assert_eq!(
            feeds,
            vec![
                FeedSpec { name: "seal".into(), url: "https://a.example/list.txt".into() },
                FeedSpec { name: "trm_1".into(), url: "http://b:8080/x".into() },
            ]
        );
        assert!(parse_feeds("").unwrap().is_empty());
    }

    #[test]
    fn rejects_bad_feeds() {
        for bad in [
            "noequals",
            "=https://x",
            "a b=https://x",
            "file=https://x",
            "x=ftp://x",
            "x=https://a,x=https://b",
        ] {
            assert!(parse_feeds(bad).is_err(), "{bad} should be rejected");
        }
    }

    proptest! {
        #[test]
        fn parse_feeds_never_panics(s in "\\PC*") {
            let _ = parse_feeds(&s);
        }

        // Invariant: accepted feeds always have unique, label-safe names and http(s) urls.
        #[test]
        fn accepted_feeds_are_well_formed(s in "([a-z_=,:/.-]|https://){0,60}") {
            if let Ok(feeds) = parse_feeds(&s) {
                let mut names = std::collections::HashSet::new();
                for f in &feeds {
                    prop_assert!(names.insert(f.name.clone()));
                    prop_assert!(f.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
                    prop_assert!(f.url.starts_with("http://") || f.url.starts_with("https://"));
                }
            }
        }
    }
}
