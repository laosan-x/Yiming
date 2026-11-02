//! 外壳：左侧导航栏 + 底部状态栏
//!
//! 对应原项目 `.sidenav`（品牌 / 5 个导航项 / 收起 / 版本引擎）与 `.statusbar`。

use std::sync::OnceLock;

use iced::widget::image::Handle;
use iced::widget::{button, column, container, image, row, text, Space};
use iced::widget::tooltip;
use iced::{Alignment, Background, Border, Color, Element, Length, Shadow};

use crate::app::App;
use crate::message::{Message, Page, SponsorMessage, TitlebarMessage};
use crate::theme::*;
use crate::ui::icons::icon;
use crate::core::i18n::{tr, trf};

/* ── 导航项几何常量（参考 sidenav.css） ──
   `.nav-item svg{width:17px;height:17px}` / `.nav-item{gap:10px;height:36px}`
   展开态、收起态、收起按钮共用同一组值，避免出现 17 / 18 混用的偏差。 */
const NAV_ICON: f32 = 17.0;
const NAV_GAP: f32 = 10.0;

/// 品牌图标：`icons/128x128.png`（128×128 对 34/38px 的显示尺寸绰绰有余），
/// `include_bytes!` 内嵌 —— 不依赖运行时工作目录，发布单文件也可用。
/// 128×128 只解码一次，各处共用同一 Handle（iced 按 Handle 缓存纹理）。
fn brand_handle() -> Handle {
    static HANDLE: OnceLock<Handle> = OnceLock::new();
    HANDLE
        .get_or_init(|| Handle::from_bytes(include_bytes!("../../icons/128x128.png").to_vec()))
        .clone()
}

/// 品牌圆徽：INK() 圆底（`logo(true, size)` 样式）+ 白色公鸡线稿，
/// 对应参考项目 `.nav-brand` 里的 `<img>`。`inner` 是线稿的显示边长，
/// 比圆底小 4px，让线稿自带的白色圆环落在圆底内部。
fn brand_badge<'a>(inner: f32) -> Element<'a, Message> {
    let size = inner + 4.0;
    container(
        image(brand_handle())
            .width(Length::Fixed(inner))
            .height(Length::Fixed(inner)),
    )
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .center_x(Length::Fixed(size))
    .center_y(Length::Fixed(size))
    .style(logo(true, size))
    .into()
}

/* ═════════════════ 侧栏 ═════════════════ */

