//! 复用的界面零件：小节、字段标签、滑块行、开关行、分段控件、色板、键值行、徽标
//!
//! 这些零件对应原项目 CSS 里的 `.sec` / `.field` / `.switch-row` / `.seg` /
//! `.swatch-grid` / `.kv-badge` / `.hint`，所有间距与圆角取自 `theme.rs` 的 token。

// 复用零件库：部分零件（hint_chip / icon_tile 等）当前页面尚未引用，但属于
// 完整的 UI 词汇表，留作后续取用，故整体豁免 dead_code 告警。
#![allow(dead_code)]

use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::widget::{button, column, container, row, slider, stack, text, Space};
use iced::{
    Alignment, Border, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme,
};

use crate::message::Message;
use crate::theme::*;
use crate::ui::icons::icon;

/* ═════════════════ 小节 ═════════════════ */

/// 检查器小节：眉标 + 内容
///
/// 对应 `.sec`。原项目用 `border-bottom:1px solid var(--line-soft)` 分隔小节，
/// 这里不画独立面板 —— 分隔线交给 [`rule`]，由调用方在小节之间插入，
/// 这样最后一个小节下面不会多出一条线（等价于 `.sec:last-child`）。
pub fn section<'a>(
    title: &'a str,
    badge: Option<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let head: Element<'a, Message> = match badge {
        Some(b) => row![sec_title(title), Space::new().width(Length::Fill), b]
            .align_y(Alignment::Center)
            .into(),
        None => row![sec_title(title)].into(),
    };
    container(column![head, content.into()].spacing(13))
        .padding(pad_all(16.0))
        .width(Length::Fill)
        .into()
}

/// 小节眉标 `.sec-title`：10.5px + 加粗 + `--txt3`
///
/// 注：CSS 还有 `.09em` 字距与 `text-transform:uppercase`，
/// iced 的 `Text` 两者都不支持 —— 中文标题本来也没有大小写，
/// 字距差异在小字号下肉眼不可辨，故不额外模拟（见 README「无法精确还原」一节）。
pub fn sec_title<'a>(title: &'a str) -> Element<'a, Message> {
    strong(title, FS_SEC, TXT3()).into()
}

/// 小节之间的 1px 细分隔线（`.sec` 的 `border-bottom`）
pub fn rule<'a>() -> Element<'a, Message> {
    container(Space::new().width(Length::Fill).height(Length::Fixed(1.0)))
        .width(Length::Fill)
        .style(|_t| container::Style {
            background: Some(iced::Background::Color(LINE_SOFT())),
            ..container::Style::default()
        })
        .into()
}

/// 「名称 —— 当前值」行（`.field-label`）
///
/// 原项目里名称是次要色、数值是深色加粗 —— 层级靠**颜色与字重**而非字号区分。
/// `value` / `unit` 取 `impl Into<String>`，调用方既可以传 owned String，
/// 也可以传 `&str` / `format!(...)`，不必为了生命周期到处 clone。
pub fn field_label<'a>(
    name: &'a str,
    value: impl Into<String>,
    unit: impl Into<String>,
) -> Element<'a, Message> {
    row![
        text(name).size(FS_SM).color(TXT2()),
        Space::new().width(Length::Fill),
        strong(value.into(), FS_SM, TXT()),
        text(unit.into()).size(FS_SM).color(TXT3()),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// 滑块行
pub fn slider_row<'a>(
    name: &'a str,
    value: i32,
    range: std::ops::RangeInclusive<i32>,
    unit: impl Into<String>,
    on_change: impl Fn(i32) -> Message + 'a,
) -> Element<'a, Message> {
    column![
        field_label(name, format!("{value}"), unit),
        slider(range, value, move |v: i32| on_change(v))
            .step(1)
            .width(Length::Fill)
            .style(crate::theme::slider_style),
    ]
    .spacing(7)
    .into()
}

/* ═════════════════ 开关 ═════════════════ */

/// 开关行（左标签 + 右开关），对应 `.switch-row` + `.switch`
pub fn switch_row<'a>(
    label: &'a str,
    on: bool,
    msg: Message,
    hint: Option<&'a str>,
) -> Element<'a, Message> {
    let mut col = column![row![
        text(label).size(FS_SM).color(TXT2()).width(Length::Fill),
        switch(on, msg)
    ]
    .align_y(Alignment::Center)
    .spacing(10)]
    .spacing(6);
    if let Some(h) = hint {
        col = col.push(text(h).size(FS_XS).color(TXT3()));
    }
    col.into()
}

