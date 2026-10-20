//! reminders — task 到期提醒（P5）
//!
//! rsrs CLI 没有后台提醒：本模块在 UI 侧定时扫描含 `【触发】` 的条目，
//! 解析其中已核实的 `YYYY-MM-DD` 日期，给出「逾期 / 今天 / 即将 / 之后」分组。
//! 只读 memory_list，不写 rsrs 数据目录；「忽略」状态落在应用配置目录。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::rsrs_client::RsrsClient;

#[derive(Debug, Clone, Serialize)]
pub struct ReminderItem {
    pub id: String,
    pub title: String,
    pub content: String,
    /// 触发日期（YYYY-MM-DD）
    pub due: String,
    /// 相对今天的天数：<0 逾期、0 今天、>0 未来
    pub days: i64,
    /// overdue | today | soon | later
    pub status: String,
    /// 【触发】行原文（去掉标记）
    pub trigger: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReminderOut {
    pub items: Vec<ReminderItem>,
    pub overdue: u32,
    pub today: u32,
    pub soon: u32,
    /// 实际扫描的最近条数
    pub scanned: u32,
    /// 扫描到上限仍未覆盖全部条目（更早的 task 可能未纳入）
    pub truncated: bool,
    pub checked_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DismissedEntry {
    pub due: String,
    pub at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DismissedFile {
    #[serde(default)]
    pub dismissed: BTreeMap<String, DismissedEntry>,
}

pub const SCAN_START: u32 = 200;
pub const SCAN_MAX: u32 = 1600;

/// 完成标记：出现在任意行首即视为已完成（扫描跳过，不再提醒）
pub const DONE_MARKERS: &[&str] = &["【完成】", "【已完成】"];

pub fn is_completed(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        DONE_MARKERS.iter().any(|m| line.starts_with(m))
    })
}

/// 追加完成行的内容；已带完成标记则返回 None
pub fn with_completion(content: &str, today: &str) -> Option<String> {
    if is_completed(content) {
        return None;
    }
    let mut out = content.trim_end().to_string();
    out.push_str(&format!("\n【完成】{today}"));
    Some(out)
}

/// 从正文中取【触发】行与其中的 YYYY-MM-DD；没有标记或没有日期则视为不可核实。
pub fn parse_trigger(content: &str) -> Option<(String, String)> {
    const NO_DATE: &[&str] = &[
        "无日期",
        "无明确日期",
        "无具体日期",
        "待定",
        "待确认",
        "看情况",
        "再议",
    ];
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with("【触发】") {
            continue;
        }
        let raw = line.trim_start_matches("【触发】").trim().to_string();
        if NO_DATE.iter().any(|m| raw.contains(m)) {
            continue;
        }
        if let Some(date) = find_date(&raw) {
            return Some((date, raw));
        }
    }
    None
}

/// 找第一个形如 YYYY-MM-DD 的子串（月/日必须两位且范围合法）
pub fn find_date(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len().saturating_sub(9) {
        let w = &chars[i..i + 10];
        if !(w[0..4].iter().all(|c| c.is_ascii_digit())
            && w[4] == '-'
            && w[5..7].iter().all(|c| c.is_ascii_digit())
            && w[7] == '-'
            && w[8..10].iter().all(|c| c.is_ascii_digit()))
        {
            continue;
        }
        let y: i64 = w[0..4].iter().collect::<String>().parse().ok()?;
        let m: i64 = w[5..7].iter().collect::<String>().parse().ok()?;
        let d: i64 = w[8..10].iter().collect::<String>().parse().ok()?;
        if (1..=12).contains(&m) && (1..=31).contains(&d) && (1970..=9999).contains(&y) {
            return Some(w.iter().collect());
        }
    }
    None
}

/// Howard Hinnant 的 days_from_civil（1970-01-01 起的天数）
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let b: Vec<&str> = s.split('-').collect();
    if b.len() != 3 {
        return None;
    }
    Some((b[0].parse().ok()?, b[1].parse().ok()?, b[2].parse().ok()?))
}

pub fn ymd_to_days(s: &str) -> Option<i64> {
    let (y, m, d) = parse_ymd(s)?;
    Some(days_from_civil(y, m, d))
}

/// 本地日期（YYYY-MM-DD）
pub fn today_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// days → YYYY-MM-DD（减少对系统时区的依赖）
pub fn civil_from_days(z: i64) -> String {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn classify(days: i64, soon_days: i64) -> &'static str {
    if days < 0 {
        "overdue"
    } else if days == 0 {
        "today"
    } else if days <= soon_days.max(0) {
        "soon"
    } else {
        "later"
    }
}

pub fn dismiss_path(cache_dir: &Path) -> std::path::PathBuf {
    cache_dir.join("reminders.json")
}

pub fn load_dismissed(cache_dir: &Path) -> DismissedFile {
    let path = dismiss_path(cache_dir);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<DismissedFile>(&t).ok())
        .unwrap_or_default()
}

