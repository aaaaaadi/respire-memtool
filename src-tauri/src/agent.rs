//! agent — 内容梳理（P10 层次二：调本机 agent 的 headless 入口）
//!
//! 与层次一（`processor` 直调 LLM）的区别：这里把内容交给一个**真实 agent**
//! （opencode / Qoder），它可能带工具与 skill，适合需要「按 agent 的方式」处理内容的场景。
//!
//! 调用方式：**子进程 + 参数数组**（不拼接 shell 字符串，避免命令注入），
//! 因此红线仍然成立：不启停 rsrs 服务、不使用 `--direct`。
//!
//! 已核实入口（2026-10-21，本机实测）：
//! - opencode：`opencode run --format json [-m provider/model] [--agent a] [--attach url] <prompt>`
//!   输出 NDJSON 事件流，取最后一条 message 里的 text 段落拼成回答。
//! - Qoder：`qodercli -p <prompt> -f json -q [--max-turns n] [--model m] [--allowed-tools t]`
//!   输出 JSON（容错解析 result / content / text…），未登录会明确报错。
//!
//! 默认超时 240s（agent 冷启动比直连模型慢得多），超时杀进程并回传明确错误。

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::AgentConfig;

/// 支持的 agent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OpenCode,
    Qoder,
}

impl Kind {
    /// 解析 agent 名（大小写不敏感）；未知返回 None
    pub fn parse(raw: &str) -> Option<Kind> {
        match raw.trim().to_lowercase().as_str() {
            "opencode" | "open-code" => Some(Kind::OpenCode),
            "qoder" | "qodercli" => Some(Kind::Qoder),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::OpenCode => "opencode",
            Kind::Qoder => "qoder",
        }
    }

    /// PATH 上的默认可执行文件名
    fn default_bin(self) -> &'static str {
        match self {
            Kind::OpenCode => "opencode",
            Kind::Qoder => "qodercli",
        }
    }
}

/// 一次 agent 调用的结果
#[derive(Debug, Clone)]
pub struct Reply {
    /// 回答正文（已 trim）
    pub text: String,
    /// 实际使用的 agent
    pub kind: Kind,
    /// 实际使用的可执行文件
    pub bin: String,
    /// 耗时（毫秒）
    pub elapsed_ms: u128,
}

/// 执行一次梳理调用。
///
/// `prompt` 为完整提示词（system 指令 + 具体任务 + 正文），由调用方组装。
pub fn ask(cfg: &AgentConfig, prompt: &str) -> Result<Reply, String> {
    ask_with(cfg, prompt, "")
}

/// 同 [`ask`]，可带 agent 服务地址（opencode 的 `--attach`；qoder 忽略）
pub fn ask_with(cfg: &AgentConfig, prompt: &str, endpoint: &str) -> Result<Reply, String> {
    if prompt.trim().is_empty() {
        return Err("内容为空".into());
    }
    let kind = Kind::parse(&cfg.kind)
        .ok_or_else(|| format!("未知 agent 类型：{}（可选 opencode / qoder）", cfg.kind.trim()))?;
    let bin = resolve_bin(kind, cfg)?;
    // .cmd/.bat 包装器不允许带换行/引号参数（Rust 的 CVE-2024-24576 防护），
    // 此时把提示词压平成单行；正常情况会优先解析出真正的 .exe，不会走到这里。
    let prompt = if is_batch(&bin) {
        prompt.replace(['\r', '\n', '"'], " ")
    } else {
        prompt.to_string()
    };
    let args = build_args_with_endpoint(kind, cfg, &prompt, endpoint);

    let mut cmd = Command::new(&bin);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // 工作目录：默认临时目录，避免 agent 把 memTools 自己的仓库当项目（会读 AGENTS.md、跑 hook）
    let cwd = workdir(cfg);
    std::fs::create_dir_all(&cwd)
        .map_err(|e| format!("无法创建 agent 工作目录 {}：{e}", cwd.display()))?;
    cmd.current_dir(&cwd);

    // 令牌：只在配置里显式给了才注入（不写入日志、不进命令行）
    let token = cfg.token.trim();
    if !token.is_empty() {
        cmd.env(cfg.token_env_name(), token);
    }
    // 关掉应答式交互（env 层面再兜一层）
    cmd.env("CI", "1");

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：不弹出控制台黑窗
        cmd.creation_flags(0x0800_0000);
    }

    let started = Instant::now();
    let (code, stdout, stderr) = run(cmd, Duration::from_secs(cfg.timeout_secs.max(10)), kind)?;
    let elapsed_ms = started.elapsed().as_millis();

    if code != 0 {
        if let Some(hint) = auth_hint(kind, &stderr, &stdout) {
            return Err(hint);
        }
        return Err(format!(
            "{} 执行失败（退出码 {code}）：{}",
            kind.as_str(),
            tail(&stderr, 400)
        ));
    }

    let text = match kind {
        Kind::OpenCode => parse_opencode(&stdout),
        Kind::Qoder => parse_qoder(&stdout),
    }?;

    Ok(Reply {
        text,
        kind,
        bin: bin.display().to_string(),
        elapsed_ms,
    })
}