/// 胶囊开关本体：38×22 轨道 + 18×18 圆钮，开时滑块右移 16px
///
/// CSS 的 `.switch::after{transform:translateX(16px)}` 在这里由
/// 「圆钮前方填 16px 空白」实现，比手工算绝对坐标更贴合 iced 的弹性布局。
pub fn switch<'a>(on: bool, msg: Message) -> Element<'a, Message> {
    let knob = container(Space::new().width(Length::Fixed(18.0)).height(Length::Fixed(18.0)))
        .style(crate::theme::switch_knob);

    let track = if on {
        row![Space::new().width(Length::Fixed(16.0)), knob]
    } else {
        row![knob, Space::new().width(Length::Fixed(16.0))]
    };

    button(container(track).padding(pad_all(2.0)))
        .on_press(msg)
        .padding(0)
        .style(crate::theme::switch(on))
        .into()
}

/* ═════════════════ 分段控件 ═════════════════ */

/// 强分段（舞台条 / 视图切换）：`--panel-3` 轨道 + **近黑实心选中块**
///
/// 标签转成 `String` 持有，返回的 `Element<'static>` 不再借用传入的切片，
/// 因此即便调用处传的是临时数组也能安全返回（不会被借用检查拦下）。
pub fn seg_strong(items: &[(&str, bool, Message)]) -> Element<'static, Message> {
    let mut r = row![].spacing(2);
    for (label, active, msg) in items {
        r = r.push(
            button(text(label.to_string()).size(FS_SM).color(if *active { Color::WHITE } else { TXT2() }))
                .on_press(msg.clone())
                .padding(pad(5.0, 12.0, 5.0, 12.0))
                .style(seg_btn(*active)),
        );
    }
    container(r)
        .padding(3.0)
        .style(|_t| container::Style {
            text_color: Some(TXT()),
            background: Some(iced::Background::Color(PANEL_3())),
            border: iced::Border { color: LINE(), width: 1.0, radius: R_BTN.into() },
            shadow: iced::Shadow::default(),
            snap: false,
        })
        .into()
}

/// 通栏分段控件（`.seg` 原样式：按钮 `flex:1` 平分宽度，证件照页单张/排版页签用）
///
/// 这是**弱分段**，与 `seg_strong`（工具条的近黑实心）刻意区分：轨道是 `--fill`
/// 浅灰、**无边框**、`--r-sm` 圆角、2px 内边距；选中块是**白底 + 近黑加粗字**
/// 加一圈半像素描边与微投影（`seg_soft_btn` 对应 `.seg-btn.on`），与
/// `page_work.rs::seg_group` 同源，保证同一种 `.seg` 在全局表现一致。
pub fn seg_fill(items: &[(&str, bool, Message)]) -> Element<'static, Message> {
    let mut r = row![].spacing(2).width(Length::Fill);
    for (label, active, msg) in items {
        // 按钮内容默认左上对齐（iced `layout::padded` 把子节点放在 padding 处），
        // 所以这里让文本撑满按钮宽度并水平居中，才能得到 HTML 按钮那样的居中标签。
        r = r.push(
            button(
                strong(label.to_string(), FS_XS, if *active { INK() } else { TXT2() })
                    .width(Length::Fill)
                    .center(),
            )
            .on_press(msg.clone())
            .padding(pad(6.0, 12.0, 6.0, 12.0))
            .width(Length::Fill)
            .style(seg_soft_btn(*active)),
        );
    }
    container(r)
        .width(Length::Fill)
        .padding(2.0)
        .style(|_t| container::Style {
            text_color: Some(TXT()),
            background: Some(iced::Background::Color(FILL())),
            border: iced::Border { color: iced::Color::TRANSPARENT, width: 0.0, radius: R_SM.into() },
            shadow: iced::Shadow::default(),
            snap: false,
        })
        .into()
}

/* ═════════════════ 色板 ═════════════════ */

