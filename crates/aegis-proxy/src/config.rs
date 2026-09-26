use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub listen_addr: String,
    pub metrics_addr: String,
    pub downstream_url: String,
    pub blocklist_path: String,
    /// Optional path to a Wasm plugin (.wasm). If set, enables Stage 3 (dynamic rules).
    pub wasm_plugin_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:8899".into(),
            metrics_addr: "0.0.0.0:9090".into(),
            downstream_url: "http://127.0.0.1:8900".into(),
            blocklist_path: "blocklist.txt".into(),
            wasm_plugin_path: None,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("LISTEN_ADDR") { cfg.listen_addr = v; }
        if let Ok(v) = std::env::var("METRICS_ADDR") { cfg.metrics_addr = v; }
        if let Ok(v) = std::env::var("DOWNSTREAM_URL") { cfg.downstream_url = v; }
        if let Ok(v) = std::env::var("BLOCKLIST_PATH") { cfg.blocklist_path = v; }
        if let Ok(v) = std::env::var("WASM_PLUGIN_PATH") { cfg.wasm_plugin_path = Some(v); }
        cfg
    }
}
