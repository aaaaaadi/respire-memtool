//! Read-only diagnostic for the configuration selected by the installed build.
//! Run: cargo run --release --example config_update_probe

use memtools_lib::{config::AppConfig, rsrs_client::RsrsClient, updater};

fn main() -> Result<(), String> {
    let config = AppConfig::load();
    let client = RsrsClient::new(
        config.rsrs.base_url.clone(),
        config.rsrs.mcp_path.clone(),
        config.rsrs.tools.clone(),
        config.rsrs.timeout_secs,
        config.rsrs.op_timeout_secs,
    );
    // Print only status, never the configuration body or memory contents.
    let runtime = client.status().map(|_| "ok").map_err(|_| "unavailable");
    let (update, _) = updater::check(&config.update.endpoint).map_err(|_| {
        "update check failed (details omitted to protect configured URLs)".to_string()
    })?;
    println!(
        "{}",
        serde_json::json!({
            "config_path": AppConfig::path(),
            "runtime": runtime,
            "update": update,
        })
    );
    runtime.map(|_| ()).map_err(str::to_owned)
}