/// 色板网格（`.swatch-grid` / `.swatch`）
///
/// 选中态是**双环**：CSS 用 `box-shadow: 0 0 0 2px panel, 0 0 0 3.5px ink` 做出
/// 「黑环 + 2px 白缝」。iced 的 `Shadow` 只有一层，改用
/// 「按钮 2px 内边距 + 1.5px 近黑描边」等价实现：描边在外、内边距形成白缝，
/// 总尺寸 28+2×2+2×1.5 = 35px，与 CSS 的 `28 + 2×3.5` 完全一致。
pub fn swatch_grid<'a>(
    colors: &'a [(&'a str, Option<Color>)],
    selected: &'a str,
    on_select: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    let mut r = row![].spacing(4).align_y(Alignment::Center);
    for (key, color) in colors {
        let is_sel = *key == selected;
        let fill = match color {
            Some(c) => *c,
            // 透明底：用浅灰斜纹示意（CSS 的 `.swatch.transparent`）
            None => CHECKER_B(),
        };
        let inner = container(Space::new().width(Length::Fixed(28.0)).height(Length::Fixed(28.0)))
            .style(move |_t| container::Style {
                text_color: Some(TXT()),
                background: Some(iced::Background::Color(fill)),
                border: iced::Border { color: LINE(), width: 1.0, radius: R_SM.into() },
                shadow: iced::Shadow::default(),
                snap: false,
            });
        r = r.push(
            button(inner)
                .on_press(on_select((*key).to_string()))
                .padding(2.0)
                .style(swatch(is_sel)),
        );
    }
    r.into()
}

/* ═════════════════ 其他零件 ═════════════════ */

/// 键值行（`.kv` / `.home-kv-row`）
pub fn kv_row(k: String, v: impl Into<String>) -> Element<'static, Message> {
    container(
        row![
            text(k).size(FS_SM).color(TXT2()),
            Space::new().width(Length::Fill),
            strong(v.into(), FS_SM, TXT()),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding(pad(8.0, 2.0, 8.0, 2.0))
    .width(Length::Fill)
    .into()
}

/// 状态徽标 `.kv-badge` / `.sec-badge`
pub fn badge(label: impl Into<String>, kind: BadgeKind) -> Element<'static, Message> {
    let (fg, bg) = match kind {
        BadgeKind::Ok => (OK(), OK_SOFT()),
        BadgeKind::Dim => (TXT2(), PANEL_3()),
        BadgeKind::Warn => (WARN(), WARN_SOFT()),
        BadgeKind::Err => (DANGER(), DANGER_SOFT()),
        BadgeKind::Ink => (INK(), FILL()),
    };
    container(text(label.into()).size(FS_XS).color(fg))
        .padding(pad(3.0, 9.0, 3.0, 9.0))
        .style(badge_style(fg, bg))
        .into()
}

#[derive(Debug, Clone, Copy)]
pub enum BadgeKind {
    Ok,
    Dim,
    Warn,
    Err,
    /// `.sec-badge` 默认态：浅灰底 + 近黑字
    Ink,
}

/// 提示行（灰色小字 `.hint-line`）
pub fn hint(s: impl Into<String>) -> Element<'static, Message> {
    text(s.into()).size(FS_XS).color(TXT3()).into()
}

/// 提示胶囊 `.hint`：白底 + 描边 + 20px 全圆角，左侧一个绿色对勾
pub fn hint_chip<'a>(label: &'a str) -> Element<'a, Message> {
    container(
        row![
            icon("check", 13.0, OK()),
            text(label).size(FS_XS).color(TXT2()),
        ]
        .spacing(5)
        .align_y(Alignment::Center),
    )
    .padding(pad(5.0, 11.0, 5.0, 11.0))
    .style(chip)
    .into()
}

/// 图标块（`.empty-icon` / `.tile-ic` / `.hp-ic`）
pub fn icon_tile<'a>(
    glyph: &'a str,
    box_size: f32,
    glyph_size: f32,
    bg: Color,
    radius: f32,
) -> Element<'a, Message> {
    container(text(glyph).size(glyph_size).color(TXT2()))
        .width(Length::Fixed(box_size))
        .height(Length::Fixed(box_size))
        .center_x(Length::Fixed(box_size))
        .center_y(Length::Fixed(box_size))
        .style(crate::theme::icon_tile(bg, radius))
        .into()
}

/* ═════════════════ 虚线框 ═════════════════ */

