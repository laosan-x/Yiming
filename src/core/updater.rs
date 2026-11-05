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
//!       "mirror_urls": ["https://<镜像前缀>/https://…/cocole-setup-0.4.0.exe", "…"],
//!       "size": 15234567,
//!       "sha256": "…（强烈建议填写：多源下载的安全性完全依赖它）"
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
//! - 多源下载：`url`（GitHub Releases 主源）→ `mirror_urls`（境内可达镜像）
//!   逐个尝试，任一源下载完成且 SHA-256 校验通过即用；换源对 UI 无感知。
//!   镜像源是第三方代理（gh-proxy 系），不可信问题由哈希强校验兜底——
//!   内容不符一律丢弃换源，所以 `sha256` 在启用镜像后是必填级别的字段。
//! - 清单地址可被 `COCOLE_UPDATE_URL` 环境变量覆盖（与 COCOLE_DEBUG_* 同族，
//!   本地 mock 服务器联调用，见 tools/update-server-mock.mjs）。
//! - 所有错误都转成中文 `String` 直接给 UI 展示，不向上抛 panic。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use iced::Task;

use crate::core::i18n::{tr, trf};
use crate::message::{Message, UpdaterMessage};

/* ═════════════════ 常量 ═════════════════ */

/// 清单候选地址（按序尝试，任一成功即用）：
/// 1. jsDelivr CDN——中国大陆可直连（raw.githubusercontent.com 在境内被墙，
///    是 v0.4.1 用户「检查更新失败：timed out」的根因）；
/// 2. raw.githubusercontent.com——官方源，作为 jsDelivr 不可用时的兜底。
///
/// 注意 jsDelivr 对 /gh/ 有 ~12h 边缘缓存，发版后需 purge
/// （curl "https://purge.jsdelivr.net/gh/laosan-x/Yiming@main/update-manifest.json"，
/// 仅接受 GET；现已由 purge-cdn.yml 在清单变更推送后自动执行）。
pub const MANIFEST_URLS: &[&str] = &[
    "https://cdn.jsdelivr.net/gh/laosan-x/Yiming@main/update-manifest.json",
    "https://raw.githubusercontent.com/laosan-x/Yiming/main/update-manifest.json",
];
/// 清单不可用时的兜底下载页（浏览器打开）
pub const DOWNLOAD_PAGE_URL: &str = "https://github.com/laosan-x/Yiming/releases/latest";
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
    /// 镜像源候选（按序尝试，位于主源之后）：
    /// GitHub Releases 附件（objects.githubusercontent.com）在境内时好时坏，
    /// 清单里提供 gh-proxy 系镜像 URL 兜底；安全性由 sha256 强校验保证
    #[serde(default)]
    pub mirror_urls: Vec<String>,
    /// 文件字节数（用于下载进度百分比；缺失时进度条显示为不定态）
    #[serde(default)]
    pub size: Option<u64>,
    /// 安装包 SHA-256（十六进制小写；填写则下载完成后强校验。
    /// 启用 mirror_urls 后必须填写——镜像内容是否被篡改全靠它拦截）
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
    /// 主源（GitHub Releases 附件）
    pub download_url: String,
    /// 镜像源候选（主源失败后按序尝试）
    pub mirror_urls: Vec<String>,
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

