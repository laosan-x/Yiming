//! 设计 token 与组件样式 —— 与前端 `src/styles/tokens.css` 逐值对齐
//!
//! iced 的 `Theme` 只提供有限的调色板，本项目是自定义的浅色中性主题，
//! 所有颜色 / 圆角 / 阴影都直接取自原项目的 CSS 变量，
//! 不依赖 iced 的夜间 / 日间主题切换。
//!
//! 原项目的视觉体系有两条硬规矩，本文件据此组织：
//!
//! 1. **中性为骨、强调为点**：界面主体是黑 / 白 / 灰三级中性色，
//!    `--acc` 蓝色只出现在链接、焦点环、耗时数值这类小面积处。
//!    主按钮、开关、分段控件的选中态一律是 `--ink` 近黑，不是蓝。
//! 2. **面板浮在底色上**：外层 `--bg` 浅灰，所有功能面板（侧栏 / 工具条 /
//!    画布 / 舞台条 / 检查器 / 状态栏）都是白底 + hairline 描边 +
//!    `--shadow-1` 极淡扩散的 14px 圆角卡片，之间留 10px 空隙。
//!
//! 文件分层：
//!   1. 颜色 token      2. 形状 / 尺寸 / 字号 token   3. 阴影
//!   4. 文本辅助        5. 容器样式                  6. 按钮 / 控件样式

// 设计系统模块：刻意暴露比当前 UI 更多的 token（强调色阶、焦点环、备用字号、
// 阴影层级、各种面板 / 控件样式），它们是参考 CSS 词汇表的一一对应，留作后续
// 页面与组件取用，故整体豁免 dead_code 告警。
#![allow(dead_code)]
#![allow(non_snake_case)] // 颜色 token 同名函数保持原 CSS 变量名（调用点兼容）

use std::sync::OnceLock;

use iced::widget::{button, container, pick_list, scrollable, slider, text, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector};

/* ═════════════════ 1. 颜色 token（tokens.css `:root`） ═════════════════ */

/* ── 主题系统：Palette 动态解析 ──
 *
 * 颜色 token 不再是编译期常量，而是从全局 `Palette`（浅色/深色两套）按当前
 * 模式取值；下方与原 token 同名的零参函数保持全部调用点兼容。
 * 自定义强调色（可选）替换 ACC() 系并派生 soft/faint/ring；深色模式下
 * INK() 也取强调色（主按钮白字在深底上需要亮色承载）。
 */

/// 解析后的主题模式（Light / Dark；「跟随系统」在上层已解析成这两者）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Light,
    Dark,
}

/// 全部颜色 token 的集合（Copy，取用时整体拷贝，无锁开销）
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub panel: Color,
    pub panel_2: Color,
    pub panel_3: Color,
    pub hover: Color,
    pub active: Color,
    pub fill: Color,
    pub fill_2: Color,
    pub acc: Color,
    pub acc_hover: Color,
    pub acc_active: Color,
    pub acc_soft: Color,
    pub acc_faint: Color,
    pub acc_ring: Color,
    pub txt: Color,
    pub txt_2: Color,
    pub txt_3: Color,
    pub ink: Color,
    pub ink_hover: Color,
    pub ink_active: Color,
    pub ring: Color,
    pub line: Color,
    pub line_soft: Color,
    pub line_strong: Color,
    pub ok: Color,
    pub ok_soft: Color,
    pub ok_bright: Color,
    pub warn: Color,
    pub warn_soft: Color,
    pub danger: Color,
    pub danger_soft: Color,
    pub checker_a: Color,
    pub checker_b: Color,
}

/// 颜色插值：t=0 → a，t=1 → b
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// 亮度调节（factor > 1 提亮，< 1 压暗）
fn shade(c: Color, factor: f32) -> Color {
    Color {
        r: (c.r * factor).min(1.0),
        g: (c.g * factor).min(1.0),
        b: (c.b * factor).min(1.0),
        a: c.a,
    }
}

