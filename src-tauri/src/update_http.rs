//! µ¢┤µû░Σ╕ôτö¿ HTTP σ«óµê╖τ½»∩╝¢Σ╗úτÉåΘÇëµï⌐Σ╕ìσ╜▒σôìµ£¼µ£║ Runtime σ«óµê╖τ½»πÇé
use tauri::Url;

#[derive(Clone, Default)]
struct ProxySettings {
    environment: Option<String>,
    system: Option<String>,
    bypass: Option<String>,
}

/// µ£¼µ£║σ¢₧τÄ»σºïτ╗êτ¢┤Φ┐₧∩╝¢NO_PROXY Σ╗ÑΣ╕╗µ£║Φ╛╣τòîσî╣Θàì∩╝îΘü┐σàìΦ»»µÄÆΘÖñτ¢╕Σ╝╝σƒƒσÉìπÇé
fn select_proxy(url: &Url, settings: &ProxySettings) -> Option<String> {
    let host = url
        .host_str()
        .unwrap_or("")
        .trim_matches(['[', ']'])
        .to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
    {
        return None;
    }
    if settings
        .bypass
        .as_deref()
        .map(|list| {
            list.split([',', ';'])
                .any(|entry| bypass_matches(&host, url.port_or_known_default(), entry))
        })
        .unwrap_or(false)
    {
        return None;
    }
    if let Some(proxy) = &settings.environment {
        return Some(proxy.clone());
    }
    let system = settings.system.as_deref()?.trim();
    if !system.contains('=') {
        return (!system.is_empty()).then(|| system.to_string());
    }
    system
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(scheme, _)| scheme.eq_ignore_ascii_case(url.scheme()))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bypass_matches(host: &str, port: Option<u16>, entry: &str) -> bool {
    let entry = entry.trim().to_ascii_lowercase();
    if entry == "*" || (entry == "<local>" && !host.contains('.') && !host.contains(':')) {
        return true;
    }
    let (domain, only_port) = match entry.rsplit_once(':') {
        Some((domain, value)) if value.parse::<u16>().is_ok() => {
            (domain, value.parse::<u16>().ok())
        }
        _ => (entry.as_str(), None),
    };
    if only_port.is_some() && only_port != port {
        return false;
    }
    let domain = domain.trim_start_matches('*').trim_start_matches('.');
    !domain.is_empty() && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// σÅ¬Φ»╗σÅû Windows σ╜ôσëìτö¿µê╖τÜäΘ¥ÖµÇüΣ╗úτÉå∩╝îΣ╕ìΣ┐«µö╣µ│¿σåîΦí¿µêûσà¿σ▒ÇΣ╗úτÉåτ¡ûτòÑπÇé
#[cfg(windows)]
fn windows_proxy() -> (Option<String>, Option<String>) {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
    else {
        return (None, None);
    };
    if key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 0 {
        return (None, None);
    }
    (
        key.get_value("ProxyServer").ok(),
        key.get_value("ProxyOverride").ok(),
    )
}

#[cfg(not(windows))]
fn windows_proxy() -> (Option<String>, Option<String>) {
    (None, None)
}

fn agent(endpoint: &str, settings: &ProxySettings) -> Result<ureq::Agent, String> {
    let url = Url::parse(endpoint).map_err(|_| "µ¢┤µû░σ£░σ¥Çµùáµòê".to_string())?;
    let mut builder = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0);
    if let Some(proxy) = select_proxy(&url, settings) {
        builder = builder.proxy(
            ureq::Proxy::new(proxy)
                .map_err(|_| "µ¢┤µû░Σ╗úτÉåΘàìτ╜«µùáµòê∩╝îΦ»╖µúÇµƒÑτÄ»σóâσÅÿΘçÅµêûτ│╗τ╗ƒΣ╗úτÉåπÇé".to_string())?,
        );
    }
    Ok(builder.build())
}

/// µîëσ╜ôσëìΦ»╖µ▒éσ£░σ¥ÇΦ»╗σÅûΣ╗úτÉå∩╝îΘçìσ«ÜσÉæσê░Σ╕ìσÉîΣ╕╗µ£║µêûσìÅΦ««µù╢σ┐àΘí╗Θçìµû░ΘÇëµï⌐πÇé
fn current_proxy_settings(url: &Url) -> ProxySettings {
    let names = if url.scheme() == "https" {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    let environment = names
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty());
    let (system, system_bypass) = windows_proxy();
    let bypass = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .ok()
        .or_else(|| {
            if environment.is_none() {
                system_bypass
            } else {
                None
            }
        });
    ProxySettings {
        environment,
        system,
        bypass,
    }
}

