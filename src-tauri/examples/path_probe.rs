//! 干跑「挂点路径 / 目标树 outline」（只读，不写库）
//!
//! cargo run --example path_probe -- <id>

use memtools_lib::config::AppConfig;
use memtools_lib::rsrs_client::RsrsClient;

fn main() -> Result<(), String> {
    let id = std::env::args()
        .nth(1)
        .ok_or("用法: path_probe <id>（支持 8 位前缀）")?;
    let cfg = AppConfig::load();
    let client = RsrsClient::new(
        cfg.rsrs.base_url.clone(),
        cfg.rsrs.mcp_path.clone(),
        cfg.rsrs.tools.clone(),
        cfg.rsrs.timeout_secs,
        cfg.rsrs.op_timeout_secs,
    )
    .with_token(cfg.rsrs.auth_token.clone());

    let shown = client.show_entry(&id).map_err(|e| format!("{e:?}"))?;
    let mut path: Vec<String> = shown.ancestors.iter().map(|n| n.title.clone()).collect();
    path.push(shown.entry.title.clone());
    println!("路径：{}", path.join(" / "));

    let nodes = client.tree_outline(&id, 2).map_err(|e| format!("{e:?}"))?;
    println!("outline {} 个节点：", nodes.len());
    for n in nodes.iter().take(20) {
        println!("{}· [{}] {}", "  ".repeat(n.depth as usize), n.short_id, n.title);
    }
    Ok(())
}
