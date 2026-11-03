//! 打赏浮层（请作者喝杯咖啡）
//!
//! 由标题栏右侧的咖啡图标唤起，采用**停靠式浮层**（同消息通知交互）：
//! - 从窗口右缘（标题栏下方）滑入停靠，不遮挡页面内容，无压暗遮罩
//! - 入场动画：横向滑入 28px + 就位，16ms/帧；浮层外点击任意处关闭
//! - 图标一律黑白灰（不用强调色），与系统原生风格一致
//! - 国内打赏引导到「其他支持渠道」的爱发电（支持微信/支付宝），
//!   不再内置收款码图片（原 content/sponsor/{wechat,alipay}.png 方案已移除）
//! - 金额仅作参考提示，不参与任何支付流程
//!
//! 引导策略（见 docs/SPONSORSHIP.md）：
//! ① 价值先行：免费/无广告/本地隐私三件套标签 + 「支持持续开发」话术
//! ② 小额心理账户：¥6.6 起 + 「随心就好」+ 0 元 Star 低门槛出口（star_url 配置）
//! ③ 透明承诺：固定脚注「打赏不升级、不解锁、不影响任何功能」
//! ⑤ 里程碑目标：links.json `goal` 配置驱动，默认关闭、整区不渲染
//! ⑥ 多语言：全部文案走 tr/trf；渠道 label / 目标名由配置自带（可任意语言）

use iced::widget::{button, column, container, mouse_area, row, stack, text, text_input, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};
use std::sync::OnceLock;

use crate::app::App;
use crate::core::i18n::{tr, trf};
use crate::message::{Message, SponsorMessage};
use crate::theme::*;
use crate::ui::icons::icon;

/// 建议金额档位（元）
const PRESETS: &[f32] = &[6.6, 9.9, 18.8, 66.0];

/// 浮层停靠位置（与消息面板的 20px 间距同规范，方向为垂直）：
/// 咖啡钮底缘 = 窗口顶 + 标题栏 32；内容区顶 = 32 + BODY_PAD(10)。
/// 间隔 = (42 + DOCK_TOP) − 32 = 10 + DOCK_TOP → DOCK_TOP = 10 得 20px。
///
/// 右偏推导：咖啡钮右缘 = 窗口右缘 − 3×36（窗口控制钮，标题栏无外边距）；
/// 内容区右缘 = 窗口右缘 − BODY_PAD(10)。对齐咖啡钮右缘 → 108 − 10 = 98。
const DOCK_TOP: f32 = 10.0;
const DOCK_RIGHT: f32 = 98.0;
/// 滑入距离（px）
const SLIDE: f32 = 28.0;

/* ── 打赏配置（links.json 配置化）── */

/// 国际/平台赞助渠道：label 展示文案（可任意语言）、icon 图标名（icons 表内）、
/// url 跳转地址。channels 为空 / 文件缺失 / 解析失败时 UI 自动隐藏该区。
#[derive(serde::Deserialize, Clone)]
pub struct LinkChannel {
    pub label: String,
    pub url: String,
    #[serde(default = "default_icon")]
    pub icon: String,
}

fn default_icon() -> String {
    "globe".to_string()
}

/// 里程碑目标（P2，默认关闭）：给捐赠一个具体用途叙事。
/// `enabled: false`（默认）时整区不渲染，零打扰。
#[derive(serde::Deserialize, Clone)]
pub struct MilestoneGoal {
    /// 用途叙事（如「覆盖模型托管费用」，可任意语言）
    pub label: String,
    /// 目标金额（元）
    pub target: f64,
    /// 当前已筹（元）
    #[serde(default)]
    pub current: f64,
    /// 默认关闭
    #[serde(default)]
    pub enabled: bool,
}

/// links.json 顶层结构（全部字段可选，向后兼容）
#[derive(serde::Deserialize, Clone, Default)]
pub struct SponsorConfig {
    #[serde(default)]
    pub channels: Vec<LinkChannel>,
    /// GitHub 仓库地址（「0 元的 Star」入口；缺省则隐藏该行）
    #[serde(default)]
    pub star_url: Option<String>,
    #[serde(default)]
    pub goal: Option<MilestoneGoal>,
}

static SPONSOR_CFG: OnceLock<SponsorConfig> = OnceLock::new();

fn sponsor_config() -> &'static SponsorConfig {
    SPONSOR_CFG.get_or_init(|| {
        // exe 同目录 → 工作目录 → 内嵌默认（改外部文件重启生效）
        let rel = std::path::Path::new("content/sponsor/links.json");
        let bytes: Option<Vec<u8>> = (|| {
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    if let Ok(b) = std::fs::read(dir.join(rel)) {
                        return Some(b);
                    }
                }
            }
            std::fs::read(rel).ok()
        })();
        let src = bytes
            .unwrap_or_else(|| include_bytes!("../../content/sponsor/links.json").to_vec());
        serde_json::from_slice(&src).unwrap_or_default()
    })
}