/// 浅色调色板（accent: 用户自定义强调色；None = 默认蓝）
fn light_palette(accent: Option<Color>) -> Palette {
    let acc = accent.unwrap_or_else(|| c(0x41, 0x76, 0xe6));
    let bg = c(0xf5, 0xf6, 0xf7);
    Palette {
        bg,
        panel: c(0xff, 0xff, 0xff),
        panel_2: c(0xfb, 0xfb, 0xfc),
        panel_3: c(0xf4, 0xf5, 0xf7),
        hover: c(0xf1, 0xf2, 0xf4),
        active: c(0xe9, 0xea, 0xee),
        fill: c(0xee, 0xf0, 0xf2),
        fill_2: c(0xf6, 0xf7, 0xf8),
        acc,
        acc_hover: shade(acc, 0.90),
        acc_active: shade(acc, 0.80),
        acc_soft: mix(acc, c(0xff, 0xff, 0xff), 0.90),
        acc_faint: mix(acc, c(0xff, 0xff, 0xff), 0.95),
        acc_ring: Color { a: 0.16, ..acc },
        txt: c(0x0f, 0x11, 0x15),
        txt_2: c(0x61, 0x66, 0x6b),
        txt_3: c(0x81, 0x85, 0x8c),
        ink: c(0x0f, 0x11, 0x15),
        ink_hover: c(0x2e, 0x2e, 0x2e),
        ink_active: Color::BLACK,
        ring: Color { a: 0.10, ..c(0x0f, 0x11, 0x15) },
        line: c(0xe9, 0xea, 0xed),
        line_soft: c(0xf1, 0xf2, 0xf4),
        line_strong: c(0xd9, 0xdc, 0xe1),
        ok: c(0x0e, 0x9f, 0x6e),
        ok_soft: c(0xe7, 0xf7, 0xf0),
        ok_bright: c(0x5f, 0xd6, 0xa4),
        warn: c(0xd9, 0x77, 0x06),
        warn_soft: c(0xfe, 0xf3, 0xe2),
        danger: c(0xdc, 0x26, 0x26),
        danger_soft: c(0xfd, 0xec, 0xec),
        checker_a: Color::WHITE,
        checker_b: c(0xf0, 0xf1, 0xf3),
    }
}

/// 深色调色板（accent 同上；INK() 取强调色承载主按钮）
fn dark_palette(accent: Option<Color>) -> Palette {
    let acc = accent.unwrap_or_else(|| c(0x5b, 0x8b, 0xf0));
    let bg = c(0x14, 0x16, 0x19);
    Palette {
        bg,
        panel: c(0x1d, 0x20, 0x24),
        panel_2: c(0x22, 0x26, 0x2a),
        panel_3: c(0x27, 0x2c, 0x31),
        hover: c(0x2d, 0x32, 0x37),
        active: c(0x34, 0x3a, 0x40),
        fill: c(0x2c, 0x31, 0x36),
        fill_2: c(0x23, 0x26, 0x29),
        acc,
        acc_hover: shade(acc, 1.12),
        acc_active: shade(acc, 0.85),
        acc_soft: mix(acc, bg, 0.78),
        acc_faint: mix(acc, bg, 0.90),
        acc_ring: Color { a: 0.25, ..acc },
        txt: c(0xe9, 0xeb, 0xee),
        txt_2: c(0xa8, 0xae, 0xb6),
        txt_3: c(0x7b, 0x82, 0x8a),
        ink: acc,
        ink_hover: shade(acc, 1.12),
        ink_active: shade(acc, 0.85),
        ring: Color { a: 0.14, ..c(0xe9, 0xeb, 0xee) },
        line: c(0x30, 0x35, 0x3b),
        line_soft: c(0x26, 0x2a, 0x2f),
        line_strong: c(0x3c, 0x41, 0x47),
        ok: c(0x34, 0xb5, 0x83),
        ok_soft: c(0x12, 0x28, 0x1f),
        ok_bright: c(0x5f, 0xd6, 0xa4),
        warn: c(0xe6, 0x9c, 0x3c),
        warn_soft: c(0x2e, 0x24, 0x16),
        danger: c(0xe5, 0x48, 0x4d),
        danger_soft: c(0x32, 0x19, 0x1a),
        checker_a: c(0x2a, 0x2e, 0x33),
        checker_b: c(0x22, 0x26, 0x2a),
    }
}

struct Palettes {
    light: Palette,
    dark: Palette,
}

static CUSTOM_ACCENT: std::sync::RwLock<Option<Color>> = std::sync::RwLock::new(None);
static PALETTES: std::sync::Mutex<Option<Palettes>> = std::sync::Mutex::new(None);
static RESOLVED: std::sync::RwLock<Resolved> = std::sync::RwLock::new(Resolved::Light);

fn build_palettes() -> Palettes {
    let accent = *CUSTOM_ACCENT.read().unwrap_or_else(|p| p.into_inner());
    Palettes {
        light: light_palette(accent),
        dark: dark_palette(accent),
    }
}

/// 设置自定义强调色（None = 恢复默认蓝），立即重建两套调色板
pub fn set_accent(accent: Option<Color>) {
    *CUSTOM_ACCENT.write().unwrap_or_else(|p| p.into_inner()) = accent;
    *PALETTES.lock().unwrap_or_else(|p| p.into_inner()) = Some(build_palettes());
}

/// 切换解析后的主题模式（应用侧负责把「跟随系统」解析成这两者）
pub fn set_resolved(r: Resolved) {
    *RESOLVED.write().unwrap_or_else(|p| p.into_inner()) = r;
}

/// 当前解析后的模式
pub fn resolved() -> Resolved {
    *RESOLVED.read().unwrap_or_else(|p| p.into_inner())
}

