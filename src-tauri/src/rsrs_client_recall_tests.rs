use super::*;

fn with_reply<T>(
    tool: &str,
    envelope: serde_json::Value,
    is_error: bool,
    run: impl FnOnce(&RsrsClient) -> T,
) -> T {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let address = server.server_addr().to_ip().unwrap();
    let expected_tool = tool.to_string();
    let worker = std::thread::spawn(move || {
        let mut req = server
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let mut body = String::new();
        req.as_reader().read_to_string(&mut body).unwrap();
        let call: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(call["params"]["name"], expected_tool);
        let response = serde_json::json!({"jsonrpc":"2.0","id":call["id"],"result":{
            "isError":is_error,"content":[{"type":"text","text":envelope.to_string()}]}});
        req.respond(tiny_http::Response::from_string(response.to_string()))
            .unwrap();
    });
    let mut cfg: crate::config::AppConfig = toml::from_str(include_str!("../config.toml")).unwrap();
    if tool == "custom_recall" {
        cfg.rsrs.tools.recall = tool.into();
    }
    let client = RsrsClient::new(
        format!("http://{address}"),
        "mcp".into(),
        cfg.rsrs.tools,
        2,
        2,
    );
    client.inner.lock().unwrap().initialized = true;
    let result = run(&client);
    worker.join().unwrap();
    result
}

#[test]
fn recall_skip_zero_candidates_is_a_valid_empty_result() {
    for tool in ["memory_recall", "custom_recall"] {
        for details in [serde_json::Value::Null, serde_json::json!([])] {
            let result = with_reply(
                tool,
                serde_json::json!({"status":"skip","summary":{"count":0},
                "items":[{"name":"results","status":"skip","value":"0"}],"details":details,"errors":[]}),
                false,
                |client| client.recall_candidates("synthetic", 3),
            );
            assert!(result.unwrap().is_empty());
        }
    }
}

#[test]
fn recall_skip_does_not_hide_errors_or_inconsistent_results() {
    for envelope in [
        serde_json::json!({"status":"skip","summary":{"count":0},"errors":["runtime unavailable"]}),
        serde_json::json!({"status":"skip","summary":{"count":1}}),
        serde_json::json!({"status":"skip","summary":{}}),
        serde_json::json!({"status":"skip","summary":{"count":0},"details":[{"entry":{}}]}),
        serde_json::json!({"status":"skip","summary":{"count":0},"details":{"unexpected":true}}),
        serde_json::json!({"status":"fail","summary":{"count":0}}),
    ] {
        assert!(with_reply("memory_recall", envelope, false, |client| client
            .recall_candidates("synthetic", 3))
        .is_err());
    }
    assert!(with_reply(
        "memory_recall",
        serde_json::json!({"status":"skip","summary":{"count":0}}),
        true,
        |client| client.recall_candidates("synthetic", 3)
    )
    .is_err());
}

#[test]
fn remember_skip_is_never_treated_as_a_successful_write() {
    assert!(with_reply(
        "memory_remember",
        serde_json::json!({"status":"skip","summary":{"count":0}}),
        false,
        |client| client.remember(RememberRequest {
            content: "synthetic only".into(),
            ..Default::default()
        })
    )
    .is_err());
}

#[test]
fn empty_recall_allows_automation_to_offer_a_new_memory() {
    let mut cfg: crate::config::AppConfig = toml::from_str(include_str!("../config.toml")).unwrap();
    cfg.automation.recall_rounds = 1;
    let plan = with_reply(
        "memory_recall",
        serde_json::json!({"status":"skip","summary":{"count":0}}),
        false,
        |client| {
            crate::automation::plan(
                client,
                "【行为】以后代码修改必须保护本地尚未提交的内容并清楚记录验证结果。",
                None,
                &[],
                &cfg.automation,
            )
        },
    );
    let plan = plan.unwrap();
    assert_eq!(plan.branch, "create");
    assert!(plan.candidates.is_empty());
    assert!(plan.needs_confirm, "没有分类树时仍须用户选择，不能直接写入");
}

// 仅在用户授权的本机诊断中显式运行；只做 recall，不创建测试记忆。
#[test]
#[ignore = "需要显式授权连接本机 RSRS Runtime 的只读检查"]
fn live_runtime_zero_recall_is_empty() {
    let cfg: crate::config::AppConfig = toml::from_str(include_str!("../config.toml")).unwrap();
    let client = RsrsClient::new(
        "http://127.0.0.1:15169".into(),
        "mcp".into(),
        cfg.rsrs.tools,
        5,
        30,
    );
    let items = client
        .recall_candidates("zzqv893fa7d5c0e14b6d96a82dff318cf764", 3)
        .unwrap();
    assert!(items.is_empty());
}