/// 探测 agent 可用性（`--version`），供 /agent 与排查使用
pub fn probe(cfg: &AgentConfig) -> Result<(Kind, String, String), String> {
    let kind = Kind::parse(&cfg.kind)
        .ok_or_else(|| format!("未知 agent 类型：{}（可选 opencode / qoder）", cfg.kind.trim()))?;
    let bin = resolve_bin(kind, cfg)?;
    let mut cmd = Command::new(&bin);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let (code, stdout, stderr) = run(cmd, Duration::from_secs(15), kind)?;
    let out = if code == 0 { stdout } else { stderr };
    Ok((kind, bin.display().to_string(), tail(&out, 120)))
}

// ---------------------------------------------------------------------------
// 命令行参数（纯函数，便于单测）
// ---------------------------------------------------------------------------

/// 构建参数数组（不含可执行文件本身）
pub fn build_args(kind: Kind, cfg: &AgentConfig, prompt: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    match kind {
        Kind::OpenCode => {
            args.push("run".into());
            args.push("--format".into());
            args.push("json".into());
            if cfg.pure {
                args.push("--pure".into());
            }
            push_opt(&mut args, "-m", &cfg.model);
            push_opt(&mut args, "--agent", &cfg.agent);
            args.extend(cfg.extra_args.iter().cloned());
            // prompt 作为位置参数放最后（--attach 由 build_args_with_endpoint 插到它前面）
            args.push(prompt.to_string());
        }
        Kind::Qoder => {
            args.push("-p".into());
            args.push(prompt.to_string());
            args.push("-f".into());
            args.push("json".into());
            args.push("-q".into());
            if cfg.max_turns > 0 {
                push_opt(&mut args, "--max-turns", &cfg.max_turns.to_string());
            }
            push_opt(&mut args, "--model", &cfg.model);
            push_opt(&mut args, "--allowed-tools", &cfg.allowed_tools);
            push_opt(&mut args, "--disallowed-tools", &cfg.disallowed_tools);
            args.extend(cfg.extra_args.iter().cloned());
        }
    }
    args
}

/// 同 [`build_args`]，额外带上 `[processor] agent_endpoint`（opencode 的 `--attach`）
pub fn build_args_with_endpoint(
    kind: Kind,
    cfg: &AgentConfig,
    prompt: &str,
    endpoint: &str,
) -> Vec<String> {
    let mut args = build_args(kind, cfg, prompt);
    let url = endpoint.trim();
    if kind == Kind::OpenCode && !url.is_empty() {
        // opencode run 的 --attach 需在位置参数（prompt）之前
        let pos = args.len().saturating_sub(1);
        args.insert(pos, "--attach".into());
        args.insert(pos + 1, url.to_string());
    }
    args
}

fn push_opt(args: &mut Vec<String>, flag: &str, value: &str) {
    let v = value.trim();
    if !v.is_empty() {
        args.push(flag.into());
        args.push(v.to_string());
    }
}

// ---------------------------------------------------------------------------
// 可执行文件探测
// ---------------------------------------------------------------------------

