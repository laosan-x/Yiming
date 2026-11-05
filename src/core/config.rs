//! 配置持久化：JSON 写入系统配置目录
//!
//! 原项目存在浏览器 localStorage，原生版改为写文件：
//!   Windows `%APPDATA%\yiming\cocole-iced\config.json`
//!   macOS    `~/Library/Application Support/cn.yiming.cocole-iced/config.json`
//!   Linux    `~/.config/cocole-iced/config.json`
//! 读写全部包在 try 里：目录不可写时降级为「内存配置」，应用照常可用。

use crate::core::image::{ExportFormat, ExportSize};
use crate::core::matting::MatteParams;
use crate::core::idphoto::RenderMode;
use crate::core::hotkey::KeymapOverrides;
use crate::core::updater::UpdatePrefs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrushProps {
    pub size: i32,
    pub opacity: i32,
    pub flow: i32,
    /// 保留画笔硬度
    pub hardness_keep: i32,
    /// 擦除画笔硬度
    pub hardness_erase: i32,
}

impl Default for BrushProps {
    fn default() -> Self {
        Self { size: 18, opacity: 100, flow: 100, hardness_keep: 85, hardness_erase: 85 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdConfig {
    pub spec_idx: usize,
    pub bg_idx: usize,
    pub render: RenderMode,
    pub zoom: i32,
    pub off_y: i32,
    pub erode: i32,
    pub paper_idx: usize,
    pub cutline: bool,
    pub realsize: bool,
    /// 毫米制规格输出像素的 DPI（像素制规格不受影响）
    #[serde(default = "default_id_dpi")]
    pub dpi: u32,
    /// 单张导出格式（png / jpg / webp）
    #[serde(default = "default_id_export_fmt")]
    pub export_fmt: String,
    /// 旋转校正角度（度，-15..=15，0 = 不旋转）
    #[serde(default)]
    pub rot: i32,
    /// 证件照页选中的抠图模型 id（None = 回退 modnet.onnx）
    #[serde(default)]
    pub model_id: Option<String>,
    /// 启用自定义底色（HEX）
    #[serde(default)]
    pub use_custom_bg: bool,
    /// 自定义底色 HEX 值
    #[serde(default = "default_custom_bg")]
    pub custom_bg_hex: String,
}

fn default_id_dpi() -> u32 {
    300
}

fn default_id_export_fmt() -> String {
    "png".to_string()
}

fn default_custom_bg() -> String {
    "#438EDB".to_string()
}

impl Default for IdConfig {
    fn default() -> Self {
        Self {
            spec_idx: 0,
            bg_idx: 0,
            render: RenderMode::Solid,
            zoom: 100,
            off_y: 0,
            erode: 1,
            paper_idx: 0,
            cutline: true,
            realsize: true,
            dpi: 300,
            export_fmt: "png".to_string(),
            rot: 0,
            model_id: None,
            use_custom_bg: false,
            custom_bg_hex: "#438EDB".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stats {
    pub count: u32,
    pub total_ms: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub nav_collapsed: bool,
    pub model: String,
    pub auto: bool,
    pub export_format: String,
    pub export_size: u32,
    pub out_color: String,
    pub bg_preview: String,
    pub params: MatteParams,
    pub brush: BrushProps,
    pub plugin_dir: Option<String>,
    pub api_url: String,
    pub seen_onboarding: bool,
    /// 首页分步新手引导的历史标记：完成 /「不再显示」时写入。
    /// 仅保留兼容旧配置与关闭行为；引导已改为「仅点按钮开启」，此字段
    /// 不再参与任何显示判定。
    #[serde(default)]
    pub guide_done: bool,
    pub id: IdConfig,
    pub stats: Stats,
    pub recent: Vec<RecentEntry>,
    #[serde(default)]
    pub keymap: KeymapOverrides,
    /// 自更新偏好（自动检查开关 / 忽略版本 / 上次检查时间）
    #[serde(default)]
    pub updates: UpdatePrefs,
    /// 外观偏好：主题模式 / 自定义强调色（hex）/ 语言
    #[serde(default)]
    pub appearance: AppearanceConfig,
    /// 免打扰时段（系统通知静默；应用内消息照常记录）
    #[serde(default)]
    pub dnd: DndConfig,
}

/// 外观偏好
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppearanceConfig {
    /// light / dark / system
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// 自定义强调色（#RRGGBB；None = 默认蓝）
    #[serde(default)]
    pub accent_hex: Option<String>,
    /// 界面语言（zh / en）
    #[serde(default = "default_lang")]
    pub lang: String,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self { theme_mode: default_theme_mode(), accent_hex: None, lang: default_lang() }
    }
}

fn default_theme_mode() -> String {
    "system".to_string()
}

fn default_lang() -> String {
    "zh".to_string()
}

/// 免打扰时段配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DndConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 起始时间 HH:MM（默认 22:00）
    #[serde(default = "default_dnd_start")]
    pub start: String,
    /// 结束时间 HH:MM（默认 08:00；跨零点区间已处理）
    #[serde(default = "default_dnd_end")]
    pub end: String,
}

impl Default for DndConfig {
    fn default() -> Self {
        Self { enabled: false, start: default_dnd_start(), end: default_dnd_end() }
    }
}

fn default_dnd_start() -> String {
    "22:00".to_string()
}

fn default_dnd_end() -> String {
    "08:00".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            nav_collapsed: false,
            model: "local-smart".to_string(),
            auto: true,
            export_format: "png".to_string(),
            export_size: 1,
            out_color: "transparent".to_string(),
            bg_preview: "checker".to_string(),
            params: MatteParams::default(),
            brush: BrushProps::default(),
            plugin_dir: None,
            api_url: String::new(),
            seen_onboarding: false,
            guide_done: false,
            id: IdConfig::default(),
            stats: Stats::default(),
            recent: Vec::new(),
            keymap: KeymapOverrides::default(),
            updates: UpdatePrefs::default(),
            appearance: AppearanceConfig::default(),
            dnd: DndConfig::default(),
        }
    }
}

impl Config {
    pub fn export_format(&self) -> ExportFormat {
        match self.export_format.as_str() {
            "jpg" => ExportFormat::Jpg,
            "webp" => ExportFormat::Webp,
            _ => ExportFormat::Png,
        }
    }
    pub fn export_size(&self) -> ExportSize {
        ExportSize::from_u32(self.export_size)
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("cn", "yiming", "cocole-iced")?;
    let dir = dirs.config_dir();
    std::fs::create_dir_all(dir).ok()?;
    Some(dir.join("config.json"))
}

impl Config {
    /// 读取配置；任何失败都回落到默认值（不因配置损坏而启动失败）。
    /// 读出的数值统一经 [`Config::sanitize`] 收敛，避免外部 JSON 越界。
    pub fn load() -> Self {
        let mut cfg = Self::read_from_disk();
        cfg.sanitize();
        cfg
    }

    fn read_from_disk() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<Self>(&text).unwrap_or_else(|e| {
            eprintln!("[config] 解析失败，回落默认配置：{e}");
            Self::default()
        })
    }

    /// 把外部 JSON 的数值字段收敛到代码允许的范围。
    ///
    /// config.json 可被手工编辑 / 旧版本迁移 / 半写入：视图层对索引有
    /// `min(len-1)` 兜底，但后台渲染 / 导出 / 批量线程直接 `ID_SIZES[spec_idx]`，
    /// 越界会 panic——批量线程 panic 后 `Finished` 永不回传，批量导出会永久
    /// 锁死。同理极端 `export_size` / `dpi` 会触发巨量分配或除零。
    /// 因此在唯一入口处一次性收敛，后续所有链路都拿到合法值。
    fn sanitize(&mut self) {
        use crate::core::idphoto::{ID_BGS, ID_PAPERS, ID_SIZES};

        let id = &mut self.id;
        id.spec_idx = id.spec_idx.min(ID_SIZES.len().saturating_sub(1));
        id.bg_idx = id.bg_idx.min(ID_BGS.len().saturating_sub(1));
        id.paper_idx = id.paper_idx.min(ID_PAPERS.len().saturating_sub(1));
        id.zoom = id.zoom.clamp(80, 200);
        id.off_y = id.off_y.clamp(-45, 45);
        id.erode = id.erode.clamp(0, 3);
        id.rot = id.rot.clamp(-15, 15);
        id.dpi = id.dpi.clamp(72, 1200);

        // 非 UI 预设值一律回落「原始尺寸」，杜绝 RgbaImage::new(t, t) 巨量分配
        if !ExportSize::all().contains(&self.export_size()) {
            self.export_size = 1;
        }
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    eprintln!("[config] 写入失败（配置不持久，应用继续运行）：{e}");
                }
            }
            Err(e) => eprintln!("[config] 序列化失败：{e}"),
        }
    }

    pub fn push_recent(&mut self, path: &str, name: &str) {
        self.recent.retain(|r| r.path != path);
        self.recent.insert(0, RecentEntry { path: path.to_string(), name: name.to_string() });
        self.recent.truncate(12);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::idphoto::{ID_BGS, ID_PAPERS, ID_SIZES};

    /// 回归：外部 JSON 的越界索引 / 极端数值必须在 load 入口被收敛，
    /// 否则后台线程直接索引常量表会 panic（批量导出会永久锁死）。
    #[test]
    fn sanitize_clamps_out_of_range_config() {
        let mut c = Config::default();
        c.id.spec_idx = usize::MAX;
        c.id.bg_idx = usize::MAX;
        c.id.paper_idx = usize::MAX;
        c.id.zoom = 999_999;
        c.id.off_y = -10_000;
        c.id.erode = -7;
        c.id.rot = 90;
        c.id.dpi = 0;
        c.export_size = 999_999;

        c.sanitize();

        assert_eq!(c.id.spec_idx, ID_SIZES.len() - 1);
        assert_eq!(c.id.bg_idx, ID_BGS.len() - 1);
        assert_eq!(c.id.paper_idx, ID_PAPERS.len() - 1);
        assert_eq!(c.id.zoom, 200);
        assert_eq!(c.id.off_y, -45);
        assert_eq!(c.id.erode, 0);
        assert_eq!(c.id.rot, 15);
        assert_eq!(c.id.dpi, 72);
        assert_eq!(c.export_size, 1);
    }

    #[test]
    fn sanitize_keeps_valid_config_unchanged() {
        let mut c = Config::default();
        c.id.spec_idx = 3;
        c.id.bg_idx = 2;
        c.id.dpi = 600;
        c.export_size = 800;
        c.sanitize();
        assert_eq!(
            (c.id.spec_idx, c.id.bg_idx, c.id.dpi, c.export_size),
            (3, 2, 600, 800)
        );
    }

    /// 序列化往返后仍能收敛（模拟手工编辑过的 config.json）
    #[test]
    fn sanitize_after_json_roundtrip() {
        let mut c = Config::default();
        c.id.spec_idx = 9999;
        let text = serde_json::to_string(&c).expect("序列化应成功");
        let mut restored: Config = serde_json::from_str(&text).expect("反序列化应成功");
        restored.sanitize();
        assert_eq!(restored.id.spec_idx, ID_SIZES.len() - 1);
    }
}
