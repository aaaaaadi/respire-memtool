//! 干跑「记忆搜索」（只读，不写库）
//!
//! cargo run --example search -- "内容查询" [条数]
//! cargo run --example search -- bd22ad3a        # 8 位 id 前缀 → 精确命中
//!
//! 内容查询走 rsrs recall 语义匹配；id 走 memory_show。

use memtools_lib::config::AppConfig;
use memtools_lib::rsrs_client::RsrsClient;

fn looks_like_id(s: &str) -> bool {
    let t = s.trim();
    (8..=36).contains(&t.len())
        && t.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && t.chars().filter(|c| c.is_ascii_hexdigit()).count() >= 8
}

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
    let query = args.next().unwrap_or_else(|| "记忆".into());
    let limit: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
        .clamp(1, 100);

    if looks_like_id(&query) {
        let shown = client.show_entry(&query).map_err(|e| e.to_string())?;
        let path = shown
            .ancestors
            .iter()
            .map(|n| n.title.clone())
            .collect::<Vec<_>>()
            .join(" / ");
        println!("id 精确命中 · 路径：{path}");
        println!("[{}] {}", shown.entry.id, shown.entry.title);
        println!("{}", shown.entry.content);
        return Ok(());
    }

    let cands = client
        .recall_candidates(&query, limit)
        .map_err(|e| e.to_string())?;
    println!("recall \"{query}\" 命中 {} 条：", cands.len());
    for c in cands {
        println!(
            "{:>3}%  [{}] {}{}",
            (c.score * 100.0).round() as i64,
            c.entry.id,
            c.entry.title,
            c.superseded_by
                .as_deref()
                .map(|s| format!("  (已被 {s} 取代)"))
                .unwrap_or_default()
        );
    }
    Ok(())
}