/// 取当前调色板（Copy 整体拷贝，样式闭包内随意调用）
pub fn P() -> Palette {
    let mut guard = PALETTES.lock().unwrap_or_else(|p| p.into_inner());
    let p = guard.get_or_insert_with(build_palettes);
    match *RESOLVED.read().unwrap_or_else(|po| po.into_inner()) {
        Resolved::Light => p.light,
        Resolved::Dark => p.dark,
    }
}

/// 用户可选的主题模式（含「跟随系统」；System 在应用侧解析成 Light/Dark）
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl ThemeMode {
    /// 解析成 Light/Dark：System 读一次系统偏好（boot 时缓存到应用侧）
    pub fn resolved(self, system_dark: bool) -> Resolved {
        match self {
            ThemeMode::Light => Resolved::Light,
            ThemeMode::Dark => Resolved::Dark,
            ThemeMode::System => {
                if system_dark {
                    Resolved::Dark
                } else {
                    Resolved::Light
                }
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "浅色",
            ThemeMode::Dark => "深色",
            ThemeMode::System => "跟随系统",
        }
    }

    pub fn from_code(s: &str) -> Self {
        match s {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::System,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
            ThemeMode::System => "system",
        }
    }

    pub const ALL: &'static [ThemeMode] =
        &[ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];
}

/// 系统是否偏好深色（Windows 注册表 AppsUseLightTheme；读取一次缓存，不实时跟随）
///
/// 用原生 RegGetValueW 直读注册表，而非派生 reg.exe 子进程——
/// GUI 子系统进程 spawn 控制台程序且未设 CREATE_NO_WINDOW 时，
/// Windows 会为子进程新建控制台窗口，导致启动瞬间闪黑框。
#[cfg(windows)]
pub fn system_prefers_dark() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("AppsUseLightTheme");
    // 默认按浅色处理（键缺失 / 读取失败 / 值非 0 均视为浅色）
    let mut data: u32 = 1;
    let mut cb = std::mem::size_of::<u32>() as u32;
    let res = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut cb),
        )
    };
    res.is_ok() && data == 0
}

#[cfg(not(windows))]
pub fn system_prefers_dark() -> bool {
    false
}

/// 映射到 iced 原生主题（原生控件：markdown / switch / pick_list 等跟随）
pub fn iced_theme(r: Resolved) -> Theme {
    match r {
        Resolved::Light => Theme::Light,
        Resolved::Dark => Theme::Dark,
    }
}

/* ── 与原 token 同名的零参函数（调用点零迁移成本） ── */

#[allow(non_snake_case)]
#[inline]
pub fn BG() -> Color { P().bg }
#[inline]
pub fn PANEL() -> Color { P().panel }
#[inline]
pub fn PANEL_2() -> Color { P().panel_2 }
#[inline]
pub fn PANEL_3() -> Color { P().panel_3 }
#[inline]
pub fn HOVER() -> Color { P().hover }
#[inline]
pub fn ACTIVE() -> Color { P().active }
#[inline]
pub fn FILL() -> Color { P().fill }
#[inline]
pub fn FILL_2() -> Color { P().fill_2 }
#[inline]
pub fn ACC() -> Color { P().acc }
#[inline]
pub fn ACC_HOVER() -> Color { P().acc_hover }
#[inline]
pub fn ACC_ACTIVE() -> Color { P().acc_active }
#[inline]
pub fn ACC_SOFT() -> Color { P().acc_soft }
#[inline]
pub fn ACC_FAINT() -> Color { P().acc_faint }
#[inline]
pub fn ACC_RING() -> Color { P().acc_ring }
#[inline]
pub fn TXT() -> Color { P().txt }
#[inline]
pub fn TXT2() -> Color { P().txt_2 }
#[inline]
pub fn TXT3() -> Color { P().txt_3 }
#[inline]
pub fn INK() -> Color { P().ink }
#[inline]
pub fn INK_HOVER() -> Color { P().ink_hover }
#[inline]
pub fn INK_ACTIVE() -> Color { P().ink_active }
#[inline]
pub fn RING() -> Color { P().ring }
#[inline]
pub fn LINE() -> Color { P().line }
#[inline]
pub fn LINE_SOFT() -> Color { P().line_soft }
#[inline]
pub fn LINE_STRONG() -> Color { P().line_strong }
#[inline]
pub fn OK() -> Color { P().ok }
#[inline]
pub fn OK_SOFT() -> Color { P().ok_soft }
#[inline]
pub fn OK_BRIGHT() -> Color { P().ok_bright }
#[inline]
pub fn WARN() -> Color { P().warn }
#[inline]
pub fn WARN_SOFT() -> Color { P().warn_soft }
#[inline]
pub fn DANGER() -> Color { P().danger }
#[inline]
pub fn DANGER_SOFT() -> Color { P().danger_soft }
#[inline]
pub fn CHECKER_A() -> Color { P().checker_a }
#[inline]
pub fn CHECKER_B() -> Color { P().checker_b }

/// `.checker` 的 `background-size:18px 18px`
pub const CHECKER_CELL: f32 = 18.0;

/* ═════════════════ 2. 形状 / 尺寸 / 字号 token ═════════════════ */

