//! 自动更新：读 update.json → 比版本 → 下载并校验 SHA-256 → 由新 exe 自替换旧 exe 并重启。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub mandatory: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckOut {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub notes: String,
    pub size: u64,
    pub mandatory: bool,
}

#[derive(Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: u64,
}

pub fn check(endpoint: &str) -> Result<(CheckOut, Manifest), String> {
    let manifest = fetch_manifest(endpoint)?;
    let has_update = version_gt(&manifest.version, CURRENT_VERSION);
    Ok((
        CheckOut {
            current: CURRENT_VERSION.to_string(),
            latest: manifest.version.clone(),
            has_update,
            notes: manifest.notes.clone(),
            size: manifest.size,
            mandatory: manifest.mandatory,
        },
        manifest,
    ))
}

fn fetch_manifest(endpoint: &str) -> Result<Manifest, String> {
    if endpoint.is_empty() {
        return Err("未配置更新地址（config.toml 的 [update] endpoint）".into());
    }
    let resp = crate::update_http::get(endpoint, Duration::from_secs(8))
        .map_err(|e| format!("检查更新失败：{e}"))?;
    let text = resp
        .into_string()
        .map_err(|e| format!("读取更新清单失败：{e}"))?;
    serde_json::from_str::<Manifest>(&text).map_err(|e| format!("更新清单解析失败：{e}"))
}

pub fn install(app: &AppHandle, manifest: &Manifest) -> Result<PathBuf, String> {
    if manifest.sha256.trim().is_empty() {
        return Err("更新清单缺少 sha256：拒绝安装未哈希/未签名的更新。".into());
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        return Err(
            "自动安装仅支持 Windows：请从 GitHub Releases 手动下载新版本并替换当前程序。".into(),
        );
    }
    #[cfg(windows)]
    {
        let new_exe = download(app, manifest)?;
        let target = std::env::current_exe().map_err(|e| format!("定位当前 exe 失败：{e}"))?;
        spawn_applier(&new_exe, &target)?;
        Ok(new_exe)
    }
}

fn download(app: &AppHandle, manifest: &Manifest) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法定位配置目录：{e}"))?
        .join("update");
    fs::create_dir_all(&dir).map_err(|e| format!("创建更新目录失败：{e}"))?;
    let dest = dir.join(format!("memtools-{}.exe", manifest.version));

    let resp = crate::update_http::get(&manifest.url, Duration::from_secs(180))
        .map_err(|e| format!("下载更新失败：{e}"))?;
    let total = manifest.size.max(
        resp.header("Content-Length")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0),
    );

    let mut reader = resp.into_reader();
    let mut file = fs::File::create(&dest).map_err(|e| format!("创建更新文件失败：{e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut last = Instant::now();
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("下载中断：{e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入更新文件失败：{e}"))?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if last.elapsed() >= Duration::from_millis(150) {
            last = Instant::now();
            let _ = app.emit(
                "update://progress",
                Progress {
                    downloaded: done,
                    total,
                },
            );
        }
    }
    let _ = file.flush();
    let _ = app.emit(
        "update://progress",
        Progress {
            downloaded: done,
            total,
        },
    );

    if total > 0 && done != total {
        let _ = fs::remove_file(&dest);
        return Err(format!("下载不完整：{done} / {total} 字节"));
    }
    let digest = to_hex(&hasher.finalize());
    let expected = manifest.sha256.trim();
    if !expected.is_empty() && !digest.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(&dest);
        return Err(format!("SHA-256 校验失败：期望 {expected}，实际 {digest}"));
    }
    Ok(dest)
}

fn spawn_applier(new_exe: &Path, target: &Path) -> Result<(), String> {
    let mut cmd = Command::new(new_exe);
    cmd.arg("--apply-update").arg(target);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map_err(|e| format!("启动更新进程失败：{e}"))?;
    Ok(())
}

pub fn log_path() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    base.join("com.memtools.desktop").join("update").join("apply.log")
}

pub fn apply_update(target: &Path) -> ! {
    let log = log_path();
    if let Some(dir) = log.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let me = std::env::current_exe().unwrap_or_default();
    log_line(&log, &format!("apply {} -> {}", me.display(), target.display()));

    std::thread::sleep(Duration::from_millis(300));
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut copied = false;
    while Instant::now() < deadline {
        match fs::copy(&me, target) {
            Ok(_) => {
                copied = true;
                break;
            }
            Err(e) => log_line(&log, &format!("copy retry: {e}")),
        }
        std::thread::sleep(Duration::from_millis(400));
    }

    if copied {
        match Command::new(target).spawn() {
            Ok(_) => log_line(&log, "replaced; target started"),
            Err(e) => log_line(&log, &format!("start target failed: {e}")),
        }
    } else {
        log_line(&log, "copy failed after 60s (target locked or no permission)");
    }
    std::process::exit(if copied { 0 } else { 1 });
}

fn log_line(path: &Path, msg: &str) {
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{}] {msg}", now_iso());
    }
}

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

pub fn cleanup(app: &AppHandle) {
    let Ok(base) = app.path().app_config_dir() else {
        return;
    };
    let dir = base.join("update");
    let Ok(rd) = fs::read_dir(&dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let is_exe = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("exe"))
            .unwrap_or(false);
        if !is_exe {
            continue;
        }
        let old = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|e| e.as_secs() > 86_400)
            .unwrap_or(false);
        if old {
            let _ = fs::remove_file(&path);
        }
    }
}

pub fn version_gt(a: &str, b: &str) -> bool {
    version_tuple(a) > version_tuple(b)
}

fn version_tuple(v: &str) -> (u64, u64, u64) {
    let mut out = [0u64; 3];
    for (i, part) in v.trim().trim_start_matches('v').split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        out[i] = digits.parse().unwrap_or(0);
    }
    (out[0], out[1], out[2])
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(version_gt("0.1.1", "0.1.0"));
        assert!(version_gt("0.2.0", "0.1.9"));
        assert!(version_gt("v1.0.0", "0.9.9"));
        assert!(version_gt("0.1.0-beta", "0.0.9"));
        assert!(!version_gt("0.1.0", "0.1.0"));
        assert!(!version_gt("0.0.9", "0.1.0"));
    }

    #[test]
    fn manifest_parse() {
        let text = r#"{"version":"0.1.2","notes":"x","url":"http://127.0.0.1:17651/a.exe","sha256":"AB","size":12}"#;
        let m: Manifest = serde_json::from_str(text).unwrap();
        assert_eq!(m.version, "0.1.2");
        assert_eq!(m.size, 12);
        assert!(!m.mandatory);
    }

    #[test]
    fn hex_encode() {
        assert_eq!(to_hex(&[0x00, 0x1f, 0xff]), "001fff");
    }
}