/// 虚线框 `.dropzone`
///
/// iced 的 `Border` 只能画实线，这里用一层**覆盖在内容之上**的 canvas 手绘
/// `border:1.5px dashed var(--line-strong)`。canvas 的 `update` 走 trait 默认实现
/// （返回 `None`），事件会穿透到下层，因此不影响拖放/点击。
/// `bg` 为 `None` 时保持透明（配合卡片底色，如 `.dropzone{background:var(--panel)}`），
/// `Some(c)` 时铺一层圆角底色（如 `.home-drop{background:var(--panel-2)}`）。
pub fn dashed_box<'a>(
    content: impl Into<Element<'a, Message>>,
    color: Color,
    radius: f32,
    bg: Option<Color>,
) -> Element<'a, Message> {
    stack![
        // 参考 `.home-drop` / `.dropzone` 都是 `align-items:center;justify-content:center`，
        // 内容整体居中（不只是内部 `text-align:center`）。
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(move |_t: &Theme| container::Style {
                background: bg.map(iced::Background::Color),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: radius.into(),
                },
                ..container::Style::default()
            }),
        Canvas::new(DashedFrame { color, radius }).width(Length::Fill).height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// 只负责描边的 canvas Program
struct DashedFrame {
    color: Color,
    radius: f32,
}

impl<Message> canvas::Program<Message> for DashedFrame {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        if w > 4.0 && h > 4.0 {
            // 1.5px 线宽以路径为中心向两侧展开，内缩半个线宽让虚线完整落在框内
            let half = 0.75;
            let path = Path::rounded_rectangle(
                Point::new(half, half),
                Size::new(w - 2.0 * half, h - 2.0 * half),
                self.radius.into(),
            );
            const SEGMENTS: [f32; 2] = [6.0, 5.0];
            frame.stroke(
                &path,
                Stroke {
                    style: canvas::Style::Solid(self.color),
                    width: 1.5,
                    line_cap: canvas::LineCap::Butt,
                    line_join: canvas::LineJoin::Round,
                    line_dash: canvas::LineDash { segments: &SEGMENTS, offset: 0 },
                },
            );
        }
        vec![frame.into_geometry()]
    }
}

/* ═════════════════ 处理中遮罩 ═════════════════ */

/// 旋转加载圈：canvas 弧线动画，`frame` 由时间订阅驱动
/// （订阅仅在载入/处理期间激活，空闲零开销）。
pub struct Spinner {
    pub frame: f32,
}

impl<Message> canvas::Program<Message> for Spinner {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = bounds.center();
        let start = self.frame * 0.6;
        let arc = canvas::path::Path::new(|b| {
            b.arc(canvas::path::arc::Arc {
                center,
                radius: 10.0,
                start_angle: iced::Radians(start),
                end_angle: iced::Radians(start + 4.4),
            });
        });
        frame.stroke(&arc, Stroke::default().with_color(TXT2()).with_width(3.0));
        vec![frame.into_geometry()]
    }
}

/// 推理中遮罩（`.processing`：`rgba(245,246,247,.72)` 半透明衬底 + 内容居中）。
/// 全屏覆盖挡掉一切交互 —— 处理中的重复提交天然被阻断。
pub fn busy_overlay<'a>(title: &'a str, sub: &'a str, frame: f32) -> Element<'a, Message> {
    container(
        container(
            column![
                Canvas::new(Spinner { frame })
                    .width(Length::Fixed(34.0))
                    .height(Length::Fixed(34.0)),
                text(title).size(13.0).font(bold_font()).color(TXT()),
                text(sub).size(FS_SM).color(TXT2()),
            ]
            .spacing(13)
            .align_x(Alignment::Center),
        )
        .padding(pad(24.0, 32.0, 24.0, 32.0))
        .style(proc_card_style),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(scrim_style)
    .into()
}

fn scrim_style(_t: &Theme) -> container::Style {
    container::Style {
        text_color: None,
        background: Some(iced::Background::Color(Color::from_rgba(
            245.0 / 255.0,
            246.0 / 255.0,
            247.0 / 255.0,
            0.72,
        ))),
        border: iced::Border::default(),
        shadow: iced::Shadow::default(),
        snap: false,
    }
}

/// 处理中卡片（`.proc-card`：白面板 + 描边 + 圆角 + 浮起阴影）
fn proc_card_style(_t: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(iced::Background::Color(PANEL())),
        border: iced::Border { color: LINE(), width: 1.0, radius: R_PANEL.into() },
        shadow: iced::Shadow {
            color: Color::from_rgba(0.06, 0.07, 0.09, 0.16),
            offset: iced::Vector::new(0.0, 10.0),
            blur_radius: 28.0,
        },
        snap: false,
    }
}