pub const R_SM: f32 = 8.0; // --r-sm
pub const R_BTN: f32 = 10.0; // --r-btn
pub const R_CARD: f32 = 12.0; // --r-card
pub const R_PANEL: f32 = 14.0; // --r-panel

/// `.sidenav{width:200px}`
pub const NAV_W: f32 = 200.0;
/// `.sidenav.collapsed{width:60px}`
pub const NAV_W_COLLAPSED: f32 = 60.0;
/// `.inspector{width:300px}`
pub const INSPECTOR_W: f32 = 300.0;
/// `.toolstrip{height:46px}`
pub const STRIP_H: f32 = 46.0;
/// `.stage-bar{height:46px}`
pub const STAGE_BAR_H: f32 = 46.0;
/// `.statusbar{height:34px}`
pub const STATUSBAR_H: f32 = 34.0;
/// `.body{padding:10px}` —— 外壳四周留白
pub const BODY_PAD: f32 = 10.0;
/// `.body{gap:10px}` / `#pageWork{gap:10px}` —— 面板间距
pub const BODY_GAP: f32 = 10.0;

/* ── 统一间距尺度（对齐参考 CSS：`.sec{padding:16}` / `.card{padding:16}`） ── */
/// 卡片 / 面板内容统一内边距（参考 `.card` / `.sec` 均为 16px）
pub const CARD_PAD: f32 = 16.0;
/// 间距尺度：4 / 8 / 10 / 12 / 16 / 20，页面一律取用，避免 13/15/18 之类的杂值
pub const SP_1: f32 = 4.0;
pub const SP_2: f32 = 8.0;
pub const SP_3: f32 = 10.0;
pub const SP_4: f32 = 12.0;
pub const SP_5: f32 = 16.0;
pub const SP_6: f32 = 20.0;

/// body 字号
pub const FS: f32 = 13.0;
/// 次要文字
pub const FS_SM: f32 = 12.0;
/// 提示 / 说明
pub const FS_XS: f32 = 11.5;
/// 卡片标题 `.home-card-head`
pub const FS_TITLE: f32 = 13.0;
/// 小节大标题
pub const FS_H: f32 = 14.0;
/// 小节眉标 `.sec-title`（10.5px 大写 + 字距）
pub const FS_SEC: f32 = 10.5;
/// 首页主标题 `.home-h1`
pub const FS_H1: f32 = 18.0;
/// 文档页大标题 `.doc-h1`
pub const FS_DOC: f32 = 22.0;

/* ═════════════════ 3. 阴影 ═════════════════ */

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

/// `--shadow-1` 面板常态：几乎不可见的 1px 下投影
pub fn shadow_1() -> Shadow {
    Shadow { color: rgba(15, 17, 21, 0.04), offset: Vector::new(0.0, 1.0), blur_radius: 2.0 }
}

/// `--shadow-2` 抬升态（空状态图标块 / 首页 CTA）
pub fn shadow_2() -> Shadow {
    Shadow { color: rgba(15, 17, 21, 0.05), offset: Vector::new(0.0, 2.0), blur_radius: 6.0 }
}

/// `--shadow-3` 浮层（下拉卡 / 对比分割线圆钮）
pub fn shadow_3() -> Shadow {
    Shadow { color: rgba(15, 17, 21, 0.06), offset: Vector::new(0.0, 8.0), blur_radius: 24.0 }
}

/// `--shadow-4` 最高层（提示条 / 模态卡）
pub fn shadow_4() -> Shadow {
    Shadow { color: rgba(15, 17, 21, 0.12), offset: Vector::new(0.0, 16.0), blur_radius: 48.0 }
}

/* ═════════════════ 4. 文本辅助 ═════════════════ */

/// 系统 Bold 字体族名（启动时由 `set_bold_family` 注入）
///
/// 原项目 CSS 大量使用 `font-weight:550~750`。iced 无法合成字重，
/// 只能命中系统里真实存在的 Bold 字面；Windows 的
/// `msyhbd.ttc` 与 `msyh.ttc` 同属 "Microsoft YaHei" 族、只差 weight，
/// 因此按 `Family::Name(族名) + Weight::Bold` 请求即可命中。
/// 若系统没有 Bold 字面，`None` 会退回常规字重（只损失层级感，不影响可读性）。
static BOLD_FAMILY: OnceLock<Option<&'static str>> = OnceLock::new();

/// 由 `main.rs` 在加载字体后调用一次
pub fn set_bold_family(name: Option<&'static str>) {
    let _ = BOLD_FAMILY.set(name);
}

/// 加粗字体；系统无 Bold 时等价于默认字体
pub fn bold_font() -> Font {
    match BOLD_FAMILY.get().copied().flatten() {
        Some(name) => Font {
            family: iced::font::Family::Name(name),
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        },
        None => Font::DEFAULT,
    }
}