pub fn sidenav<'a>(app: &'a App) -> Element<'a, Message> {
    let collapsed = app.nav_collapsed;
    let w = if collapsed { NAV_W_COLLAPSED } else { NAV_W };

    // ── 品牌 ──
    let brand: Element<'a, Message> = if collapsed {
        brand_badge(34.0)
    } else {
        row![
            brand_badge(30.0),
            column![
                strong("壹鸣", FS_H, TXT()),
                text("Wonderland").size(FS_XS).color(TXT3()),
            ]
            .spacing(0)
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    };

    // `.nav-brand`：展开态 `padding:0 4px`；折叠态参考 CSS 是
    // `.sidenav.collapsed .nav-brand{justify-content:center;padding:0}` ——
    // 必须去掉左右 padding 并居中，否则折叠宽 60 - 侧栏 padding 20 = 40 的
    // 内容区再被 brand_block 吃掉 8px，38px 的圆徽会被水平夹成 32px（蛋形）。
    let brand_block = if collapsed {
        container(brand).width(Length::Fill).center_x(Length::Fill)
    } else {
        container(brand).padding(pad(0.0, 4.0, 0.0, 4.0))
    };

    // 导航项内边距：展开态 `0 10px`（参考 `.nav-item{padding:0 10px}`），折叠态 0
    let item_pad = if collapsed { pad_all(0.0) } else { pad(0.0, 10.0, 0.0, 10.0) };

    // ── 导航组 `.nav-group`（gap:2px） ──
    let mut nav_group: iced::widget::Column<'a, Message> = column![].spacing(2);
    for page in Page::ALL {
        let active = app.page == page;
        let icon_name = match page {
            Page::Home => "home",
            Page::Work => "image-plus",
            Page::IdPhoto => "playing-cards-fan",
            Page::Settings => "settings",
            Page::About => "info",
        };
        let ic_color = if active { TXT() } else { TXT2() };
        let inner: Element<'a, Message> = if collapsed {
            // 折叠态只留图标：参考 `.sidenav.collapsed .nav-item{justify-content:center}` → 居中
            nav_center(icon(icon_name, NAV_ICON, ic_color))
        } else {
            // 展开态：参考**构建产物** `.nav-item{display:flex;align-items:center;gap:10px;
            // padding:0 10px}`（无 justify-content，即 flex-start）→ 垂直居中、水平左对齐
            nav_line(
                row![
                    icon(icon_name, NAV_ICON, ic_color),
                    if active {
                        strong(tr(page.label()), FS, TXT())
                    } else {
                        text(tr(page.label())).size(FS).color(TXT2())
                    },
                ]
                .spacing(NAV_GAP)
                .align_y(Alignment::Center),
            )
        };
        // `.nav-item{height:36px}`；hover=panel-3 / selected=fill+ink+bold（见 nav_btn）
        nav_group = nav_group.push(
            button(inner)
                .on_press(Message::Navigate(page))
                .width(Length::Fill)
                .height(Length::Fixed(36.0))
                .padding(item_pad)
                .style(nav_btn(active)),
        );
    }

    // ── 消息中心入口（铃铛 + 未读徽标；折叠态只留图标 + 圆点） ──
    let unread_badge: Element<'a, Message> = if app.unread > 0 {
        container(
            text(if app.unread > 99 { "99+".to_string() } else { app.unread.to_string() })
                .size(9.0)
                .color(iced::Color::WHITE),
        )
        .width(16.0)
        .height(16.0)
        .center_x(Length::Fixed(16.0))
        .center_y(Length::Fixed(16.0))
        .style(|_t| container::Style {
            text_color: None,
            background: Some(iced::Background::Color(DANGER())),
            border: iced::Border { color: PANEL(), width: 1.0, radius: 8.0.into() },
            shadow: iced::Shadow::default(),
            snap: false,
        })
        .into()
    } else {
        Space::new().width(0.0).height(0.0).into()
    };
    let bell_inner: Element<'a, Message> = if collapsed {
        nav_center({
            let bell_el: Element<'a, Message> = container(icon("bell", NAV_ICON, TXT2()))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into();
            let badge_el: Element<'a, Message> = container(unread_badge)
                .align_x(Alignment::End)
                .align_y(Alignment::Start)
                .width(Length::Fill)
                .height(Length::Fill)
                .into();
            let layer: Element<'a, Message> =
                iced::widget::Stack::with_children(vec![bell_el, badge_el])
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into();
            layer
        })
    } else {
        nav_line(
            row![
                icon("bell", NAV_ICON, if app.unread > 0 { TXT() } else { TXT2() }),
                text(crate::core::i18n::tr("msg.center"))
                    .size(FS)
                    .color(TXT2()),
                Space::new().width(Length::Fill),
                unread_badge,
            ]
            .spacing(NAV_GAP)
            .align_y(Alignment::Center),
        )
    };
    let bell_btn = button(bell_inner)
        .on_press(Message::ToggleMsgCenter)
        .width(Length::Fill)
        .height(Length::Fixed(36.0))
        .padding(item_pad)
        .style(nav_btn(app.msg_center_open));

    // ── 收起 / 展开 `.nav-collapse`（与菜单项同一套视觉，height:36px） ──
    // 参考 `.nav-collapse` 展开态带 `justify-content:center`（与 `.nav-item` 不同）→ 保持居中
    let collapse_inner: Element<'a, Message> = if collapsed {
        nav_center(icon("chevron-right", NAV_ICON, TXT2()))
    } else {
        nav_center(
            row![
                icon("chevron-left", NAV_ICON, TXT2()),
                text(tr("收起侧栏")).size(FS).color(TXT2()),
            ]
            .spacing(NAV_GAP)
            .align_y(Alignment::Center),
        )
    };
    let collapse_btn = button(collapse_inner)
        .on_press(Message::ToggleNav)
        .width(Length::Fill)
        .height(Length::Fixed(36.0))
        .padding(item_pad)
        .style(nav_btn(false));

    // ── 底部版本 / 引擎 `.nav-foot`（border-top:1px line-soft + padding-top:10 + gap:6） ──
    let foot: Element<'a, Message> = if collapsed {
        column![].into()
    } else {
        column![
            container(Space::new().width(Length::Fill).height(Length::Fixed(1.0)))
                .style(|_t| container::Style {
                    background: Some(iced::Background::Color(LINE_SOFT())),
                    ..container::Style::default()
                }),
            column![
                row![
                    text(tr("版本")).size(FS_XS).color(TXT3()),
                    Space::new().width(Length::Fill),
                    text(env!("CARGO_PKG_VERSION")).size(FS_XS).color(TXT2()),
                ],
                row![
                    text(tr("引擎")).size(FS_XS).color(TXT3()),
                    Space::new().width(Length::Fill),
                    text(tr("本地")).size(FS_XS).color(TXT2()),
                ],
            ]
            .spacing(6)
            .padding(pad(0.0, 8.0, 0.0, 8.0)),
        ]
        .spacing(10)
        .into()
    };

    // 侧栏 = 品牌组 / 导航组 / 收起 / 消息 / 页脚，组间距 16px（参考 `.sidenav{gap:16px}`）
    // 消息按钮在收起按钮下方（应用户要求调整布局顺序）
    container(
        column![
            brand_block,
            nav_group,
            Space::new().height(Length::Fill),
            collapse_btn,
            bell_btn,
            foot,
        ]
        .spacing(16),
    )
    .padding(pad(12.0, 10.0, 12.0, 10.0))
    .width(Length::Fixed(w))
    .height(Length::Fill)
    .style(crate::theme::sidenav)
    .into()
}

