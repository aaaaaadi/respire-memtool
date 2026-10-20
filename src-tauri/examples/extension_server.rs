//! 浏览器扩展端点冒烟测试（只读，不写记忆）
//!
//! 运行：`cargo run --example extension_server`
//! 验证：随机端口启动、无令牌 401、错误令牌 401、正确令牌 /health 200、空内容 /remember 400。

use std::time::Duration;

use memtools_lib::config::AppConfig;
use memtools_lib::server;

fn main() {
    let mut cfg = AppConfig::load();
    cfg.extension.enabled = true;
    cfg.extension.port = 0; // 让系统分配空闲端口，避免和正在运行的应用冲突

    let token_path = std::env::temp_dir().join("memtools_ext_token_smoke.txt");
    let _ = std::fs::remove_file(&token_path);

    let started = server::start(cfg, token_path.clone(), std::env::temp_dir())
        .expect("启动扩展端点失败（端口被占用？）");
    println!("endpoint = http://127.0.0.1:{}", started.port);
    println!("token    = {}…", &started.token[..8]);

    let base = format!("http://127.0.0.1:{}", started.port);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();

    // 1. 无令牌 → 401
    match agent.get(&format!("{base}/health")).call() {
        Err(ureq::Error::Status(401, _)) => println!("ok: 无令牌 → 401"),
        other => panic!("无令牌应返回 401，实际: {other:?}"),
    }

    // 2. 错令牌 → 401
    match agent
        .get(&format!("{base}/health"))
        .set("X-MemTools-Token", "wrong")
        .call()
    {
        Err(ureq::Error::Status(401, _)) => println!("ok: 错令牌 → 401"),
        other => panic!("错令牌应返回 401，实际: {other:?}"),
    }

    // 3. 正确令牌 → 200 + JSON
    let body: serde_json::Value = agent
        .get(&format!("{base}/health"))
        .set("X-MemTools-Token", &started.token)
        .call()
        .expect("/health 应返回 200")
        .into_json()
        .expect("应返回 JSON");
    assert_eq!(body["ok"], true);
    assert_eq!(body["name"], "memTools");
    println!("ok: /health → {}", body);

    // 4. 空内容 → 400（不会写库）
    match agent
        .post(&format!("{base}/remember"))
        .set("X-MemTools-Token", &started.token)
        .send_json(serde_json::json!({ "text": "   " }))
    {
        Err(ureq::Error::Status(400, _)) => println!("ok: 空内容 → 400"),
        other => panic!("空内容应返回 400，实际: {other:?}"),
    }

    println!("全部通过（未写入任何记忆）");
}
