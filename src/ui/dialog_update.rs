//! 更新对话框（全局最高层模态）
//!
//! 覆盖自更新全流程的四种用户可见状态：
//! 发现新版本（更新日志 + 一键更新 / 忽略此版本 / 稍后）→
//! 下载中（进度条 + 取消）→ 就绪（重启并安装）→
//! 失败（原因 + 重试 / 前往下载页）。另有 Checking / UpToDate 两个轻状态，
//! 供手动检查时反馈结果。
//!
//! 视觉走主题 token：白卡 + hairline + `--shadow-4`，主按钮近黑（同全站）。

use iced::widget::{button, column, container, progress_bar, row, scrollable, space::Space, stack, text};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::{Alignment, Color, Element, Length};

use crate::app::{UpdateStatus, UpdaterState};
use crate::core::updater;
use crate::message::{Message, UpdaterMessage};
use crate::theme::*;
use crate::core::i18n::{tr, trf};

/// 对话框宽度（逻辑像素）
const DIALOG_W: f32 = 440.0;

/// 模态叠层：半透明遮罩 + 居中卡片。`dialog_open` 时才进 stack，无需空元素分支。
pub fn modal(state: &UpdaterState) -> Element<'static, Message> {
    let card: Element<'static, Message> = match &state.status {
        UpdateStatus::Checking => checking_view(),
        UpdateStatus::UpToDate => up_to_date_view(),
        UpdateStatus::Available(info) => available_view(info),
        UpdateStatus::Downloading { info, received, total } => {
            downloading_view(&info.version, *received, *total)
        }
        UpdateStatus::Ready { info, path } => ready_view(&info.version, path),
        UpdateStatus::Failed { msg, info } => failed_view(msg, info.as_deref()),
        UpdateStatus::Idle => {
            // 理论上 dialog_open 与 Idle 不同时出现；兜底空层
            Space::new().width(0).height(0).into()
        }
    };

    let card = container(card)
        .width(Length::Fixed(DIALOG_W))
        .padding(pad_all(CARD_PAD))
        .style(dialog_card);

    // iced 0.14 Stack 布局：覆盖层子项固定在 (0,0)、不自动居中——
    // 卡片必须包一层 Fill×Fill + center 的容器才能在遮罩上居中。
    container(stack![
        scrim_layer(),
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/* ═════════════════ 各状态视图 ═════════════════ */

fn checking_view() -> Element<'static, Message> {
    column![
        head(tr("正在检查更新"), true),
        text(tr("正在连接更新服务器…")).size(FS).color(TXT2()),
    ]
    .spacing(14)
    .into()
}

fn up_to_date_view() -> Element<'static, Message> {
    column![
        head(tr("已是最新版本"), true),
        text(trf("当前版本 v{}", &[updater::current_version_text()]))
            .size(FS)
            .color(TXT2()),
    ]
    .spacing(14)
    .into()
}

fn available_view(info: &updater::ReleaseInfo) -> Element<'static, Message> {
    // 版本对照行：当前 vX → 新版 vY（+ 发布日期）
    let ver_row = row![
        text(format!("v{}", updater::current_version_text())).size(FS).color(TXT2()),
        text("→").size(FS).color(TXT3()),
        text(format!("v{}", info.version)).size(FS_H).color(TXT()).font(bold_font()),
        text(info.pub_date.clone()).size(FS_XS).color(TXT3()),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut col = column![head(tr("发现新版本"), info.mandatory), ver_row].spacing(12);
    col = col.push(section_title(tr("更新日志")));
    col = col.push(notes_view(&info.notes));
    col = col.push(footer(
        info.mandatory.then(|| ghost_btn(tr("忽略此版本"), Message::Updater(UpdaterMessage::SkipVersion))),
        row![
            secondary_btn(tr("稍后"), Message::Updater(UpdaterMessage::Dismiss)),
            primary_btn(tr("一键更新"), Message::Updater(UpdaterMessage::StartInstall)),
        ]
        .spacing(8),
    ));
    col.spacing(12).into()
}

fn downloading_view(version: &str, received: u64, total: Option<u64>) -> Element<'static, Message> {
    let pct = total.map(|t| (received as f32 / t.max(1) as f32 * 100.0).min(100.0));

    let status_text = match (total, pct) {
        (Some(t), Some(p)) => format!(
            "{} / {}  ·  {:.0}%",
            human_size(received),
            human_size(t),
            p
        ),
        _ => human_size(received),
    };

    let bar: Element<'static, Message> = match pct {
        Some(p) => progress_bar(0.0..=100.0, p).into(),
        None => text(status_text.clone()).size(FS).color(TXT2()).into(),
    };

    column![
        head(tr("正在下载更新"), false),
        text(trf("新版本 v{}", &[version])).size(FS).color(TXT2()),
        bar,
        text(status_text).size(FS_XS).color(TXT3()),
        footer(
            None,
            secondary_btn(tr("取消下载"), Message::Updater(UpdaterMessage::CancelDownload)),
        ),
    ]
    .spacing(12)
    .into()
}