/// 版本号**字面量**白名单：只允许可选 v 前缀 + 三段纯数字。
///
/// 清单里的 `version` 会被拼进 `%TEMP%` 安装包文件名（[`installer_dest`]），
/// 也会进 UI 与发布说明。`parse_version` 为兼容 beta 后缀做了宽松截断，
/// 不能拿来当文件名校验：`"0.4.0.1\\..\\evil"` 会被它截成 0.4.0 后放行，
/// 拼接后可能把安装包写到 %TEMP% 之外。凡是来自外部清单、要参与路径拼接
/// 的版本号，必须先过这里。
pub fn is_safe_version_literal(s: &str) -> bool {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
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

/// 清单候选地址（COCOLE_UPDATE_URL 环境变量优先，本地 mock 联调用）
pub fn manifest_urls() -> Vec<String> {
    match std::env::var("COCOLE_UPDATE_URL") {
        Ok(u) if !u.is_empty() => vec![u],
        _ => MANIFEST_URLS.iter().map(|s| s.to_string()).collect(),
    }
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

/// 检查更新（异步入口）：逐个候选源拉取清单 → 比对版本 → 摘出当前平台资产
pub async fn check() -> Result<CheckOutcome, UpdaterError> {
    let urls = manifest_urls();
    tokio::task::spawn_blocking(move || {
        let mut last: Option<UpdaterError> = None;
        for url in urls {
            match check_blocking(&url) {
                Ok(outcome) => return Ok(outcome),
                // 平台缺失 / 版本解析失败是清单本身的问题，换源也没用，直接返回
                Err(e @ (UpdaterError::NoPlatformAsset(_) | UpdaterError::Parse(_))) => {
                    return Err(e)
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(UpdaterError::Network("无可用清单源".into())))
    })
    .await
    .map_err(|e| UpdaterError::Network(trf("后台任务异常：{}", &[&e.to_string()])))?
}

/// 清单资产校验：镜像源是第三方代理，内容可被篡改；没有 SHA-256 就等于
/// 放行任意可执行文件。只要清单提供了镜像源，摘要就是硬性前置条件。
fn validate_asset(asset: &PlatformAsset) -> Result<(), UpdaterError> {
    if !asset.mirror_urls.is_empty() && asset.sha256.is_none() {
        return Err(UpdaterError::Parse(
            tr("更新清单缺少 SHA-256 校验值，已拒绝下载").to_string(),
        ));
    }
    Ok(())
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

    // 先用严格白名单挡住路径遍历 / 多余段，再用宽松解析器做版本比较
    if !is_safe_version_literal(&manifest.version) {
        return Err(UpdaterError::Parse(trf("非法版本号：{}", &[&manifest.version])));
    }
    let remote = parse_version(&manifest.version)
        .ok_or_else(|| UpdaterError::Parse(trf("非法版本号：{}", &[&manifest.version])))?;

    if !is_newer(remote, current_version()) {
        return Ok(CheckOutcome::UpToDate);
    }

    let asset = manifest
        .platforms
        .get(PLATFORM_KEY)
        .ok_or(UpdaterError::NoPlatformAsset(PLATFORM_KEY))?;

    validate_asset(asset)?;

    Ok(CheckOutcome::Update(Box::new(ReleaseInfo {
        version: manifest.version,
        notes: manifest.notes,
        pub_date: manifest.pub_date,
        mandatory: manifest.mandatory,
        download_url: asset.url.clone(),
        mirror_urls: asset.mirror_urls.clone(),
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
    if !is_safe_version_literal(version) {
        return None;
    }
    let dir = std::env::temp_dir();
    let name = format!(
        "cocole-setup-{}.exe",
        version.trim_start_matches(['v', 'V'])
    );
    Some(dir.join(name))
}

/// 构造下载任务：后台线程流式下载，进度事件经消息流逐条回传 UI。
///
/// 候选源 = 主源（GitHub Releases）+ `mirror_urls` 镜像，逐个尝试，
/// 任一源下载完成且 SHA-256 校验通过即成功；换源对 UI 无感知。
/// `cancel` 置位后线程尽快退出并清理半成品文件。
pub fn download_task(
    info: &ReleaseInfo,
    dest: PathBuf,
    cancel: Arc<AtomicBool>,
) -> Task<Message> {
    let info = info.clone();
    Task::stream(iced::stream::channel(8, async move |mut out| {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadEvent>();
        let mut urls = vec![info.download_url.clone()];
        urls.extend(info.mirror_urls.iter().cloned());
        let sha = info.sha256.clone();
        let handle = std::thread::spawn(move || {
            download_blocking(&urls, &dest, sha.as_deref(), &cancel, &tx)
        });
        // 线程侧每发一条事件这里转发一条；线程结束（tx drop）后循环自然退出
        while let Some(ev) = rx.recv().await {
            use iced::futures::SinkExt;
            let _ = out.send(Message::Updater(UpdaterMessage::DownloadEvent(ev))).await;
        }
        let _ = handle.join();
    }))
}

/// 单源下载结果（内部枚举，不出模块）
enum OneOutcome {
    /// 下载完成，返回十六进制 SHA-256 摘要
    Success(String),
    /// 用户取消（半成品已清理）
    Cancelled,
    /// 失败（含原因；半成品由调用方清理）
    Error(String),
}

/// 阻塞式多源下载（在独立线程跑）：
/// 逐个候选源尝试（主源 → 镜像源），单源网络失败或哈希不匹配自动换
/// 下一源；全部源耗尽才向 UI 报失败。
/// SHA-256 强校验（清单提供摘要时）在每源下载完成后执行——镜像源是
/// 第三方代理，内容被篡改或缓存错版本时在此拦截丢弃换源。
fn download_blocking(
    urls: &[String],
    dest: &Path,
    sha256: Option<&str>,
    cancel: &AtomicBool,
    tx: &tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
) {
    // 纵深防御：即使 ReleaseInfo 由别的路径构造，也不能出现
    // 「多源下载 + 无摘要」的组合（check_blocking 已在入口拒收）。
    if urls.len() > 1 && sha256.is_none() {
        let _ = tx.send(DownloadEvent::Failed(
            tr("更新清单缺少 SHA-256 校验值，已拒绝下载").to_string(),
        ));
        return;
    }
    let mut last_err: Option<String> = None;
    for url in urls {
        match download_one(url, dest, cancel, tx) {
            OneOutcome::Success(hash) => {
                if let Some(expect) = sha256 {
                    if !hash.eq_ignore_ascii_case(expect) {
                        let _ = std::fs::remove_file(dest);
                        last_err = Some(UpdaterError::HashMismatch.to_string());
                        continue; // 换下一源
                    }
                }
                let _ = tx.send(DownloadEvent::Done { path: dest.to_path_buf() });
                return;
            }
            OneOutcome::Cancelled => return,
            OneOutcome::Error(e) => {
                let _ = std::fs::remove_file(dest);
                last_err = Some(e);
                // 还有候选源时静默换源（进度条会在下一源开头归零）
            }
        }
    }
    let _ = tx.send(DownloadEvent::Failed(last_err.unwrap_or_else(|| {
        UpdaterError::Network("无可用下载源".into()).to_string()
    })));
}

/// 下载单个 URL：流式落盘 + 计算摘要，64KB 块读写，
/// 每 ~256KB / 每 100ms 回报一次进度。不做哈希比对（调用方负责）。
fn download_one(
    url: &str,
    dest: &Path,
    cancel: &AtomicBool,
    tx: &tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
) -> OneOutcome {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;

    use sha2::Digest;

    // 进度归零：多源重试时 UI 进度条从头计
    let _ = tx.send(DownloadEvent::Progress { received: 0, total: None });

    let resp = match ureq::get(url).call() {
        Ok(r) => r,
        Err(e) => {
            return OneOutcome::Error(trf("连接失败：{}", &[&e.to_string()]));
        }
    };
    let total = resp
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok());

    let mut reader = resp.into_reader();
    let file = match std::fs::File::create(dest) {
        Ok(f) => f,
        Err(e) => {
            return OneOutcome::Error(trf("无法创建临时文件：{}", &[&e.to_string()]));
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
            return OneOutcome::Cancelled;
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.update(&buf[..n]);
                received += n as u64;
                if let Err(e) = writer.write_all(&buf[..n]) {
                    return OneOutcome::Error(trf("写入失败：{}", &[&e.to_string()]));
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
                return OneOutcome::Error(trf("网络中断：{}", &[&e.to_string()]));
            }
        }
    }
    if writer.flush().is_err() {
        return OneOutcome::Error("写入失败（收尾）".into());
    }

    OneOutcome::Success(format!("{:x}", hasher.finalize()))
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

    /// 回归：清单 version 会拼进安装包文件名，畸形串不得放行
    #[test]
    fn safe_version_literal_rejects_path_traversal() {
        assert!(is_safe_version_literal("0.4.5"));
        assert!(is_safe_version_literal("v0.4.5"));
        assert!(!is_safe_version_literal(r"0.4.0.1\..\evil"));
        assert!(!is_safe_version_literal("0.4.0/../../evil"));
        assert!(!is_safe_version_literal("0.4.5-beta"));
        assert!(!is_safe_version_literal("0..5"));
        assert_eq!(installer_dest(r"0.4.0.1\..\evil"), None);
        assert!(installer_dest("0.4.5").is_some());
    }

    /// 回归：有镜像源却没摘要时必须拒收（第三方代理不能被静默信任）
    #[test]
    fn mirror_without_sha256_is_rejected() {
        let with_mirror: PlatformAsset = serde_json::from_str(
            r#"{"url":"https://example.com/a.exe","mirror_urls":["https://mirror.example.com/a.exe"]}"#,
        )
        .unwrap();
        assert!(validate_asset(&with_mirror).is_err());

        let with_hash: PlatformAsset = serde_json::from_str(
            r#"{"url":"https://example.com/a.exe","mirror_urls":["https://mirror.example.com/a.exe"],"sha256":"deadbeef"}"#,
        )
        .unwrap();
        assert!(validate_asset(&with_hash).is_ok());

        let no_mirror: PlatformAsset =
            serde_json::from_str(r#"{"url":"https://example.com/a.exe"}"#).unwrap();
        assert!(validate_asset(&no_mirror).is_ok());
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