/// 解析可执行文件：配置 > 环境变量 > PATH > 已知安装位置
pub fn resolve_bin(kind: Kind, cfg: &AgentConfig) -> Result<PathBuf, String> {
    let custom = cfg.bin.trim();
    if !custom.is_empty() {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Ok(prefer_exe(p));
        }
        // 允许只写文件名（走 PATH）
        if !custom.contains(['\\', '/']) {
            if let Some(found) = which(custom) {
                return Ok(prefer_exe(found));
            }
        }
        return Err(format!(
            "找不到 agent 可执行文件：{custom}。请在 config.toml [processor.agent] bin 填完整路径。"
        ));
    }

    for cand in candidates(kind) {
        if cand.is_file() {
            return Ok(prefer_exe(cand));
        }
    }

    let name = kind.default_bin();
    if let Some(found) = which(name) {
        return Ok(prefer_exe(found));
    }
    Err(format!(
        "未找到 {}：请把它装好并加进 PATH，或在 config.toml [processor.agent] bin 指定路径（{}）。",
        name,
        hint_for(kind)
    ))
}

/// 是否是批处理包装器（Windows 的 npm 全局命令多为 .cmd）
fn is_batch(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .as_deref(),
        Some("cmd") | Some("bat")
    )
}

/// 把 `.cmd/.bat` 包装器换成同包真正的 `.exe`（能带换行/引号参数，且无控制台包装开销）。
///
/// npm 全局命令结构：`<prefix>/opencode.cmd` + `<prefix>/node_modules/<pkg>/bin/<name>.exe`。
fn prefer_exe(path: PathBuf) -> PathBuf {
    if !is_batch(&path) {
        return path;
    }
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem()) else {
        return path;
    };
    let stem = stem.to_string_lossy().to_string();

    // ① <dir>/node_modules/<任意包>/bin/<stem>.exe
    if let Ok(entries) = std::fs::read_dir(dir.join("node_modules")) {
        for entry in entries.flatten() {
            let cand = entry.path().join("bin").join(format!("{stem}.exe"));
            if cand.is_file() {
                return cand;
            }
        }
    }
    // ② 同目录 <stem>.exe
    let same = dir.join(format!("{stem}.exe"));
    if same.is_file() {
        return same;
    }
    path
}

/// 已知安装位置（Qoder 的 CLI 藏在 IDE 的 resources 目录里）
fn candidates(kind: Kind) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    match kind {
        Kind::OpenCode => {
            if let Some(home) = std::env::var_os("APPDATA") {
                out.push(PathBuf::from(home).join("npm").join("opencode.cmd"));
            }
            if let Some(home) = std::env::var_os("LOCALAPPDATA") {
                out.push(PathBuf::from(home).join("npm").join("opencode.cmd"));
            }
        }
        Kind::Qoder => {
            let tail = PathBuf::from("resources")
                .join("app")
                .join("resources")
                .join("bin")
                .join("x86_64_windows")
                .join("qodercli.exe");
            // ① 由 PATH 上的 qoder 启动器反推 IDE 根目录
            if let Some(launcher) = which("qoder") {
                // <root>/bin/qoder.cmd → <root>/resources/app/resources/bin/x86_64_windows/qodercli.exe
                if let Some(root) = launcher.parent().and_then(|p| p.parent()) {
                    out.push(root.join(&tail));
                }
            }
            // ② 常见安装位置
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                out.push(PathBuf::from(&local).join("Programs").join("Qoder").join(&tail));
            }
            for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                if let Some(pf) = std::env::var_os(var) {
                    out.push(PathBuf::from(pf).join("Qoder").join(&tail));
                }
            }
        }
    }
    out
}

fn hint_for(kind: Kind) -> &'static str {
    match kind {
        Kind::OpenCode => "npm i -g opencode-ai 后 opencode 应在 PATH 上",
        Kind::Qoder => "Qoder 的 qodercli 通常在 <Qoder 安装目录>/resources/app/resources/bin/x86_64_windows/ 下",
    }
}