fn ready_view(version: &str, path: &std::path::Path) -> Element<'static, Message> {
    column![
        head(tr("安装包已就绪"), false),
        text(trf("v{} 下载完成，点击「重启并安装」将启动安装器并关闭本程序。", &[version]))
            .size(FS)
            .color(TXT2()),
        text(path.display().to_string()).size(FS_XS).color(TXT3()),
        footer(
            None,
            row![
                secondary_btn(tr("稍后安装"), Message::Updater(UpdaterMessage::Dismiss)),
                primary_btn(tr("重启并安装"), Message::Updater(UpdaterMessage::QuitAndInstall)),
            ]
            .spacing(8),
        ),
    ]
    .spacing(12)
    .into()
}

fn failed_view(msg: &str, info: Option<&updater::ReleaseInfo>) -> Element<'static, Message> {
    // 检查失败 → 重试「检查」；下载失败 → 重试「下载」
    let retry = match info {
        Some(_) => primary_btn(tr("重试下载"), Message::Updater(UpdaterMessage::StartInstall)),
        None => primary_btn(tr("重试检查"), Message::Updater(UpdaterMessage::Check)),
    };

    column![
        head_err(tr("更新失败"), true),
        text(msg.to_string()).size(FS).color(TXT2()),
        text(tr("也可以前往官网手动下载最新安装包。")).size(FS_XS).color(TXT3()),
        footer(
            None,
            row![
                secondary_btn(tr("前往下载页"), Message::Updater(UpdaterMessage::OpenDownloadPage)),
                retry,
            ]
            .spacing(8),
        ),
    ]
    .spacing(12)
    .into()
}

/* ═════════════════ 私有组件 ═════════════════ */

/// 标题行：标题 + 右上角关闭钮（dismissable=false 时不给关闭）
fn head(title: &'static str, dismissable: bool) -> Element<'static, Message> {
    let close: Element<'static, Message> = if dismissable {
        button(text("×").size(16.0).color(TXT3()))
            .padding(pad(2.0, 10.0, 2.0, 10.0))
            .style(rail_btn(false))
            .on_press(Message::Updater(UpdaterMessage::Dismiss))
            .into()
    } else {
        Space::new().width(Length::Fixed(30.0)).height(1.0).into()
    };
    row![
        strong(title, FS_H, TXT()),
        Space::new().width(Length::Fill),
        close,
    ]
    .align_y(Alignment::Center)
    .into()
}

/// 失败标题（红字，必须走重试 / 下载页出口）
fn head_err(title: &'static str, dismissable: bool) -> Element<'static, Message> {
    let close: Element<'static, Message> = if dismissable {
        button(text("×").size(16.0).color(TXT3()))
            .padding(pad(2.0, 10.0, 2.0, 10.0))
            .style(rail_btn(false))
            .on_press(Message::Updater(UpdaterMessage::Dismiss))
            .into()
    } else {
        Space::new().width(Length::Fixed(30.0)).height(1.0).into()
    };
    row![
        strong(title, FS_H, DANGER()),
        Space::new().width(Length::Fill),
        close,
    ]
    .align_y(Alignment::Center)
    .into()
}

fn section_title(s: &'static str) -> Element<'static, Message> {
    text(s.to_uppercase()).size(FS_SEC).color(TXT3()).into()
}

/// 更新日志正文：按 \n 分行，滚动展示（高度固定，行多不撑爆对话框）
fn notes_view(notes: &str) -> Element<'static, Message> {
    let mut col = column![].spacing(4);
    for line in notes.lines().filter(|l| !l.trim().is_empty()) {
        col = col.push(text(format!("· {line}")).size(FS_SM).color(TXT2()));
    }
    scrollable(col)
        .height(Length::Fixed(150.0))
        .direction(Direction::Vertical(Scrollbar::new().width(3).scroller_width(3)))
        .style(scrollbar_style)
        .into()
}

/// 底部按钮排：左侧可选的弱操作（忽略此版本），右侧主操作组
fn footer(
    left: Option<Element<'static, Message>>,
    right: impl Into<Element<'static, Message>>,
) -> Element<'static, Message> {
    let mut r: iced::widget::Row<'static, Message> = row![].spacing(8);
    if let Some(l) = left {
        r = r.push(l);
    }
    r = r.push(Space::new().width(Length::Fill));
    r = r.push(right.into());
    r.align_y(Alignment::Center).into()
}

fn primary_btn(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(FS))
        .padding(pad(7.0, 16.0, 7.0, 16.0))
        .style(btn_primary)
        .on_press(msg)
        .into()
}

fn secondary_btn(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(FS).color(TXT()))
        .padding(pad(7.0, 16.0, 7.0, 16.0))
        .style(btn_secondary)
        .on_press(msg)
        .into()
}

fn ghost_btn(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(FS).color(TXT2()))
        .padding(pad(7.0, 16.0, 7.0, 16.0))
        .style(btn_ghost)
        .on_press(msg)
        .into()
}

fn human_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

/* ═════════════════ 样式 ═════════════════ */

/// 半透明遮罩（对应 theme::scrim，但这里盖全屏）
fn scrim_layer() -> Element<'static, Message> {
    container(Space::new().width(Length::Fill).height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_t| container::Style {
            background: Some(iced::Background::Color(Color::from_rgba(0.06, 0.07, 0.08, 0.35))),
            ..container::Style::default()
        })
        .into()
}

/// 对话框卡：白底 + hairline + 最高层阴影（`--shadow-4`，同提示条层级）
fn dialog_card(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(iced::Background::Color(PANEL())),
        border: iced::Border {
            color: LINE(),
            width: 1.0,
            radius: R_PANEL.into(),
        },
        shadow: shadow_4(),
        snap: false,
    }
}