/// 加粗文本（对应 CSS `font-weight:600+`）
pub fn strong<'a>(s: impl Into<String>, size: f32, color: Color) -> iced::widget::Text<'a> {
    text(s.into()).size(size).color(color).font(bold_font())
}

/// `iced::Padding` 具名构造的语法糖：padding(top, right, bottom, left)
///
/// iced 0.14 没有实现 `Padding::from([f32; 4])`，只能具名构造；
/// 这里统一提供一个短名，让布局代码的 CSS 速记顺序仍可读。
pub fn pad(top: f32, right: f32, bottom: f32, left: f32) -> iced::Padding {
    iced::Padding { top, right, bottom, left }
}

/// 四周等距的 padding
pub fn pad_all(v: f32) -> iced::Padding {
    iced::Padding { top: v, right: v, bottom: v, left: v }
}

const fn c(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}

fn border(color: Color, width: f32, radius: f32) -> Border {
    Border { color, width, radius: radius.into() }
}

/* ═════════════════ 5. 容器样式 ═════════════════ */

/// 应用底色（窗口最外层）
pub fn app_bg(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(BG())),
        ..container::Style::default()
    }
}

/// 透明容器（只做布局，不画底）
pub fn flat(_theme: &Theme) -> container::Style {
    container::Style { text_color: Some(TXT()), ..container::Style::default() }
}

/// 悬浮面板（`--r-panel` 圆角）：侧栏 / 工具条 / 舞台条 / 画布 / 检查器 / 状态栏
///
/// 对应 CSS 里成组出现的
/// `background:var(--panel); border:1px solid var(--line); box-shadow:var(--shadow-1)`。
/// iced 的 `Shadow` 只有一层，故把 CSS 阴影里的 `.5px` hairline 环交给
/// 真实的 1px border，剩下的 `0 1px 2px` 由 `shadow_1()` 承担。
pub fn panel(_theme: &Theme) -> container::Style {
    pane(R_PANEL)
}

/// 指定圆角的面板（内边距由调用方决定）
pub fn pane(radius: f32) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(PANEL())),
        border: border(LINE(), 1.0, radius),
        shadow: shadow_1(),
        snap: false,
    }
}

/// 圆角卡片（`.home-card` / `.about-card`）
pub fn card(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(PANEL())),
        border: border(LINE(), 1.0, R_CARD),
        shadow: shadow_1(),
        snap: false,
    }
}

/// `.tile` / `.home-preset`：`--panel-2` 底 + `--line` 描边 + `--r-card` 圆角
pub fn tile(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(PANEL_2())),
        border: border(LINE(), 1.0, R_CARD),
        shadow: shadow_1(),
        snap: false,
    }
}

/// `.doc` 文档页大面板（圆角 14 + 内边距 26/30）
pub fn doc(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(PANEL())),
        border: border(LINE(), 1.0, R_PANEL),
        shadow: shadow_1(),
        snap: false,
    }
}

/// 侧栏 `.sidenav`：`--panel` 浮卡（圆角 14 + hairline + shadow-1）
///
/// 对应 CSS `.sidenav{border-radius:14px; box-shadow:var(--shadow-1)}`，
/// 与通用面板同一套视觉，单独命名只是为了语义清晰与调用处易读。
pub fn sidenav(_theme: &Theme) -> container::Style {
    panel(_theme)
}

/// 状态栏 `.statusbar`：`--panel` 浮卡（`--r-panel` 圆角）
///
/// 对应 CSS `.statusbar`，悬浮在窗口底部，四周留 10px 外边距由外壳统一处理。
pub fn statusbar(_theme: &Theme) -> container::Style {
    panel(_theme)
}

/// 画布容器 `.canvas-wrap`：纯色底（棋盘 / 白 / 黑由调用方指定）+ `--line` 描边 + `--r-panel` 圆角
///
/// 对应 CSS `.canvas-wrap{border:1px solid var(--line);border-radius:14px}`，
/// 棋盘在 canvas 内部绘制，容器底色用作兜底；描边 / 圆角 / 阴影与 `.canvas-wrap` 完全一致。
pub fn stage_bg(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(color)),
        border: border(LINE(), 1.0, R_PANEL),
        shadow: shadow_1(),
        snap: false,
    }
}

/// `.hint` 提示胶囊：白底 + 描边 + 20px 全圆角
pub fn chip(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT2()),
        background: Some(Background::Color(PANEL())),
        border: border(LINE(), 1.0, 20.0),
        shadow: shadow_1(),
        snap: false,
    }
}

/// `.sec-badge` / `.kv-badge` 徽标底
pub fn badge_style(fg: Color, bg: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        text_color: Some(fg),
        background: Some(Background::Color(bg)),
        border: border(Color::TRANSPARENT, 0.0, 7.0),
        snap: false,
        ..container::Style::default()
    }
}

