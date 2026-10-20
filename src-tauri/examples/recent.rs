//! 干跑「最近记忆浏览」（只读，不写库）
//!
//! cargo run --example recent -- [分类名|全部] [条数]
//! 例：cargo run --example recent -- 全部 20
//!     cargo run --example recent -- 编程开发 10

use std::collections::HashSet;

use memtools_lib::config::AppConfig;
use memtools_lib::recent;
use memtools_lib::rsrs_client::RsrsClient;

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
    let cat = args.next().unwrap_or_else(|| "全部".into());
    let limit: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
        .clamp(1, 200);

    if cat == "全部" || cat == "all" {
        let items = client.list_recent(limit).map_err(|e| format!("{e:?}"))?;
        println!("全部类别最近 {} 条：", items.len());
        for it in items {
            println!(
                "{}  {}  [{}]",
                it.created_at.unwrap_or_default(),
                it.title,
                it.id
            );
        }
        return Ok(());
    }

    let taxo = client.list_taxonomy().map_err(|e| format!("{e:?}"))?;
    let node = taxo
        .iter()
        .find(|t| t.name == cat || t.id == cat)
        .ok_or_else(|| format!("未找到分类 {cat}"))?
        .clone();
    let nodes = client.tree_flat(&node.id, 50).map_err(|e| format!("{e:?}"))?;
    let ids: HashSet<String> = nodes.into_iter().map(|n| n.id).collect();
    println!("分类「{}」子树节点 {} 个", node.name, ids.len());

    let mut window = recent::initial_window(limit);
    loop {
        let items = client.list_recent(window).map_err(|e| format!("{e:?}"))?;
        let scanned = items.len() as u32;
        let filtered = recent::filter_entries(items, &ids, limit as usize);
        let enough = filtered.len() >= limit as usize;
        let exhausted = scanned < window;
        let can_expand = !exhausted && window < 2000;
        if enough || !can_expand {
            println!(
                "扫描最近 {} 条，命中 {} 条（truncated={}）：",
                scanned,
                filtered.len(),
                !enough && !exhausted
            );
            for it in filtered {
                println!(
                    "{}  {}  [{}]",
                    it.created_at.unwrap_or_default(),
                    it.title,
                    it.id
                );
            }
            return Ok(());
        }
        window = (window * 2).min(2000);
    }
}
