//! 干跑「提醒扫描」（只读，不写库）
//!
//! cargo run --example reminders_probe -- [显示条数]

use memtools_lib::config::AppConfig;
use memtools_lib::reminders;
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
    let limit: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20)
        .min(200);

    let cache = std::env::temp_dir().join("memtools-reminders-probe");
    let out = reminders::scan(&client, cfg.reminder.soon_days, &cache)?;
    println!(
        "核对日期 {} · 扫描 {} 条 · truncated={} · 逾期 {} / 今天 {} / 即将 {}",
        out.checked_at, out.scanned, out.truncated, out.overdue, out.today, out.soon
    );
    for it in out.items.iter().take(limit) {
        println!("{}  {:>8}  [{}]  {}", it.due, it.status, it.id, it.title);
        for line in it.content.lines() {
            if line.trim_start().starts_with("【触发】") {
                println!("    {line}");
            }
        }
    }
    Ok(())
}