/// PATH 查找（Windows 按可执行扩展名优先，避免命中同名的 bash/ps1 脚本）
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    // 只接受 CreateProcess 真能执行的扩展名（.ps1 / .sh / 无扩展名都会报 os error 193）
    let exts: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if cfg!(windows) {
            for ext in exts {
                let cand = dir.join(format!("{name}{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
            let direct = dir.join(name);
            let lower = name.to_lowercase();
            if direct.is_file() && exts.iter().any(|e| lower.ends_with(e)) {
                return Some(direct);
            }
        } else {
            let direct = dir.join(name);
            if direct.is_file() {
                return Some(direct);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 进程执行（带超时）
// ---------------------------------------------------------------------------

fn workdir(cfg: &AgentConfig) -> PathBuf {
    let custom = cfg.workdir.trim();
    if !custom.is_empty() {
        return PathBuf::from(custom);
    }
    std::env::temp_dir().join("memtools-agent")
}

/// 跑子进程并收集输出；超时杀进程并回明确错误
fn run(mut cmd: Command, timeout: Duration, kind: Kind) -> Result<(i32, String, String), String> {
    let mut child = cmd.spawn().map_err(|e| {
        format!(
            "无法启动 {}：{e}（请检查 config.toml [processor.agent] bin / PATH）",
            kind.as_str()
        )
    })?;

    let (tx_out, rx_out) = mpsc::channel::<String>();
    let (tx_err, rx_err) = mpsc::channel::<String>();
    if let Some(mut out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = out.read_to_string(&mut buf);
            let _ = tx_out.send(buf);
        });
    }
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = err.read_to_string(&mut buf);
            let _ = tx_err.send(buf);
        });
    }

    let deadline = Instant::now() + timeout;
    let mut killed = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                return Err(format!("等待 {} 退出失败：{e}", kind.as_str()));
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            killed = true;
            break -1;
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let stdout = rx_out.recv_timeout(Duration::from_secs(3)).unwrap_or_default();
    let stderr = rx_err.recv_timeout(Duration::from_secs(3)).unwrap_or_default();

    if killed {
        return Err(format!(
            "{} 调用超时（{}s）：已终止。可在 config.toml [processor.agent] 增大 timeout_secs，或换更小的 model。",
            kind.as_str(),
            timeout.as_secs()
        ));
    }
    Ok((code, stdout, stderr))
}

/// 取末尾 N 个字符（错误回显用，避免整段日志灌进 UI）
fn tail(s: &str, n: usize) -> String {
    let t = s.trim();
    let count = t.chars().count();
    if count <= n {
        return t.to_string();
    }
    t.chars().skip(count - n).collect::<String>()
}

/// 鉴权类失败 → 可操作的提示（不回传整段日志，避免泄漏令牌）
fn auth_hint(kind: Kind, stderr: &str, stdout: &str) -> Option<String> {
    let raw = format!("{stderr}\n{stdout}").to_lowercase();
    let hit = raw.contains("not logged in")
        || raw.contains("please login")
        || raw.contains("unauthorized")
        || raw.contains("no api key")
        || raw.contains("api key is not");
    if !hit {
        return None;
    }
    Some(match kind {
        Kind::Qoder => "Qoder 未登录：先运行 `qodercli /login`（或设置 QODER_PERSONAL_ACCESS_TOKEN，\
也可写在 config.toml [processor.agent] token）。"
            .into(),
        Kind::OpenCode => "opencode 未配置模型鉴权：请确认 opencode 的 provider/apiKey 可用，\
并在 [processor.agent] model 填 provider/model（如 opencode/big-pickle）。"
            .into(),
    })
}

// ---------------------------------------------------------------------------
// 输出解析
// ---------------------------------------------------------------------------