/// 导航按钮内容容器（**左对齐**，垂直居中）—— 展开态菜单项用
///
/// iced 的 `Button::layout` 走 `layout::padded()`，内容节点被直接放到
/// `(padding.left, padding.top)`，既不会水平居中也不会垂直居中。
/// 参考实现的**构建产物** `dist/assets/index-*.css` 里
/// `.nav-item{width:100%;height:36px;padding:0 10px;display:flex;align-items:center;gap:10px}`
/// 没有 `justify-content`（即 `flex-start`），所以图标左端落在 padding 10 处、
/// 文字落在 10+17+10=37 处（像素实测与之一致）。这里用「Fill + 水平 Start +
/// 垂直 Center」的 container 复刻该行为。
fn nav_line<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Start)
        .align_y(Alignment::Center)
        .into()
}

/// 导航按钮内容容器（**双轴居中**）—— 折叠态菜单项与「收起侧栏」用
///
/// 对应参考构建产物里的 `.sidenav.collapsed .nav-item{...justify-content:center}`
/// 与 `.nav-collapse{...justify-content:center}`（这两条确实带 center）。
fn nav_center<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/* ═════════════════ 状态栏 ═════════════════ */

pub fn statusbar<'a>(app: &'a App) -> Element<'a, Message> {
    let m = &app.work.metrics;

    let time = if m.elapsed_ms > 0.0 {
        format!("{:.0} ms", m.elapsed_ms)
    } else {
        "—".to_string()
    };
    let ratio = if m.w > 0 {
        format!("{:.1}%", m.fg_ratio * 100.0)
    } else {
        "—".to_string()
    };
    let res = if m.w > 0 {
        format!("{}×{}", m.w, m.h)
    } else {
        "—".to_string()
    };
    let holes = if m.w > 0 { format!("{}", m.holes) } else { "—".to_string() };
    let state = if app.work.busy { tr("计算中") } else { tr("就绪") };

    // 左侧消息（参考 `stMsg`）：工作台已载入图片时显示「已载入 名 · W×H」，
    // 其余页面显示页面名
    let st_msg = if app.page == Page::Work && m.w > 0 {
        trf("已载入 {} · {}×{}", &[&app.work.stem, &m.w.to_string(), &m.h.to_string()])
    } else {
        tr(app.page.label()).to_string()
    };

    container(
        row![
            text(st_msg).size(FS_XS).color(TXT2()),
            Space::new().width(Length::Fill),
            item(tr("耗时"), time, true),
            item(tr("前景"), ratio, false),
            item(tr("尺寸"), res, false),
            item(tr("补孔"), holes, false),
            text(state).size(FS_XS).color(OK()),
            Space::new().width(12.0),
            sb_div(),
            Space::new().width(12.0),
            text(&app.cpu_text).size(FS_XS).color(TXT3()),
            text(&app.mem_text).size(FS_XS).color(TXT3()),
            row![
                icon("dot", 8.0, OK()),
                text(tr("本地推理")).size(FS_XS).color(TXT2())
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        ]
        .spacing(14)
        .align_y(Alignment::Center)
        .padding(pad(0.0, 14.0, 0.0, 14.0)),
    )
    .width(Length::Fill)
    .height(Length::Fixed(STATUSBAR_H))
    .align_y(iced::alignment::Vertical::Center)
    .style(crate::theme::statusbar)
    .into()
}