/// 金额展示：整数去小数点，非整保留 1 位
fn fmt_amount(v: f32) -> String {
    if (v - v.round()).abs() < 0.001 {
        format!("¥{}", v as i64)
    } else {
        format!("¥{v:.1}")
    }
}

/// 浮层卡片样式（PANEL 底 + hairline + 大圆角 + 最高层阴影；文字主题灰）
fn card_style(_t: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(Background::Color(PANEL())),
        border: Border {
            color: LINE(),
            width: 1.0,
            radius: R_PANEL.into(),
        },
        shadow: shadow_4(),
        snap: false,
    }
}

/// 金额 chip：未选中（黑白灰 hairline）
fn chip_style(_t: &Theme, s: button::Status) -> button::Style {
    match s {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Background::Color(HOVER())),
            border: Border {
                color: LINE_STRONG(),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        },
        _ => button::Style {
            border: Border {
                color: LINE(),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        },
    }
}

/// 金额 chip：选中（黑白反色——墨底浅字，不用强调色）
fn chip_sel_style(_t: &Theme, _s: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(TXT())),
        border: Border {
            color: TXT(),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..button::Style::default()
    }
}

/// 浮层关闭钮：幽灵样式（透明底 + 主题灰图标，悬停浅灰底）——修复此前
/// 误用窗口关闭钮红色悬停底导致的「红方块」观感
fn ghost_close_style(_t: &Theme, s: button::Status) -> button::Style {
    match s {
        button::Status::Hovered | button::Status::Pressed => button::Style {
            background: Some(Background::Color(HOVER())),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        },
        _ => button::Style::default(),
    }
}