/// opencode `--format json`：NDJSON 事件流，取最后一条 message 的 text 段落。
///
/// 若 stdout 不是事件流（例如没传 --format json），退化为「去掉 `> build · model` 头部后的原文」。
pub fn parse_opencode(stdout: &str) -> Result<String, String> {
    // (messageID, 文本) 按出现顺序累积
    let mut groups: Vec<(String, String)> = Vec::new();
    let mut saw_event = false;

    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        let Some(part) = v.get("part") else { continue };
        let Some(text) = part.get("text").and_then(|t| t.as_str()) else {
            continue;
        };
        saw_event = true;
        let mid = part
            .get("messageID")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        match groups.last_mut() {
            Some((id, acc)) if *id == mid => acc.push_str(text),
            _ => groups.push((mid, text.to_string())),
        }
    }

    if saw_event {
        let last = groups
            .last()
            .map(|(_, t)| t.trim().to_string())
            .unwrap_or_default();
        if last.is_empty() {
            return Err("opencode 未返回文本内容".into());
        }
        return Ok(last);
    }

    // 退化路径：默认格式输出，第一行是 `> agent · model`
    let text = stdout
        .lines()
        .skip_while(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with('>')
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("opencode 未返回文本内容".into());
    }
    Ok(text)
}

/// Qoder `-f json`：单个 JSON 对象（或 NDJSON），取 result / content / text 等字段
pub fn parse_qoder(stdout: &str) -> Result<String, String> {
    let raw = stdout.trim();
    if raw.is_empty() {
        return Err("qodercli 未返回内容".into());
    }

    if let Ok(v) = serde_json::from_str::<Value>(raw) {
        if let Some(t) = extract_text(&v, 0) {
            return Ok(t);
        }
    }

    // NDJSON：逐行找 result 类事件
    let mut last: Option<String> = None;
    for line in raw.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            if let Some(t) = extract_text(&v, 0) {
                last = Some(t);
            }
        }
    }
    if let Some(t) = last {
        return Ok(t);
    }

    // 非 JSON（例如未加 -f json）：直接用原文
    if raw.starts_with('{') {
        return Err("qodercli 输出无法解析为文本".into());
    }
    Ok(raw.to_string())
}