/// 状态栏的「键 值」单元；v 按值接收，避免把局部变量借进 Element
fn item<'a>(k: &'a str, v: String, acc: bool) -> Element<'a, Message> {
    row![
        text(k).size(FS_XS).color(TXT3()),
        text(v).size(FS_XS).color(if acc { ACC() } else { TXT2() }),
    ]
    .spacing(5)
    .align_y(Alignment::Center)
    .into()
}

/// 竖向分隔线（参考 `.sb-div`：1px 宽、14px 高的 `--line` 竖线）
fn sb_div() -> Element<'static, Message> {
    container(Space::new().width(Length::Fixed(1.0)).height(Length::Fixed(14.0)))
        .style(|_t| container::Style {
            background: Some(Background::Color(LINE())),
            ..container::Style::default()
        })
        .into()
}

/* ═════════════════ 局部样式 ═════════════════ */

fn logo(dark: bool, size: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme| container::Style {
        text_color: Some(if dark { Color::WHITE } else { TXT() }),
        background: Some(Background::Color(if dark { INK() } else { PANEL_3() })),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: (size / 2.0).into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}
/* ═════════════════ 自绘标题栏 ═════════════════ */

/// 标题栏窗口控制钮样式：悬停浅灰底（Win 风格方形按钮）
fn win_btn_style(_t: &iced::Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    match s {
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
            iced::widget::button::Style {
                background: Some(Background::Color(HOVER())),
                ..iced::widget::button::Style::default()
            }
        }
        _ => iced::widget::button::Style::default(),
    }
}

/// 咖啡（打赏）钮样式：纯图标按钮——无底色无描边，仅悬停时浅灰底提示可点
fn sponsor_btn_style(_t: &iced::Theme, s: iced::widget::button::Status) -> iced::widget::button::Style {
    match s {
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
            iced::widget::button::Style {
                background: Some(Background::Color(HOVER())),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 6.0.into(),
                },
                ..iced::widget::button::Style::default()
            }
        }
        _ => iced::widget::button::Style::default(),
    }
}

/// 标题栏上的窗口控制钮（36 宽 × 撑满 32 高，字形双轴居中）
///
/// iced 的 button 内容布局是 padded 左上角原点、**不会自动居中**——
/// 必须用 nav_center 同款手法：内容包 Fill×Fill + center 容器，
/// 否则悬停底铺满全钮而字形贴左上，看起来「不居中/变形」。
/// square（最大化）墨迹为满幅 18/24，视觉偏大，单独收到 15px。
fn win_ctrl<'a>(
    name: &'static str,
    msg: Message,
    style: fn(&iced::Theme, iced::widget::button::Status) -> iced::widget::button::Style,
) -> Element<'a, Message> {
    let px = match name {
        "square" => 13.0,
        _ => NAV_ICON,
    };
    button(nav_center(icon(name, px, TXT2())))
        .width(Length::Fixed(36.0))
        .height(Length::Fill)
        .style(style)
        .on_press(msg)
        .into()
}

