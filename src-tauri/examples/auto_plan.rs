//! 自动化路径冒烟工具（默认 dry-run，不写入）。
//!
//! 用法：
//!   1) 干跑（不写入，仅打印 plan）：
//!      cargo run --example auto_plan -- "<内容>" [hint]
//!
//!   2) 实跑单分支（会写入！仅用于自测，用后请清理）：
//!      cargo run --example auto_plan -- apply <journal|attach|create|update|merge> "<内容>" [candidate] [tree]
//!
//! 用途：无需 GUI 即可验证「判档 + 判重两轮 + 判树 + 择一执行」在真实 runtime 上的行为。

use memtools_lib::automation;
use memtools_lib::config::AppConfig;
use memtools_lib::rsrs_client::RsrsClient;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cfg = AppConfig::load();
    let client = RsrsClient::new(
        cfg.rsrs.base_url.clone(),
        cfg.rsrs.mcp_path.clone(),
        cfg.rsrs.tools.clone(),
        cfg.rsrs.timeout_secs,
        cfg.rsrs.op_timeout_secs,
    )
    .with_token(cfg.rsrs.auth_token.clone());

    if args.first().map(|s| s == "apply").unwrap_or(false) {
        let branch = args.get(1).cloned().unwrap_or_default();
        let content = args.get(2).cloned().unwrap_or_default();
        let candidate = args.get(3).cloned();
        let tree = args.get(4).cloned();
        let title = automation::build_title(&content);
        let importance = automation::judge_importance(&content).to_string();
        let backup_dir = std::env::temp_dir().join("memtools-smoke-backups");
        println!("[auto_plan] apply branch={branch} candidate={candidate:?} tree={tree:?}");
        match automation::apply(
            &client,
            &content,
            &title,
            &branch,
            candidate.as_deref(),
            tree.as_deref(),
            &importance,
            &backup_dir,
        ) {
            Ok(o) => println!(
                "[auto_plan] outcome =\n{}",
                serde_json::to_string_pretty(&o).unwrap_or_default()
            ),
            Err(e) => {
                eprintln!("[auto_plan] apply 失败: {e}");
                std::process::exit(2);
            }
        }
        return;
    }

    let content = args
        .first()
        .cloned()
        .unwrap_or_else(|| "【前因】干跑【行为】干跑【后果】干跑".to_string());
    let hint = args.get(1).cloned();

    let taxonomy = match client.list_taxonomy() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[auto_plan] taxonomy 读取失败: {e}");
            std::process::exit(1);
        }
    };
    println!("[auto_plan] 分类树根: {} 条", taxonomy.len());
    println!(
        "[auto_plan] 判档 judge_importance = {}",
        automation::judge_importance(&content)
    );

    match automation::plan(&client, &content, hint.as_deref(), &taxonomy, &cfg.automation) {
        Ok(plan) => println!(
            "[auto_plan] plan =\n{}",
            serde_json::to_string_pretty(&plan).unwrap_or_default()
        ),
        Err(e) => {
            eprintln!("[auto_plan] plan 失败: {e}");
            std::process::exit(2);
        }
    }
}