/// 从 JSON 里取正文：优先常见字段，其次深度优先找第一个非空字符串
fn extract_text(v: &Value, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match v {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Value::Array(items) => {
            for item in items.iter().rev() {
                if let Some(t) = extract_text(item, depth + 1) {
                    return Some(t);
                }
            }
            None
        }
        Value::Object(map) => {
            for key in ["result", "response", "content", "text", "message", "output", "answer"] {
                if let Some(found) = map.get(key) {
                    if let Some(t) = extract_text(found, depth + 1) {
                        return Some(t);
                    }
                }
            }
            // 兜底：跳过明显的元数据字段
            for (k, val) in map {
                if matches!(
                    k.as_str(),
                    "type" | "subtype" | "id" | "session_id" | "sessionID" | "uuid" | "model"
                ) {
                    continue;
                }
                if let Some(t) = extract_text(val, depth + 1) {
                    return Some(t);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(kind: &str) -> AgentConfig {
        AgentConfig {
            kind: kind.into(),
            ..Default::default()
        }
    }

    #[test]
    fn kind_parse_accepts_aliases() {
        assert_eq!(Kind::parse("opencode"), Some(Kind::OpenCode));
        assert_eq!(Kind::parse("OpenCode"), Some(Kind::OpenCode));
        assert_eq!(Kind::parse(" qoder "), Some(Kind::Qoder));
        assert_eq!(Kind::parse("qodercli"), Some(Kind::Qoder));
        assert_eq!(Kind::parse("claude"), None);
        assert_eq!(Kind::parse(""), None);
    }

    #[test]
    fn opencode_args_carry_prompt_and_options() {
        let mut c = cfg("opencode");
        c.model = "opencode/big-pickle".into();
        c.agent = "processor".into();
        let args = build_args(Kind::OpenCode, &c, "总结这段");
        assert_eq!(args[0], "run");
        assert!(args.contains(&"--format".to_string()));
        assert!(args.contains(&"--pure".to_string()));
        assert!(args.windows(2).any(|w| w[0] == "-m" && w[1] == "opencode/big-pickle"));
        assert!(args.windows(2).any(|w| w[0] == "--agent" && w[1] == "processor"));
        assert_eq!(args.last().unwrap(), "总结这段");
        // 空 model/agent 不产生空参数
        let plain = build_args(Kind::OpenCode, &cfg("opencode"), "x");
        assert!(!plain.iter().any(|a| a.is_empty()));
        assert!(!plain.contains(&"-m".to_string()));
    }

    #[test]
    fn opencode_args_attach_endpoint_before_prompt() {
        let c = cfg("opencode");
        let args = build_args_with_endpoint(Kind::OpenCode, &c, "正文", "http://127.0.0.1:4096");
        let attach = args.iter().position(|a| a == "--attach").expect("应带 --attach");
        assert_eq!(args[attach + 1], "http://127.0.0.1:4096");
        assert_eq!(args.last().unwrap(), "正文");
        assert!(attach < args.len() - 1);
        // qoder 忽略 endpoint
        let q = build_args_with_endpoint(Kind::Qoder, &cfg("qoder"), "正文", "http://x");
        assert!(!q.contains(&"--attach".to_string()));
    }

    #[test]
    fn qoder_args_print_json_and_limits() {
        let mut c = cfg("qoder");
        c.max_turns = 3;
        c.model = "efficient".into();
        c.disallowed_tools = "Bash,Edit".into();
        let args = build_args(Kind::Qoder, &c, "你好");
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "你好");
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "json"));
        assert!(args.windows(2).any(|w| w[0] == "--max-turns" && w[1] == "3"));
        assert!(args.windows(2).any(|w| w[0] == "--model" && w[1] == "efficient"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--disallowed-tools" && w[1] == "Bash,Edit"));
        // max_turns=0 时不传
        let plain = build_args(Kind::Qoder, &cfg("qoder"), "x");
        assert!(!plain.contains(&"--max-turns".to_string()));
    }

    #[test]
    fn opencode_parse_takes_last_message_group() {
        let stdout = concat!(
            r#"{"type":"text","part":{"messageID":"m1","type":"text","text":"我先看看"}}"#,
            "\n",
            r#"{"type":"tool_use","part":{"tool":"bash"}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m2","type":"text","text":"【前因】"}}"#,
            "\n",
            r#"{"type":"text","part":{"messageID":"m2","type":"text","text":"【行为】"}}"#,
            "\n"
        );
        assert_eq!(parse_opencode(stdout).unwrap(), "【前因】【行为】");
    }

    #[test]
    fn opencode_parse_falls_back_to_plain_text() {
        let stdout = "\n> build · big-pickle\n你好\n";
        assert_eq!(parse_opencode(stdout).unwrap(), "你好");
        assert!(parse_opencode("   ").is_err());
    }

    #[test]
    fn qoder_parse_reads_result_field() {
        let stdout = r#"{"type":"result","subtype":"success","result":"整理后的正文","session_id":"s1"}"#;
        assert_eq!(parse_qoder(stdout).unwrap(), "整理后的正文");
    }

    #[test]
    fn qoder_parse_handles_nested_and_plain() {
        let nested = r#"{"data":{"response":{"content":"要点一"}}}"#;
        assert_eq!(parse_qoder(nested).unwrap(), "要点一");
        assert_eq!(parse_qoder("纯文本回答").unwrap(), "纯文本回答");
        assert!(parse_qoder("  ").is_err());
    }

    #[test]
    fn token_env_defaults_by_kind() {
        assert_eq!(cfg("qoder").token_env_name(), "QODER_PERSONAL_ACCESS_TOKEN");
        assert_eq!(cfg("opencode").token_env_name(), "OPENCODE_API_KEY");
        let mut c = cfg("qoder");
        c.token_env = "MY_TOKEN".into();
        assert_eq!(c.token_env_name(), "MY_TOKEN");
    }

    #[test]
    fn resolve_bin_reports_missing_custom_path() {
        let mut c = cfg("qoder");
        c.bin = r"D:\definitely\missing\qodercli.exe".into();
        let err = resolve_bin(Kind::Qoder, &c).unwrap_err();
        assert!(err.contains("找不到 agent 可执行文件"));
    }

    #[test]
    fn extract_text_skips_metadata() {
        let v: Value =
            serde_json::from_str(r#"{"session_id":"abc","type":"result","result":"正文"}"#).unwrap();
        assert_eq!(extract_text(&v, 0).unwrap(), "正文");
        let none: Value = serde_json::from_str(r#"{"session_id":"abc","type":"result"}"#).unwrap();
        assert!(extract_text(&none, 0).is_none());
    }
}