/// 打赏浮层：右缘停靠（标题栏下方），横向滑入；浮层外点击关闭
pub fn modal(app: &App) -> Element<'_, Message> {
    let a = app.sponsor_anim.clamp(0.0, 1.0);
    // 浮层宽：固定 360，窄窗钳制
    let card_w = (app.win_w - 2.0 * BODY_PAD - 12.0).clamp(300.0, 360.0);

    // 透明拦截层：浮层外点击任意处关闭（不绘制遮罩，页面不被压暗）
    let scrim = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(Message::Sponsor(SponsorMessage::Open(false)));

    // 头部：咖啡图标（黑白）+ 标题副标 + 幽灵关闭钮
    let head = row![
        icon("coffee", 16.0, TXT2()),
        column![
            text(tr("请作者喝杯咖啡")).size(FS_H).color(TXT()).font(bold_font()),
            text(tr("打赏完全自愿 · 壹鸣永久免费")).size(FS_XS).color(TXT3()),
        ]
        .spacing(1),
        Space::new().width(Length::Fill),
        button(icon("x", 16.0, TXT3()))
            .width(Length::Fixed(24.0))
            .height(Length::Fixed(24.0))
            .style(ghost_close_style)
            .on_press(Message::Sponsor(SponsorMessage::Open(false))),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    // ── ① 价值先行：「免费 · 无广告 · 本地隐私」三件套标签（黑白灰 hairline）──
    let tag = |label: &'static str| {
        container(text(tr(label)).size(FS_XS).color(TXT2()))
            .padding(pad(2.0, 8.0, 2.0, 8.0))
            .style(|_t| container::Style {
                border: Border {
                    color: LINE(),
                    width: 1.0,
                    radius: 9.0.into(),
                },
                ..container::Style::default()
            })
    };
    let trio = row![tag("免费"), tag("无广告"), tag("本地隐私")]
        .spacing(6)
        .align_y(Alignment::Center);

    // 主文案：话术落在「支持持续开发」而非「救济作者」
    let copy = text(tr(
        "壹鸣的所有功能完全免费、没有广告，图片处理全程留在你的电脑上，不上传任何数据。如果它帮到了你，欢迎支持持续开发——让每一个新功能都稳定到来。",
    ))
    .size(FS)
    .color(TXT2())
    .width(Length::Fill);

    // ── ② 心理账户：0 元的低门槛出口（Star），放在所有金额选项之前 ──
    let star_sec: Option<Element<'_, Message>> = sponsor_config().star_url.as_ref().map(|url| {
        row![
            icon("star", 12.0, TXT2()),
            text(tr("0 元的 Star 也是很好的支持。"))
                .size(FS_SM)
                .color(TXT2()),
            Space::new().width(Length::Fill),
            button(
                row![icon("star", 11.0, TXT2()), text(tr("去点 Star")).size(FS_XS).color(TXT2())]
                    .spacing(4)
                    .align_y(Alignment::Center),
            )
            .padding(pad(3.0, 9.0, 3.0, 9.0))
            .style(chip_style)
            .on_press(Message::OpenUrl(url.clone())),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
    });

    // 链接渠道 chips（links.json 配置驱动；配置为空则整区隐藏）
    let links = &sponsor_config().channels;
    let links_section: Option<Element<'_, Message>> = if links.is_empty() {
        None
    } else {
        let mut chips_row = row![].spacing(6).align_y(Alignment::Center);
        for ch in links {
            chips_row = chips_row.push(
                button(
                    row![
                        icon(&ch.icon, 12.0, TXT2()),
                        text(&ch.label).size(FS_XS).color(TXT2()),
                    ]
                    .spacing(5)
                    .align_y(Alignment::Center),
                )
                .padding(pad(4.0, 9.0, 4.0, 9.0))
                .style(chip_style)
                .on_press(Message::OpenUrl(ch.url.clone())),
            );
        }
        Some(
            column![
                text(tr("其他支持渠道")).size(FS_XS).color(TXT3()),
                chips_row,
            ]
            .spacing(4)
            .width(Length::Fill)
            .into(),
        )
    };

    // ── ⑤ 里程碑目标（P2）：进度条式用途叙事，配置默认关闭、整区不渲染 ──
    let goal_sec: Option<Element<'_, Message>> = sponsor_config()
        .goal
        .as_ref()
        .filter(|g| g.enabled)
        .map(|g| {
            let pct = if g.target > 0.0 {
                (g.current / g.target).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let fill = ((pct * 100.0).round() as u16).max(1);
            let rest = 100 - fill;
            let bar = row![
                container(Space::new().height(Length::Fixed(6.0)))
                    .width(Length::FillPortion(fill))
                    .style(|_t| container::Style {
                        background: Some(Background::Color(TXT())),
                        border: Border {
                            color: iced::Color::TRANSPARENT,
                            width: 0.0,
                            radius: 3.0.into(),
                        },
                        ..container::Style::default()
                    }),
                container(Space::new().height(Length::Fixed(6.0)))
                    .width(Length::FillPortion(rest.max(1)))
                    .style(|_t| container::Style {
                        background: Some(Background::Color(LINE())),
                        border: Border {
                            color: iced::Color::TRANSPARENT,
                            width: 0.0,
                            radius: 3.0.into(),
                        },
                        ..container::Style::default()
                    }),
            ]
            .width(Length::Fill);
            column![
                text(&g.label).size(FS_XS).color(TXT3()),
                bar,
                text(format!(
                    "{} / {}",
                    fmt_amount(g.current as f32),
                    fmt_amount(g.target as f32)
                ))
                .size(FS_XS)
                .color(TXT3()),
            ]
            .spacing(4)
            .width(Length::Fill)
            .into()
        });

    // 建议金额 chips + 自定义输入
    let mut chips = row![].spacing(6).align_y(Alignment::Center);
    for v in PRESETS {
        let selected = app.sponsor_custom.is_empty() && (app.sponsor_amount - v).abs() < 0.001;
        chips = chips.push(
            button(
                text(fmt_amount(*v))
                    .size(FS_SM)
                    .color(if selected { BG() } else { TXT2() }),
            )
            .padding(pad(4.0, 10.0, 4.0, 10.0))
            .style(if selected { chip_sel_style } else { chip_style })
            .on_press(Message::Sponsor(SponsorMessage::Amount(*v))),
        );
    }
    chips = chips.push(
        text_input(tr("自定义"), &app.sponsor_custom)
            .size(FS_SM)
            .width(Length::Fixed(96.0))
            .style(text_input_style)
            .on_input(|s| Message::Sponsor(SponsorMessage::CustomInput(s))),
    );

    // 金额提示：自定义优先，其次选中的档位（引导到渠道页输入）
    let custom = app.sponsor_custom.trim().parse::<f32>().ok();
    let hint = if let Some(v) = custom.filter(|v| *v > 0.0) {
        trf("在爱发电 / Ko-fi 输入 {} 即可，金额不限，随心就好。", &[&fmt_amount(v)])
    } else {
        trf(
            "在爱发电 / Ko-fi 输入 {} 即可，金额不限，随心就好。",
            &[&fmt_amount(app.sponsor_amount)],
        )
    };
    let hint = text(hint).size(FS_SM).color(TXT2());

    let footnote = text(tr(
        "打赏不升级、不解锁、不影响任何功能——你的每一杯咖啡，都会变成下一个新功能。",
    ))
    .size(FS_XS)
    .color(TXT3())
    .width(Length::Fill);

    // 主体：固定区（价值三件套 → 文案 → Star）+ 可选区（渠道 / 里程碑）
    let mut body = column![head, trio, copy].spacing(8).width(Length::Fill);
    if let Some(sec) = star_sec {
        body = body.push(sec);
    }
    if let Some(sec) = links_section {
        body = body.push(sec);
    }
    if let Some(sec) = goal_sec {
        body = body.push(sec);
    }
    let card = container(body.push(chips).push(hint).push(footnote))
        .width(Length::Fixed(card_w))
        .padding(pad_all(CARD_PAD))
        .style(card_style);

    // 停靠：右缘、标题栏下方；入场时右侧 padding SLIDE→0，横向滑入就位
    container(stack![
        scrim,
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::End)
            .align_y(Alignment::Start)
            .padding(pad(
                DOCK_TOP,
                DOCK_RIGHT + (1.0 - a) * SLIDE,
                0.0,
                0.0,
            )),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}