/// `.empty-icon` / `.tile-ic` 图标块：纯色底 + 指定圆角
pub fn icon_tile(bg: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        text_color: Some(TXT2()),
        background: Some(Background::Color(bg)),
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: radius.into() },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// 画布中「裁剪框外」的遮罩（`rgba(15,17,21,.45)`）
pub fn scrim(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(Color::WHITE),
        background: Some(Background::Color(rgba(15, 17, 21, 0.45))),
        ..container::Style::default()
    }
}

/* ═════════════════ 6. 按钮 / 控件样式 ═════════════════ */

/// 主按钮（`.btn` / `.home-cta`）：近黑实心 + 白字
pub fn btn_primary(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Active => INK(),
        button::Status::Hovered => INK_HOVER(),
        button::Status::Pressed => INK_ACTIVE(),
        button::Status::Disabled => {
            return button::Style {
                background: Some(Background::Color(INK())),
                text_color: rgba(255, 255, 255, 0.34),
                border: border(Color::TRANSPARENT, 0.0, R_BTN),
                shadow: Shadow::default(),
                snap: false,
            }
        }
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: Color::WHITE,
        border: border(Color::TRANSPARENT, 0.0, R_BTN),
        shadow: shadow_1(),
        snap: false,
    }
}

/// 次级按钮（`.btn-sm:not(.btn)`）：`--panel-3` 底 + `--line-strong` 描边
pub fn btn_secondary(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, bc, fg) = match status {
        button::Status::Active => (PANEL_3(), LINE_STRONG(), TXT()),
        button::Status::Hovered => (HOVER(), INK(), INK()),
        button::Status::Pressed => (ACTIVE(), INK(), INK()),
        button::Status::Disabled => (PANEL_3(), LINE_STRONG(), rgba(15, 17, 21, 0.34)),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: border(bc, 1.0, R_SM),
        shadow: shadow_1(),
        snap: false,
    }
}

/// 首页「开始」卡里的浅色次级按钮（`.home-start-actions .btn`）：
/// 比常规次级更淡（`--panel-2` 底），在同一张卡里与拖放区区分层级
pub fn btn_ghost(_theme: &Theme, status: button::Status) -> button::Style {
    let (bg, bc) = match status {
        button::Status::Active => (PANEL_2(), LINE_STRONG()),
        button::Status::Hovered => (HOVER(), TXT3()),
        button::Status::Pressed => (ACTIVE(), TXT3()),
        button::Status::Disabled => (PANEL_2(), LINE_STRONG()),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TXT2(),
        border: border(bc, 1.0, R_SM),
        shadow: shadow_1(),
        snap: false,
    }
}

