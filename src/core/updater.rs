//! 应用自更新：版本检测 / 更新日志拉取 / 安装包下载与校验
//!
//! 数据接口 —— 更新清单 `manifest.json`（托管在静态服务器即可，无后端要求）：
//! ```json
//! {
//!   "version": "0.4.0",
//!   "notes": "修复了……；新增……（一行一条，用 \n 分隔）",
//!   "pub_date": "2026-10-18",
//!   "mandatory": false,
//!   "platforms": {
//!     "windows-x86_64": {
//!       "url": "https://…/cocole-setup-0.4.0.exe",
//!       "size": 15234567,
//!       "sha256": "…（可选，填写则下载后强校验）"
//!     }
//!   }
//! }
//! ```
//!
//! 关键逻辑：
//! - 版本比较：手写 semver 三段式（major.minor.patch），不引第三方 semver。
//! - 检查：`ureq` 阻塞请求，包在 `tokio::task::spawn_blocking` 里，不卡 UI。
//! - 下载：后台线程流式落盘，进度经 tokio mpsc 回传给 `Task::stream` 驱动的
//!   消息流（参考 IdMessage::RenderDone 的防乱序思路——事件按序到达，无需 seq）。
//! - 清单地址可被 `COCOLE_UPDATE_URL` 环境变量覆盖（与 COCOLE_DEBUG_* 同族，
//!   本地 mock 服务器联调用，见 tools/update-server-mock.mjs）。
//! - 所有错误都转成中文 `String` 直接给 UI 展示，不向上抛 panic。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use iced::Task;

use crate::core::i18n::trf;
use crate::message::{Message, UpdaterMessage};

/* ═════════════════ 常量 ═════════════════ */

/// 正式清单地址（上线时替换为真实 CDN 地址）
pub const UPDATE_MANIFEST_URL: &str = "https://update.yiming.app/cocole-iced/manifest.json";
/// 清单不可用时的兜底下载页（浏览器打开）
pub const DOWNLOAD_PAGE_URL: &str = "https://yiming.app/cocole-iced/download";
/// 自动检查冷却期：24 小时检查一次
pub const CHECK_COOLDOWN_SECS: u64 = 24 * 3600;
/// 启动后延迟检查（错开启动高峰，不与模型扫描抢 IO）
pub const AUTO_CHECK_DELAY_MS: u64 = 3000;

/// 当前平台的清单键位（与 manifest.json 的 platforms 字段对应）
pub const PLATFORM_KEY: &str = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    "windows-x86_64"
} else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
    "windows-aarch64"
} else if cfg!(target_os = "macos") {
    "macos-aarch64"
} else {
    "linux-x86_64"
};

/* ═════════════════ 偏好持久化（挂进 Config） ═════════════════ */

/// 更新偏好（随 config.json 持久化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrefs {
    /// 启动时自动检查（默认开）
    pub auto_check: bool,
    /// 用户点「忽略此版本」记录的版本号；该版本的自动提醒不再弹窗
    pub skipped_version: Option<String>,
    /// 上次成功检查的 Unix 秒（冷却期内不重复打扰）
    pub last_check: Option<u64>,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self { auto_check: true, skipped_version: None, last_check: None }
    }
}

/* ═════════════════ 清单数据结构 ═════════════════ */

#[derive(Debug, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    /// 更新日志正文（\n 分行，UI 直接分行渲染）
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pub_date: String,
    /// 强制更新：true 时对话框不提供「忽略此版本」
    #[serde(default)]
    pub mandatory: bool,
    #[serde(default)]
    pub platforms: HashMap<String, PlatformAsset>,
}

#[derive(Debug, Deserialize)]
pub struct PlatformAsset {
    pub url: String,
    /// 文件字节数（用于下载进度百分比；缺失时进度条显示为不定态）
    #[serde(default)]
    pub size: Option<u64>,
    /// 安装包 SHA-256（十六进制小写；填写则下载完成后强校验）
    #[serde(default)]
    pub sha256: Option<String>,
}

/// 给 UI 层使用的「新版本」信息（从清单中摘出当前平台条目后的结果）
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub notes: String,
    pub pub_date: String,
    pub mandatory: bool,
    pub download_url: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

/// 检查结果（进 Message，需 Clone）
#[derive(Debug, Clone)]
pub enum CheckOutcome {
    UpToDate,
    Update(Box<ReleaseInfo>),
}

/* ═════════════════ 错误 ═════════════════ */

