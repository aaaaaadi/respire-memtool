//! 干跑「模型判档 / 判树」（只读，不写库）
//!
//! cargo run --example llm_probe -- "<内容>" [项目/关键词提示]
//! cargo run --example llm_probe -- --mock        # 内置 mock 端点，验证 HTTP 解析链路
//! 使用 config.toml 的 [automation.llm] 端点与模型。

use memtools_lib::config::AppConfig;
use memtools_lib::config::LlmConfig;
use memtools_lib::llm;
use memtools_lib::rsrs_client::RsrsClient;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s.as_str()) == Some("--mock") {
        return mock_probe();
    }
    let cfg = AppConfig::load();
    let content = args.first().ok_or("用法: llm_probe <内容> [提示]")?.clone();
    let hint = args.get(1).cloned();

    let client = RsrsClient::new(
        cfg.rsrs.base_url.clone(),
        cfg.rsrs.mcp_path.clone(),
        cfg.rsrs.tools.clone(),
        cfg.rsrs.timeout_secs,
        cfg.rsrs.op_timeout_secs,
    )
    .with_token(cfg.rsrs.auth_token.clone());
    let trees: Vec<String> = client
        .list_taxonomy()
        .map_err(|e| format!("{e:?}"))?
        .into_iter()
        .map(|t| t.name)
        .collect();
    println!("端点 {} · 模型 {}", cfg.automation.llm.endpoint, cfg.automation.llm.model);

    let v = llm::judge(&cfg.automation.llm, &content, hint.as_deref(), &trees)?;
    println!("importance = {:?}", v.importance);
    println!("tree       = {:?}", v.tree);
    println!("title      = {:?}", v.title);
    println!("reason     = {:?}", v.reason);
    Ok(())
}

fn mock_probe() -> Result<(), String> {
    use tiny_http::{Header, Response, Server};

    let server = Server::http("127.0.0.1:0").map_err(|e| format!("mock 启动失败: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .ok_or("mock 地址不是 IP")?
        .port();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let body = r#"{"choices":[{"message":{"content":"```json\n{\"importance\":\"important\",\"tree\":\"编程开发\",\"title\":\"验证模型判档链路\",\"reason\":\"mock 端点\"}\n```"}}]}"#;
            let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
            let _ = req.respond(Response::from_string(body).with_header(ct));
        }
    });

    let cfg = LlmConfig {
        enabled: true,
        endpoint: format!("http://127.0.0.1:{port}/v1/chat/completions"),
        model: "mock".into(),
        api_key: String::new(),
        timeout_secs: 10,
    };
    let trees = vec!["编程开发".to_string(), "生活日常".to_string()];
    let v = llm::judge(&cfg, "【行为】实现提醒与模型判档", None, &trees)?;
    println!("mock port = {port}");
    println!("importance = {:?}", v.importance);
    println!("tree       = {:?}", v.tree);
    println!("title      = {:?}", v.title);
    assert_eq!(v.importance, "important");
    assert_eq!(v.tree, "编程开发");
    assert_eq!(v.title, "验证模型判档链路");
    println!("mock OK");
    Ok(())
}