/// 工具轨图标按钮 `.rail-btn`：透明底，悬停 `--panel-3`，选中 `--fill` + 近黑
pub fn rail_btn(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg = if active {
            FILL()
        } else {
            match status {
                button::Status::Hovered => PANEL_3(),
                button::Status::Pressed => ACTIVE(),
                _ => Color::TRANSPARENT,
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if active { INK() } else { TXT2() },
            border: border(Color::TRANSPARENT, 0.0, R_BTN),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// 工具条按钮禁用态：看图模式（对比 / 原图）下编辑类工具置灰（参考项目
/// `applyToolAvailability`：`el.disabled = noImage || viewOnly`，无悬停反馈）
pub fn rail_btn_disabled() -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, _status| button::Style {
        background: None,
        text_color: TXT3(),
        border: border(Color::TRANSPARENT, 0.0, R_BTN),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// 侧栏菜单项 `.nav-item`：悬停 `--panel-3`，选中 `--fill` + 近黑 + 加粗
pub fn nav_btn(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg = if active {
            FILL()
        } else {
            match status {
                button::Status::Hovered => PANEL_3(),
                button::Status::Pressed => ACTIVE(),
                _ => Color::TRANSPARENT,
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if active { INK() } else { TXT2() },
            border: border(Color::TRANSPARENT, 0.0, R_BTN),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// 分段控件 `.seg-group button`：选中 = **近黑实心 + 白字**（这是原设计的标志性元素）
pub fn seg_btn(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        if active {
            let bg = match status {
                button::Status::Hovered => INK_HOVER(),
                _ => INK(),
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: Color::WHITE,
                border: border(Color::TRANSPARENT, 0.0, R_SM),
                shadow: shadow_1(),
                snap: false,
            }
        } else {
            button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                text_color: match status {
                    button::Status::Hovered => TXT(),
                    _ => TXT2(),
                },
                border: border(Color::TRANSPARENT, 0.0, R_SM),
                shadow: Shadow::default(),
                snap: false,
            }
        }
    }
}

/// 检查器内的二选一分段 `.seg`：轨道已经是 `--fill`，
/// 所以选中态是「白底 + 近黑字 + 半像素描边」，与工具条的分段控件刻意不同
pub fn seg_soft_btn(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        if active {
            button::Style {
                background: Some(Background::Color(PANEL())),
                text_color: INK(),
                border: Border { color: rgba(15, 17, 21, 0.06), width: 0.5, radius: 5.0.into() },
                shadow: Shadow {
                    color: rgba(15, 17, 21, 0.10),
                    offset: Vector::new(0.0, 1.0),
                    blur_radius: 2.0,
                },
                snap: false,
            }
        } else {
            button::Style {
                background: Some(Background::Color(Color::TRANSPARENT)),
                text_color: match status {
                    button::Status::Hovered => TXT(),
                    _ => TXT2(),
                },
                border: Border::default(),
                shadow: Shadow::default(),
                snap: false,
            }
        }
    }
}

/// 色板 `.swatch`：选中时描近黑边（配合调用方的 2px 内边距形成双环）
pub fn swatch(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let (bc, bw) = if selected {
            (INK(), 1.5)
        } else {
            match status {
                button::Status::Hovered => (LINE_STRONG(), 1.0),
                _ => (LINE(), 1.0),
            }
        };
        button::Style {
            background: None,
            text_color: TXT(),
            border: border(bc, bw, R_SM),
            shadow: if selected { Shadow::default() } else { shadow_1() },
            snap: false,
        }
    }
}

/// 开关轨道 `.switch`：开 = 近黑，关 = `--line-strong`
pub fn switch(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg = if on {
            match status {
                button::Status::Hovered => INK_HOVER(),
                _ => INK(),
            }
        } else {
            match status {
                button::Status::Hovered => TXT3(),
                _ => LINE_STRONG(),
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: Color::WHITE,
            border: border(Color::TRANSPARENT, 0.0, 11.0),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// 开关滑块：白色圆形 + 极淡描边
pub fn switch_knob(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(PANEL())),
        border: border(rgba(15, 17, 21, 0.06), 0.5, 9.0),
        shadow: Shadow {
            color: rgba(15, 17, 21, 0.12),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 2.0,
        },
        snap: false,
    }
}

/// 图标方块按钮 `.cp-eyebrow` / `.icon-btn`（吸管 / 旋转等）
pub fn icon_btn(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg = if active {
            FILL()
        } else {
            match status {
                button::Status::Hovered => PANEL_3(),
                button::Status::Pressed => ACTIVE(),
                _ => Color::TRANSPARENT,
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if active { INK() } else { TXT3() },
            border: border(Color::TRANSPARENT, 0.0, 7.0),
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// 滑块 `.slider`（`--fill` 轨道 + 近黑圆钮，外圈 2px 白色描边形成「浮起」感）
///
/// CSS 的 `box-shadow:0 0 0 2px var(--panel)` 在 iced 里由 handle 的
/// `border_width` 等价实现：`radius` 是外径，描边画在内侧，
/// 所以 `radius=10 / border=2` 正好得到「16px 黑钮 + 2px 白环」。
pub fn slider_style(_theme: &Theme, _status: slider::Status) -> slider::Style {
    slider::Style {
        rail: slider::Rail {
            backgrounds: (Background::Color(LINE_STRONG()), Background::Color(LINE_STRONG())),
            width: 4.0,
            border: border(Color::TRANSPARENT, 0.0, 4.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 10.0 },
            background: Background::Color(INK()),
            border_width: 2.0,
            border_color: PANEL(),
        },
    }
}

/// 提示条 `.toast` 的深色胶囊
pub fn toast_ink(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(Color::WHITE),
        background: Some(Background::Color(INK())),
        border: border(Color::TRANSPARENT, 0.0, R_CARD),
        shadow: shadow_4(),
        snap: false,
    }
}

/// 悬停提示 `.tip` / `.rail-btn .tip`：ink 底 + `--r-sm` 圆角 + `--shadow-3` + 白字
///
/// 工具条图标按钮（`.rail-btn .tip`，`top:calc(100% + 9px)`）与侧栏折叠态提示
/// 共用同一套视觉；文字尺寸由调用方给定（参考为 11.5px）。
pub fn tooltip_ink(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(Color::WHITE),
        background: Some(Background::Color(INK())),
        border: border(Color::TRANSPARENT, 0.0, R_SM),
        shadow: shadow_3(),
        snap: false,
    }
}

/// 失败提示条 `.toast.err`
pub fn toast_err(theme: &Theme) -> container::Style {
    container::Style { background: Some(Background::Color(DANGER())), ..toast_ink(theme) }
}

/// 下拉菜单 `.pick_list`（默认态：白底 + `--line` 描边 + `--r-sm` 圆角）
pub fn pick_list_style(_theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    match status {
        pick_list::Status::Active => pick_list::Style {
            background: Background::Color(PANEL()),
            text_color: TXT(),
            border: border(LINE_STRONG(), 1.0, R_SM),
            placeholder_color: TXT2(),
            handle_color: TXT2(),
        },
        pick_list::Status::Hovered => pick_list::Style {
            background: Background::Color(HOVER()),
            text_color: INK(),
            border: border(INK(), 1.0, R_SM),
            placeholder_color: INK(),
            handle_color: INK(),
        },
        pick_list::Status::Opened { .. } => pick_list::Style {
            background: Background::Color(PANEL()),
            text_color: TXT(),
            border: border(ACC(), 1.0, R_SM),
            placeholder_color: ACC(),
            handle_color: ACC(),
        },
    }
}

/// 下拉弹层 `.menu`：白底 + `--line` hairline 描边 + `--r-sm` 圆角 + `--shadow-3`
/// 浮层阴影；选中项浅灰底（`--fill`）+ 近黑字。与字段样式（`pick_list_style`）
/// 成对使用，保证「收起框」与「展开层」视觉一致。
pub fn menu_style(_theme: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(PANEL()),
        border: border(LINE(), 1.0, R_SM),
        text_color: TXT(),
        selected_text_color: INK(),
        selected_background: Background::Color(FILL()),
        shadow: shadow_3(),
    }
}

/// 文本输入框 `.text-input`：白底 + `--line-strong` 描边 + `--r-sm` 圆角
///
/// 默认态：`--panel` 底 + `--line-strong` 描边
/// 悬停态：`--hover` 底 + `--ink` 描边
/// 聚焦态：`--panel` 底 + `--acc` 描边（强调色焦点环）
/// 禁用态：`--fill` 底 + `--line` 描边 + 半透明文字
pub fn text_input_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    match status {
        text_input::Status::Active => text_input::Style {
            background: Background::Color(PANEL()),
            border: border(LINE_STRONG(), 1.0, R_SM),
            icon: TXT3(),
            placeholder: TXT3(),
            value: TXT(),
            selection: ACC_SOFT(),
        },
        text_input::Status::Hovered => text_input::Style {
            background: Background::Color(HOVER()),
            border: border(INK(), 1.0, R_SM),
            icon: TXT2(),
            placeholder: TXT2(),
            value: INK(),
            selection: ACC_SOFT(),
        },
        text_input::Status::Focused { is_hovered: _ } => text_input::Style {
            background: Background::Color(PANEL()),
            border: border(ACC(), 1.0, R_SM),
            icon: TXT2(),
            placeholder: TXT3(),
            value: TXT(),
            selection: ACC_SOFT(),
        },
        text_input::Status::Disabled => text_input::Style {
            background: Background::Color(FILL()),
            border: border(LINE(), 1.0, R_SM),
            icon: TXT3(),
            placeholder: TXT3(),
            value: Color { a: 0.34, ..TXT() },
            selection: ACC_SOFT(),
        },
    }
}

/* ═════════════════ 7. 滚动条样式（对齐参考项目 base.css 滚动条） ═════════════════ */

/// 滚动条轨道与滑块颜色（对应参考项目 `rgba(15,17,21,.20)` 与 `.32`）
const SCROLLBAR_THUMB: Color = rgba(15, 17, 21, 0.20);
const SCROLLBAR_THUMB_HOVER: Color = rgba(15, 17, 21, 0.32);

/// 3px 细滚动条：默认透明，悬停容器时滑块淡入，直接悬停滑块时加深。
///
/// 参考 CSS：
/// ```css
/// * { scrollbar-width: thin; scrollbar-color: transparent transparent; }
/// *:hover { scrollbar-color: rgba(15,17,21,.20) transparent; }
/// ::-webkit-scrollbar { width: 3px; height: 3px; background: transparent; }
/// ::-webkit-scrollbar-track { background: transparent; }
/// ::-webkit-scrollbar-thumb { background: transparent; border-radius: 99px; }
/// *:hover::-webkit-scrollbar-thumb { background: rgba(15,17,21,.20); }
/// *:hover::-webkit-scrollbar-thumb:hover { background: rgba(15,17,21,.32); }
/// ```
///
/// iced 0.14 的 `scrollable::Status` 提供三种状态：
/// - `Active`：未悬停 → 滑块透明
/// - `Hovered { is_horizontal_scrollbar_hovered, is_vertical_scrollbar_hovered, .. }`
///   → 容器被悬停，滑块变为 `SCROLLBAR_THUMB`；若具体滑块被悬停则变为
///   `SCROLLBAR_THUMB_HOVER`
/// - `Dragged`：拖拽中 → 保持 `SCROLLBAR_THUMB_HOVER`
pub fn scrollbar_style(_theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let thumb_color = match status {
        scrollable::Status::Active { .. } => Color::TRANSPARENT,
        scrollable::Status::Hovered {
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
            ..
        } => {
            if is_horizontal_scrollbar_hovered || is_vertical_scrollbar_hovered {
                SCROLLBAR_THUMB_HOVER
            } else {
                SCROLLBAR_THUMB
            }
        }
        scrollable::Status::Dragged { .. } => SCROLLBAR_THUMB_HOVER,
    };

    let scroller = scrollable::Scroller {
        background: Background::Color(thumb_color),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 99.0.into(),
        },
    };

    let rail = scrollable::Rail {
        background: None,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 99.0.into(),
        },
        scroller,
    };

    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: Color::TRANSPARENT,
        },
    }
}