#[derive(Debug, thiserror::Error)]
pub enum UpdaterError {
    #[error("网络请求失败：{0}")]
    Network(String),
    #[error("更新清单解析失败：{0}")]
    Parse(String),
    #[error("清单未提供当前平台（{0}）的安装包")]
    NoPlatformAsset(&'static str),
    #[error("安装包校验失败（SHA-256 不匹配），已丢弃")]
    HashMismatch,
}

/* ═════════════════ 版本号 ═════════════════ */

/// 解析 "major.minor.patch"（容忍 "v" 前缀与 "-beta" 等后缀）
pub fn parse_version(s: &str) -> Option<[u16; 3]> {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut out = [0u16; 3];
    for (i, part) in core.split('.').take(3).enumerate() {
        out[i] = part.trim().parse().ok()?;
    }
    Some(out)
}

/// a 是否严格新于 b
pub fn is_newer(a: [u16; 3], b: [u16; 3]) -> bool {
    a > b
}

/// 本程序版本（Cargo.toml 的 version）
pub fn current_version() -> [u16; 3] {
    parse_version(crate::version::APP_VERSION).unwrap_or([0, 0, 0])
}

/// 版本号文本（如 0.3.2）
pub fn current_version_text() -> &'static str {
    crate::version::APP_VERSION
}

/* ═════════════════ 检查 ═════════════════ */

/// 清单地址（COCOLE_UPDATE_URL 环境变量优先，本地 mock 联调用）
pub fn manifest_url() -> String {
    std::env::var("COCOLE_UPDATE_URL").unwrap_or_else(|_| UPDATE_MANIFEST_URL.to_string())
}

/// Unix 秒
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 是否到了自动检查的时间点（从未检查过，或距上次超过冷却期）
pub fn should_auto_check(last_check: Option<u64>) -> bool {
    match last_check {
        None => true,
        Some(t) => now_unix().saturating_sub(t) >= CHECK_COOLDOWN_SECS,
    }
}

/// 检查更新（异步入口）：拉取清单 → 比对版本 → 摘出当前平台资产
pub async fn check() -> Result<CheckOutcome, UpdaterError> {
    let url = manifest_url();
    tokio::task::spawn_blocking(move || check_blocking(&url))
        .await
        .map_err(|e| UpdaterError::Network(trf("后台任务异常：{}", &[&e.to_string()])))?
}

fn check_blocking(url: &str) -> Result<CheckOutcome, UpdaterError> {
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => UpdaterError::Network(format!("HTTP {code}")),
            ureq::Error::Transport(t) => UpdaterError::Network(t.to_string()),
        })?;

    let manifest: UpdateManifest = serde_json::from_reader(resp.into_reader())
        .map_err(|e| UpdaterError::Parse(e.to_string()))?;

    let remote = parse_version(&manifest.version)
        .ok_or_else(|| UpdaterError::Parse(trf("非法版本号：{}", &[&manifest.version])))?;

    if !is_newer(remote, current_version()) {
        return Ok(CheckOutcome::UpToDate);
    }

    let asset = manifest
        .platforms
        .get(PLATFORM_KEY)
        .ok_or(UpdaterError::NoPlatformAsset(PLATFORM_KEY))?;

    Ok(CheckOutcome::Update(Box::new(ReleaseInfo {
        version: manifest.version,
        notes: manifest.notes,
        pub_date: manifest.pub_date,
        mandatory: manifest.mandatory,
        download_url: asset.url.clone(),
        size: asset.size,
        sha256: asset.sha256.clone(),
    })))
}

/* ═════════════════ 下载 ═════════════════ */

/// 下载事件（经 Task::stream 驱动的消息流回传，天然保序）
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    Progress { received: u64, total: Option<u64> },
    /// 下载完成（版本号取自状态机里的 ReleaseInfo，这里只回传路径）
    Done { path: PathBuf },
    Failed(String),
}

/// 默认安装包落盘位置：%TEMP%\cocole-setup-<版本>.exe
pub fn installer_dest(version: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir();
    let name = format!("cocole-setup-{}.exe", version.trim_start_matches(['v', 'V']));
    Some(dir.join(name))
}

/// 构造下载任务：后台线程流式下载，进度事件经消息流逐条回传 UI。
///
/// `cancel` 置位后线程尽快退出并清理半成品文件。
pub fn download_task(
    info: &ReleaseInfo,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Task<Message> {
    let info = info.clone();
    Task::stream(iced::stream::channel(8, async move |mut out| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadEvent>();
        let url = info.download_url.clone();
        let sha = info.sha256.clone();
        let handle = std::thread::spawn(move || {
            download_blocking(&url, &dest, sha.as_deref(), &cancel, &tx)
        });
        // 线程侧每发一条事件这里转发一条；线程结束（tx drop）后循环自然退出
        while let Some(ev) = rx.recv().await {
            use iced::futures::SinkExt;
            let _ = out.send(Message::Updater(UpdaterMessage::DownloadEvent(ev))).await;
        }
        let _ = handle.join();
    }))
}