pub fn save_dismissed(cache_dir: &Path, file: &DismissedFile) -> Result<(), String> {
    std::fs::create_dir_all(cache_dir).map_err(|e| format!("创建提醒缓存目录失败: {e}"))?;
    let text = serde_json::to_string_pretty(file)
        .map_err(|e| format!("序列化提醒缓存失败: {e}"))?;
    std::fs::write(dismiss_path(cache_dir), text)
        .map_err(|e| format!("写入提醒缓存失败: {e}"))
}

/// 忽略某条提醒；`due` 变化后会自动重新出现（避免旧忽略掩盖新日期）
pub fn dismiss(cache_dir: &Path, id: &str, due: &str, now: &str) -> Result<(), String> {
    let mut file = load_dismissed(cache_dir);
    file.dismissed.insert(
        id.to_string(),
        DismissedEntry {
            due: due.to_string(),
            at: now.to_string(),
        },
    );
    save_dismissed(cache_dir, &file)
}

/// 扫描最近条目里的 task（含【触发】且日期可核实）
pub fn scan(client: &RsrsClient, soon_days: i64, cache_dir: &Path) -> Result<ReminderOut, String> {
    let today = today_local();
    let today_days = ymd_to_days(&today).unwrap_or(0);

    let mut seen: BTreeMap<String, (String, String, String, String)> = BTreeMap::new();
    let mut window = SCAN_START;
    let mut scanned = 0u32;
    let mut truncated = false;
    loop {
        let items = client
            .list_recent(window)
            .map_err(|e| format!("读取最近记忆失败: {e}"))?;
        let got = items.len() as u32;
        scanned = scanned.max(got);
        for it in items {
            if seen.contains_key(&it.id) {
                continue;
            }
            if it.importance.as_deref() == Some("trivial") {
                continue;
            }
            if is_completed(&it.content) {
                continue;
            }
            if let Some((due, trigger)) = parse_trigger(&it.content) {
                seen.insert(it.id, (due, it.title, trigger, it.content));
            }
        }
        if got < window {
            break;
        }
        if window >= SCAN_MAX {
            truncated = true;
            break;
        }
        window = (window * 2).min(SCAN_MAX);
    }

    let dismissed = load_dismissed(cache_dir);
    let mut items = Vec::new();
    for (id, (due, title, trigger, content)) in seen {
        if let Some(d) = dismissed.dismissed.get(&id) {
            if d.due == due {
                continue;
            }
        }
        let Some(days_abs) = ymd_to_days(&due) else {
            continue;
        };
        let days = days_abs - today_days;
        items.push(ReminderItem {
            content,
            id,
            title,
            due,
            days,
            status: classify(days, soon_days).to_string(),
            trigger,
        });
    }
    items.sort_by(|a, b| a.days.cmp(&b.days).then_with(|| a.title.cmp(&b.title)));

    let overdue = items.iter().filter(|i| i.status == "overdue").count() as u32;
    let today_n = items.iter().filter(|i| i.status == "today").count() as u32;
    let soon = items.iter().filter(|i| i.status == "soon").count() as u32;
    Ok(ReminderOut {
        items,
        overdue,
        today: today_n,
        soon,
        scanned,
        truncated,
        checked_at: today,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trigger_reads_date() {
        let c = "【前因】x\n【行为】y\n【后果】z\n【触发】2026-10-20 10:00 交付";
        let (due, raw) = parse_trigger(c).expect("should parse");
        assert_eq!(due, "2026-10-20");
        assert!(raw.starts_with("2026-10-20"));
    }

    #[test]
    fn parse_trigger_rejects_relative() {
        assert!(parse_trigger("【触发】下周找时间").is_none());
        assert!(parse_trigger("普通内容 2026-10-20").is_none());
        assert!(parse_trigger("【触发】无日期条件，本条为 2026-10-11 会话快照").is_none());
    }

    #[test]
    fn date_math_matches_known_days() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 20) - days_from_civil(2026, 10, 19), 1);
        assert_eq!(civil_from_days(0), "1970-01-01");
    }

    #[test]
    fn status_classification() {
        assert_eq!(classify(-3, 7), "overdue");
        assert_eq!(classify(0, 7), "today");
        assert_eq!(classify(3, 7), "soon");
        assert_eq!(classify(30, 7), "later");
    }

    #[test]
    fn completed_marker_is_detected() {
        assert!(!is_completed("【触发】2026-10-20"));
        assert!(!is_completed("完成情况见【触发】行"));
        assert!(is_completed("【行为】x\n【触发】2026-10-20\n【完成】2026-10-21"));
        assert!(is_completed("【已完成】2026-10-21"));
    }

    #[test]
    fn completion_append_is_idempotent() {
        let c = "【行为】x\n【触发】2026-10-20";
        assert_eq!(
            with_completion(c, "2026-10-21").unwrap(),
            "【行为】x\n【触发】2026-10-20\n【完成】2026-10-21"
        );
        assert!(with_completion("【完成】2026-10-20", "2026-10-21").is_none());
    }

    #[test]
    fn dismiss_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "memtools-reminder-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dismiss(&dir, "abc", "2026-10-20", "2026-10-19").unwrap();
        let f = load_dismissed(&dir);
        assert_eq!(f.dismissed.get("abc").map(|d| d.due.as_str()), Some("2026-10-20"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
