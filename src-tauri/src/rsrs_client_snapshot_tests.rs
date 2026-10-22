use super::*;
    fn snapshot_fixture(expected: u32, returned: u32) -> Result<Vec<ListedEntry>, RsrsError> {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..2 {
                let mut req = server
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap()
                    .unwrap();
                let mut body = String::new();
                req.as_reader().read_to_string(&mut body).unwrap();
                let call: serde_json::Value = serde_json::from_str(&body).unwrap();
                let envelope = if call["params"]["name"] == "memory_status" {
                    serde_json::json!({"status":"ok","summary":{"local_alive":expected}})
                } else {
                    assert_eq!(call["params"]["name"], "memory_list");
                    assert!(
                        call["params"]["arguments"]["limit"].as_u64().unwrap() > expected as u64
                    );
                    let entries:Vec<_>=(0..returned).map(|i|serde_json::json!({"id":format!("fixture-{i}"),"title":"fixture","content":"test only","importance":if i == 0 {"trivial"} else {"important"},"parent_id":null})).collect();
                    serde_json::json!({"status":"ok","summary":{"count":returned},"details":entries})
                };
                let result = serde_json::json!({"jsonrpc":"2.0","id":call["id"],"result":{"content":[{"type":"text","text":envelope.to_string()}]}});
                req.respond(tiny_http::Response::from_string(result.to_string()))
                    .unwrap();
            }
        });
        let cfg: crate::config::AppConfig = toml::from_str(include_str!("../config.toml")).unwrap();
        let client = RsrsClient::new(
            format!("http://{address}"),
            "mcp".into(),
            cfg.rsrs.tools,
            2,
            2,
        );
        client.inner.lock().unwrap().initialized = true;
        let result = client.list_all();
        worker.join().unwrap();
        result
    }
    #[test]
    fn all_snapshot_includes_diary_and_more_than_two_hundred() {
        let items = snapshot_fixture(478, 478).unwrap();
        assert_eq!(items.len(), 478);
        assert_eq!(items[0].importance.as_deref(), Some("trivial"));
    }
    #[test]
    fn partial_snapshot_cannot_be_reported_as_all() {
        assert!(matches!(snapshot_fixture(478, 200), Err(RsrsError::Mcp(_))));
    }