/// 阻塞式流式下载（在独立线程跑）：
/// ureq 拉流 → 64KB 块落盘 → 每 ~256KB / 每 100ms 回报一次进度 →
/// 完成后 SHA-256 校验（清单提供时）→ 发 Done；任何一步失败发 Failed。
fn download_blocking(
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    cancel: &AtomicBool,
    tx: &tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
) {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;

    use sha2::Digest;

    let _ = tx.send(DownloadEvent::Progress { received: 0, total: None });

    let resp = match ureq::get(url).call() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Failed(trf("连接失败：{}", &[&e.to_string()])));
            return;
        }
    };
    let total = resp
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok());

    let mut reader = resp.into_reader();
    let file = match std::fs::File::create(dest) {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Failed(trf("无法创建临时文件：{}", &[&e.to_string()])));
            return;
        }
    };
    let mut writer = std::io::BufWriter::new(file);

    let mut buf = [0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_report = std::time::Instant::now();
    let mut hasher = sha2::Sha256::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = writer.flush();
            let _ = std::fs::remove_file(dest);
            let _ = tx.send(DownloadEvent::Failed("已取消下载".into()));
            return;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                received += n as u64;
                if let Err(e) = writer.write_all(&buf[..n]) {
                    let _ = tx.send(DownloadEvent::Failed(trf("写入失败：{}", &[&e.to_string()])));
                    let _ = std::fs::remove_file(dest);
                    return;
                }
                // 限频回报：每 256KB 或每 100ms，避免消息风暴
                if received % (256 * 1024) < 64 * 1024
                    || last_report.elapsed() >= std::time::Duration::from_millis(100)
                {
                    last_report = std::time::Instant::now();
                    let _ = tx.send(DownloadEvent::Progress { received, total });
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(dest);
                let _ = tx.send(DownloadEvent::Failed(trf("网络中断：{}", &[&e.to_string()])));
                return;
            }
        }
    }
    if writer.flush().is_err() {
        let _ = tx.send(DownloadEvent::Failed("写入失败（收尾）".into()));
        let _ = std::fs::remove_file(dest);
        return;
    }

    // SHA-256 强校验（清单提供摘要时）
    if let Some(expect) = sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expect) {
            let _ = std::fs::remove_file(dest);
            let _ = tx.send(DownloadEvent::Failed(UpdaterError::HashMismatch.to_string()));
            return;
        }
    }

    let _ = tx.send(DownloadEvent::Done { path: dest.to_path_buf() });
}

/* ═════════════════ 安装 / 浏览器 ═════════════════ */

/// 启动下载好的安装器（独立进程，主程序随后退出）
#[cfg(windows)]
pub fn launch_installer(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS：安装器脱离主程序生命周期，主程序退出不影响安装
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(path)
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| trf("无法启动安装器：{}", &[&e.to_string()]))
}

#[cfg(not(windows))]
pub fn launch_installer(path: &Path) -> Result<(), String> {
    std::process::Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("无法启动安装器：{e}"))
}

/// 用系统默认浏览器打开 URL（兜底的「前往下载页」）
pub fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // start 是 cmd 内建命令，必须经 cmd /c 调起；
        // CREATE_NO_WINDOW 防止 GUI 进程派生 cmd 时闪现控制台窗口
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
            .map(|_| ())
            .map_err(|e| trf("无法打开浏览器：{}", &[&e.to_string()]))
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

/* ═════════════════ 单元测试 ═════════════════ */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse() {
        assert_eq!(parse_version("0.3.2"), Some([0, 3, 2]));
        assert_eq!(parse_version("v1.2.3-beta"), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2"), Some([1, 2, 0]));
        assert_eq!(parse_version("abc"), None);
    }

    #[test]
    fn version_compare() {
        assert!(is_newer([0, 3, 3], [0, 3, 2]));
        assert!(is_newer([0, 4, 0], [0, 3, 99]));
        assert!(is_newer([1, 0, 0], [0, 99, 99]));
        assert!(!is_newer([0, 3, 2], [0, 3, 2]));
        assert!(!is_newer([0, 3, 1], [0, 3, 2]));
    }
}