/// µúÇµƒÑΣ╕ÄΣ╕ïΦ╜╜σà▒τö¿ GET∩╝¢µ»ÅΣ╕ÇΦ╖│σìòτï¼ΘÇëµï⌐Σ╗úτÉå∩╝îµëÇµ£ëΦ╖│σà▒Σ║½µÇ╗Φ╢àµù╢Σ╕Äµ¼íµò░Σ╕èΘÖÉπÇé
pub fn get(endpoint: &str, timeout: std::time::Duration) -> Result<ureq::Response, String> {
    get_following_redirects(endpoint, timeout, current_proxy_settings)
}

fn get_following_redirects(
    endpoint: &str,
    timeout: std::time::Duration,
    settings_for: impl Fn(&Url) -> ProxySettings,
) -> Result<ureq::Response, String> {
    let mut url = Url::parse(endpoint).map_err(|_| "µ¢┤µû░σ£░σ¥Çµùáµòê".to_string())?;
    let start = std::time::Instant::now();
    for redirects in 0..=10 {
        // 1. µ»ÅΦ╖│Θçìµû░σ║öτö¿σìÅΦ««πÇüσ¢₧τÄ»σÆî NO_PROXY ΦºäσêÖ∩╝îΘü┐σàìµèèΣ╕èΣ╕ÇΦ╖│Σ╗úτÉåσ╕ªσê░µû░Σ╕╗µ£║πÇé
        if !matches!(url.scheme(), "http" | "https") {
            return Err("µ¢┤µû░σ£░σ¥Çσ┐àΘí╗Σ╜┐τö¿ HTTP µêû HTTPS".into());
        }
        let remaining = timeout
            .checked_sub(start.elapsed())
            .filter(|d| !d.is_zero())
            .ok_or("µ¢┤µû░Φ»╖µ▒éΦ╢àµù╢")?;
        let response = agent(url.as_str(), &settings_for(&url))?
            .get(url.as_str())
            .set("User-Agent", "memTools-updater")
            .timeout(remaining)
            .call()
            .map_err(|e| e.to_string())?;
        // 2. σÅ¬Φ╖ƒΘÜÅ GET τÜäµáçσçåΘçìσ«ÜσÉæ∩╝îΦºúµ₧Éτ¢╕σ»╣ Location∩╝îσ╣╢ΘÖÉσê╢σ╛¬τÄ»Σ╕Äµò┤Σ╜ôΦÇùµù╢πÇé
        if !matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if redirects == 10 {
            return Err("µ¢┤µû░Θçìσ«ÜσÉæµ¼íµò░Φ╢àΦ┐çΣ╕èΘÖÉ∩╝ê10∩╝ë".into());
        }
        let location = response
            .header("Location")
            .ok_or("µ¢┤µû░Θçìσ«ÜσÉæτ╝║σ░æ Location")?;
        url = url
            .join(location)
            .map_err(|_| "µ¢┤µû░Θçìσ«ÜσÉæσ£░σ¥Çµùáµòê".to_string())?;
    }
    unreachable!("redirect loop is bounded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_environment_proxy_overrides_windows_proxy() {
        let settings = ProxySettings {
            environment: Some("http://env:8080".into()),
            system: Some("http=win:80;https=win:443".into()),
            bypass: None,
        };
        assert_eq!(
            select_proxy(
                &Url::parse("https://updates.example/a.exe").unwrap(),
                &settings
            )
            .as_deref(),
            Some("http://env:8080")
        );
    }

    #[test]
    fn windows_scheme_map_is_used_when_environment_has_no_proxy() {
        let settings = ProxySettings {
            system: Some("http=win:80;https=win:443".into()),
            ..Default::default()
        };
        assert_eq!(
            select_proxy(
                &Url::parse("https://updates.example/a.exe").unwrap(),
                &settings
            )
            .as_deref(),
            Some("win:443")
        );
    }

    #[test]
    fn local_runtime_and_mock_servers_always_bypass_proxy() {
        let settings = ProxySettings {
            environment: Some("http://proxy:8080".into()),
            ..Default::default()
        };
        for host in ["localhost", "127.0.0.1", "127.12.34.56", "[::1]"] {
            assert!(select_proxy(
                &Url::parse(&format!("http://{host}:15169/mcp")).unwrap(),
                &settings
            )
            .is_none());
        }
    }

    #[test]
    fn no_proxy_covers_domain_boundaries_and_ports() {
        let settings = ProxySettings {
            environment: Some("proxy:8080".into()),
            bypass: Some(".internal.example,other.example:8443".into()),
            ..Default::default()
        };
        assert!(select_proxy(
            &Url::parse("https://a.internal.example/update").unwrap(),
            &settings
        )
        .is_none());
        assert!(select_proxy(
            &Url::parse("https://other.example:8443/update").unwrap(),
            &settings
        )
        .is_none());
        assert_eq!(
            select_proxy(
                &Url::parse("https://notinternal.example/update").unwrap(),
                &settings
            )
            .as_deref(),
            Some("proxy:8080")
        );
        assert_eq!(
            select_proxy(
                &Url::parse("https://other.example/update").unwrap(),
                &settings
            )
            .as_deref(),
            Some("proxy:8080")
        );
    }

    #[test]
    fn real_request_goes_through_the_selected_proxy() {
        let proxy = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = proxy.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            if let Some(req) = proxy
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
            {
                assert_eq!(req.url(), "http://updates.invalid/update.json");
                req.respond(tiny_http::Response::from_string(r#"{"version":"0.3.1"}"#))
                    .unwrap();
            }
        });
        let settings = ProxySettings {
            environment: Some(format!("http://{address}")),
            ..Default::default()
        };
        let response = agent("http://updates.invalid/update.json", &settings)
            .unwrap()
            .get("http://updates.invalid/update.json")
            .timeout(std::time::Duration::from_secs(2))
            .call();
        worker.join().unwrap();
        let value: serde_json::Value = response.unwrap().into_json().unwrap();
        assert_eq!(value["version"], "0.3.1");
    }
    #[test]
    fn redirect_from_loopback_reselects_external_proxy() {
        let origin = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let origin_addr = origin.server_addr().to_ip().unwrap();
        let proxy = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.server_addr().to_ip().unwrap();
        let origin_worker = std::thread::spawn(move || {
            let req = origin
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .unwrap();
            req.respond(
                tiny_http::Response::empty(302).with_header(
                    tiny_http::Header::from_bytes("Location", "http://updates.invalid/update.json")
                        .unwrap(),
                ),
            )
            .unwrap();
        });
        let proxy_worker = std::thread::spawn(move || {
            if let Some(req) = proxy
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
            {
                req.respond(tiny_http::Response::from_string(
                    r#"{"source":"external-proxy"}"#,
                ))
                .unwrap();
            }
        });
        let settings = ProxySettings {
            environment: Some(format!("http://{proxy_addr}")),
            ..Default::default()
        };
        let url = format!("http://{origin_addr}/redirect");
        let result = get_following_redirects(&url, std::time::Duration::from_secs(2), |_| {
            settings.clone()
        });
        origin_worker.join().unwrap();
        proxy_worker.join().unwrap();
        let value: serde_json::Value = result.unwrap().into_json().unwrap();
        assert_eq!(value["source"], "external-proxy");
    }

    #[test]
    fn redirect_from_proxy_to_loopback_bypasses_proxy() {
        let origin = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let origin_addr = origin.server_addr().to_ip().unwrap();
        let proxy = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.server_addr().to_ip().unwrap();
        let origin_worker = std::thread::spawn(move || {
            if let Some(req) = origin
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
            {
                req.respond(tiny_http::Response::from_string(
                    r#"{"source":"direct-origin"}"#,
                ))
                .unwrap();
            }
        });
        let proxy_worker = std::thread::spawn(move || {
            let req = proxy
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .unwrap();
            req.respond(
                tiny_http::Response::empty(302).with_header(
                    tiny_http::Header::from_bytes(
                        "Location",
                        format!("http://{origin_addr}/payload"),
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
        });
        let settings = ProxySettings {
            environment: Some(format!("http://{proxy_addr}")),
            ..Default::default()
        };
        let result = get_following_redirects(
            "http://updates.invalid/update.json",
            std::time::Duration::from_secs(2),
            |_| settings.clone(),
        );
        origin_worker.join().unwrap();
        proxy_worker.join().unwrap();
        let value: serde_json::Value = result.unwrap().into_json().unwrap();
        assert_eq!(value["source"], "direct-origin");
    }

    #[test]
    fn relative_redirect_loop_is_bounded() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..11 {
                let req = server
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap()
                    .unwrap();
                req.respond(
                    tiny_http::Response::empty(302)
                        .with_header(tiny_http::Header::from_bytes("Location", "/loop").unwrap()),
                )
                .unwrap();
            }
        });
        let result = get_following_redirects(
            &format!("http://{address}/loop"),
            std::time::Duration::from_secs(2),
            |_| ProxySettings::default(),
        );
        worker.join().unwrap();
        assert!(result.unwrap_err().contains("Θçìσ«ÜσÉæµ¼íµò░"));
    }
}
