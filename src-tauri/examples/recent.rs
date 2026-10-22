//! 只读验证全部有效记忆和空分类，不创建或改挂节点。
use memtools_lib::{config::AppConfig, recent, rsrs_client::RsrsClient};
fn main() -> Result<(), String> {
    let cfg = AppConfig::load();
    let client = RsrsClient::new(
        cfg.rsrs.base_url.clone(),
        cfg.rsrs.mcp_path.clone(),
        cfg.rsrs.tools.clone(),
        cfg.rsrs.timeout_secs,
        cfg.rsrs.op_timeout_secs,
    )
    .with_token(cfg.rsrs.auth_token.clone());
    let mut args = std::env::args().skip(1);
    let category = args.next().filter(|s| s != "全部" && s != "all");
    let limit = args
        .next()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n != 0);
    let entries = client.list_all().map_err(|e| e.to_string())?;
    let total = entries.len();
    let items = recent::filter_category(entries, category.as_deref(), limit);
    println!(
        "valid_total={total}, selected={}, category={}",
        items.len(),
        category.as_deref().unwrap_or("全部")
    );
    Ok(())
}