/// 自绘标题栏（window decorations 已关闭）：
/// 左侧品牌 + 窗口标题 = 拖动区（按下拖动 / 双击最大化切换），
/// 右侧打赏咖啡钮 + 最小化 / 最大化还原 / 关闭（Win 风格字形）。
pub fn titlebar(app: &App) -> Element<'_, Message> {
    use iced::widget::mouse_area;

    let title_text = app.title();
    // 纵向居中规范：标题栏所有子元素一律 height(Fill) 撑满整条栏，
    // 由各控件内部把内容垂直居中——栏高变化时无需改动即保持居中
    let drag_area = mouse_area(container(
        row![
            brand_badge(16.0),
            text(title_text).size(FS_XS).color(TXT2()),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(pad(0.0, 12.0, 0.0, 12.0))
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Alignment::Center))
    .on_press(Message::Titlebar(TitlebarMessage::Drag))
    .on_double_click(Message::Titlebar(TitlebarMessage::ToggleMaximize));

    // 打赏入口：咖啡图标（黑白灰，与窗口控制一致）；已展开时点击 = 关闭
    let sponsor_msg = if app.sponsor_open {
        SponsorMessage::Open(false)
    } else {
        SponsorMessage::Open(true)
    };
    let sponsor_btn = tooltip(
        button(nav_center(icon("coffee", NAV_ICON, TXT2())))
            .width(Length::Fixed(36.0))
            .height(Length::Fill)
            .style(sponsor_btn_style)
            .on_press(Message::Sponsor(sponsor_msg)),
        container(text(tr("请作者喝杯咖啡")).size(FS_XS).color(TXT()))
            .padding(pad(5.0, 10.0, 5.0, 10.0))
            .style(|_t| container::Style {
                background: Some(Background::Color(PANEL())),
                border: Border {
                    color: LINE(),
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }),
        tooltip::Position::Bottom,
    );

    let controls = row![
        sponsor_btn,
        win_ctrl("minus", Message::Titlebar(TitlebarMessage::Minimize), win_btn_style),
        win_ctrl(
            if app.win_maximized { "copy" } else { "square" },
            Message::Titlebar(TitlebarMessage::ToggleMaximize),
            win_btn_style,
        ),
        win_ctrl("x", Message::Titlebar(TitlebarMessage::Close), win_btn_style),
    ]
    .height(Length::Fill)
    .align_y(Alignment::Center);

    // 行高必须显式 Fill：Shrink 行内 Fill 子项无法解析，内容会与按钮错位；
    // 撑满 32px 后品牌徽 / 标题 / 按钮全部落在同一垂直中线
    container(row![drag_area, controls].height(Length::Fill).align_y(Alignment::Center))
        .width(Length::Fill)
        .height(Length::Fixed(32.0))
        .style(|_t| container::Style {
            background: Some(Background::Color(BG())),
            ..container::Style::default()
        })
        .into()
}

/* ═════════════════ 窗口圆角 / 阴影恢复 ═════════════════ */

/// Win11：给无装饰窗口（decorations: false）恢复系统圆角 + 阴影。
///
/// 原理：`DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, ROUND)`
/// 让 DWM 对本窗口应用 Win11 圆角裁剪，并附带标准窗口阴影与 1px 描边。
/// iced 不暴露 HWND，这里按「当前进程 + 窗口标题匹配」枚举顶层窗口定位。
/// 找不到或设置失败返回 false（调用方在后续 resize 事件时重试）。
#[cfg(windows)]
pub fn apply_window_rounding(title: &str) -> bool {
    use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    };

    static TARGET_PID: AtomicU32 = AtomicU32::new(0);
    static FOUND: AtomicIsize = AtomicIsize::new(0);
    static TITLE: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();

    let _ = TITLE.set(title.encode_utf16().collect::<Vec<u16>>());
    TARGET_PID.store(std::process::id(), Ordering::SeqCst);
    FOUND.store(0, Ordering::SeqCst);

    unsafe extern "system" fn find(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        let mut pid = 0u32;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid != TARGET_PID.load(Ordering::SeqCst) || !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1); // 继续枚举
        }
        let want = TITLE.get().map(|v| v.as_slice()).unwrap_or(&[]);
        if want.is_empty() {
            return BOOL(1);
        }
        let mut buf = [0u16; 128];
        let len = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        let n = want.len();
        if len >= n && n > 0 && buf[..n] == want[..n] {
            FOUND.store(hwnd.0 as isize, Ordering::SeqCst);
            return BOOL(0); // 找到即停
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(find), LPARAM(0));
        let raw = FOUND.load(Ordering::SeqCst);
        if raw == 0 {
            return false;
        }
        let hwnd = HWND(raw as *mut _);
        let pref = DWMWCP_ROUND.0; // 2：Win11 圆角（附带阴影与 1px 描边）
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        )
        .is_ok()
    }
}

/// 非 Windows 平台无 DWM，保持原样
#[cfg(not(windows))]
pub fn apply_window_rounding(_title: &str) -> bool {
    true
}
