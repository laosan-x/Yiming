//! 通知模块 —— 系统级通知 + 免打扰（DND）判定
//!
//! - 系统通知走 `notify-rust`（Windows 用 WinRT toast，无需权限弹窗；
//!   是否显示由系统的「专注助手」与通知设置决定，应用侧只需发出）
//! - 免打扰时段内系统通知静默（直接不发）；应用内消息中心照常记录
//! - 时段解析支持跨零点（如 22:00 → 08:00）

use crate::core::config::DndConfig;

/// 当前本地时间是否处于免打扰时段
pub fn in_dnd(cfg: &DndConfig) -> bool {
    if !cfg.enabled {
        return false;
    }
    let (sh, sm) = match parse_hhmm(&cfg.start) {
        Some(v) => v,
        None => return false,
    };
    let (eh, em) = match parse_hhmm(&cfg.end) {
        Some(v) => v,
        None => return false,
    };
    let now = local_hhmm();
    window_contains(now.0 * 60 + now.1, sh * 60 + sm, eh * 60 + em)
}

/// 纯窗口判定（可测）：now 是否落在 [start, end) 内（支持跨零点）
fn window_contains(now_m: u32, start_m: u32, end_m: u32) -> bool {
    if start_m == end_m {
        return false;
    }
    if start_m < end_m {
        now_m >= start_m && now_m < end_m
    } else {
        // 跨零点：22:00 → 08:00 = now >= 22:00 或 now < 08:00
        now_m >= start_m || now_m < end_m
    }
}

/// 发系统通知（免打扰时段内静默跳过；失败仅打日志不影响主流程）
pub fn system_notify(title: &str, body: &str, cfg: &DndConfig) {
    if in_dnd(cfg) {
        eprintln!("[notify] 免打扰时段，静默：{title}");
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .appname("壹鸣 Wonderland")
            .show();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = notify_rust::Notification::new().summary(title).body(body).show();
    }
}

/// 当前本地时间 (时, 分)（Windows GetLocalTime，无时区换算）
#[cfg(windows)]
fn local_hhmm() -> (u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY：无副作用的标准系统调用
    unsafe {
        let st = GetLocalTime();
        (st.wHour as u32, st.wMinute as u32)
    }
}

#[cfg(not(windows))]
fn local_hhmm() -> (u32, u32) {
    // 非 Windows：UTC 兜底（项目实际只面向 Windows）
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() % 86400;
    ((secs / 3600) as u32, ((secs % 3600) / 60) as u32)
}

/// 解析 `HH:MM` 为 (时, 分)
fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hhmm_ok_and_reject() {
        assert_eq!(parse_hhmm("22:00"), Some((22, 0)));
        assert_eq!(parse_hhmm(" 8:5 "), Some((8, 5)));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("abc"), None);
    }

    #[test]
    fn dnd_window_including_midnight_crossing() {
        // 纯窗口判定（不依赖当前时间）
        assert!(window_contains(420, 1320, 480)); // 07:00 in 22:00→08:00（跨零点，后半夜）
        assert!(window_contains(1380, 1320, 480)); // 23:00 in 22:00→08:00
        assert!(!window_contains(900, 1320, 480)); // 15:00 not in
        assert!(window_contains(540, 540, 720)); // 边界：09:00 in 09:00→12:00
        assert!(!window_contains(720, 540, 720)); // 12:00 不在 [09:00,12:00)
        assert!(!window_contains(600, 480, 480)); // 起止相等 = 不启用
    }
}
