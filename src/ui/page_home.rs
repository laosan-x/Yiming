//! 首页 · 证件照工作台落地页
//!
//! 对应参考项目 `core/home.js` + `#pageHome` 样式：
//! 顶部居中标题 + 右上「新手引导」；左「开始」卡（虚线拖放区 + 载入示例 +
//! 重置参数）；右列「快速预设 / 当前状态 / 去制作证件照」。
//!
//! 与参考一致的三条行为约定：
//! 1. 快速预设点击**只应用参数**（规格 + 底色联动）并 toast，不跳页；
//! 2. 「载入示例」用内置合成人像（`buildSampleFile` 思路），直接送入证件照页；
//! 3. 「重置参数」按参考 `resetIdParams` 语义重置**证件照**参数，
//!    而非工作台的抠图蒙版参数。

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Rectangle, Shadow, Task, Theme};

use crate::app::App;
use crate::core::i18n::{tr, trf};
use crate::core::idphoto::{ID_BGS, ID_PAPERS, ID_SIZES};
use crate::message::{HomeMessage, Message, Page};
use crate::theme::*;
use crate::ui::guide::{self, Target};
use crate::ui::icons::icon;
use crate::ui::widgets::dashed_box;

/// 左侧「开始」卡里虚线拖放区的高度。
///
/// 参考是 `.home-drop{flex:1;min-height:300px}`——在 CSS Grid 的
/// `align-items:stretch` 下会被拉伸到与右列等高。iced 的行布局没有「等高一列」
/// 的等价物（`Length::Fill` 高度在滚动容器里会解析成 ∞），所以这里用一个
/// 与右列自然高度对齐的固定值（实测：右列 551 逻辑 px，左卡非拖放区部分
/// 104 逻辑 px，故 551 − 104 = 447，两列盒底（含 1px 边框）齐平）。
const DROP_H: f32 = 447.0;

/// 快速预设表（参考 home.js `PRESETS`）：
/// (lucide 图标, 标题, 副标, 规格索引, 底色索引)
const PRESETS: &[(&str, &str, &str, usize, usize)] = &[
    ("user", "一寸 · 蓝底", "简历 / 入职 / 社保", 0, 1),
    ("file-text", "二寸 · 白底", "签证 / 档案", 3, 0),
    ("award", "考公 · 事业编", "35×45mm · 机器校验蓝底", 12, 2),
    (
        "graduation-cap",
        "教资 / 四六级",
        "30×40mm · 144×192px",
        13,
        1,
    ),
    ("plane", "护照 · 签证", "33×48mm", 6, 0),
    ("globe", "美国签证", "2×2英寸 600×600px", 10, 0),
    ("credit-card", "社保 / 驾驶证", "358×441 / 22×32mm", 18, 0),
];

/// 分步新手引导：（目标区域，标题，说明）；顺序即步骤顺序。
/// Target 对应聚焦挖洞的区域，说明气泡锚定在洞旁。
const GUIDE_STEPS: &[(Target, &str, &str)] = &[
    (
        Target::StartCard,
        "上传照片",
        "点「开始」卡的虚线框或直接拖入照片，支持 JPG / PNG / WebP，全程本地处理。",
    ),
    (
        Target::Presets,
        "快速预设",
        "预设一键套用常用证件规格与底色，也可进入证件照页后自行调整。",
    ),
    (
        Target::Status,
        "当前状态",
        "汇总当前规格、输出像素、排版相纸与推理引擎，确认参数再动手。",
    ),
    (
        Target::MakeBtn,
        "开始制作",
        "点「去制作证件照」进入证件照页：换底 → 调构图 → 排版导出。",
    ),
];

pub fn update(app: &mut App, msg: HomeMessage) -> Task<Message> {
    match msg {
        HomeMessage::OpenPhoto => {
            // 「开始」卡点击：直达抠图工作台并弹原生文件选择器（与工作台
            // 「打开图片」共用 pick_image 链路，选完自动载入蒙版工作流）
            app.page = Page::Work;
            Task::batch([
                Task::done(Message::Navigate(Page::Work)),
                Task::perform(crate::ui::page_work::pick_image(), |p| {
                    Message::Work(crate::message::WorkMessage::FilePicked(p))
                }),
            ])
        }
        HomeMessage::ApplyPreset(i) => {
            if let Some((_, title, _, spec, bg)) = PRESETS.get(i) {
                app.id.spec_idx = *spec;
                app.id.bg_idx = *bg;
                app.persist();
                // 参考 home.js：只应用参数 + toast，留在首页
                app.toast(trf(
                    "{} · {} · {}（去证件照页即可制作）",
                    &[tr("已应用预设"), tr(title), ID_BGS[*bg].n],
                ));
            }
            Task::none()
        }
        HomeMessage::LoadSample => {
            // 加载内置示例图片（编译时内嵌，不依赖运行时文件系统）
            const SAMPLE_DATA: &[u8] = include_bytes!("../../tools/Example.jpg");
            app.page = Page::IdPhoto;
            app.toast(tr("已载入示例图片"));
            crate::ui::page_idphoto::load_id_bytes(SAMPLE_DATA, "Example.jpg")
        }
        HomeMessage::ResetParams => {
            // 参考 resetIdParams：重置证件照参数（非工作台蒙版参数）
            app.id.spec_idx = 0;
            app.id.bg_idx = 0;
            app.id.paper_idx = 0;
            app.id.off_y = 0;
            app.id.zoom = 100;
            app.id.erode = 1;
            app.id.cutline = true;
            app.persist();
            app.toast(tr("证件照参数已重置"));
            Task::none()
        }
        // 状态转移统一走 guide::transition 纯函数（可 headless 单测）；
        // 这里只负责把结果写回 App 与持久化副作用。
        HomeMessage::StartGuide => {
            app.guide_step =
                guide::transition(app.guide_step, guide::Action::Start, GUIDE_STEPS.len()).0;
            Task::none()
        }
        HomeMessage::GuidePrev => {
            app.guide_step =
                guide::transition(app.guide_step, guide::Action::Prev, GUIDE_STEPS.len()).0;
            Task::none()
        }
        HomeMessage::GuideNext => {
            let (next, done) =
                guide::transition(app.guide_step, guide::Action::Next, GUIDE_STEPS.len());
            app.guide_step = next;
            if done && !app.config.guide_done {
                app.config.guide_done = true;
                app.persist();
            }
            Task::none()
        }
        HomeMessage::GuideGoto(i) => {
            app.guide_step =
                guide::transition(app.guide_step, guide::Action::Goto(i), GUIDE_STEPS.len()).0;
            Task::none()
        }
        HomeMessage::GuideSkip => {
            // 仅关闭：下次进首页（未标记完成时）仍会自动弹出
            app.guide_step =
                guide::transition(app.guide_step, guide::Action::Skip, GUIDE_STEPS.len()).0;
            Task::none()
        }
        HomeMessage::GuideNever => {
            let (next, done) =
                guide::transition(app.guide_step, guide::Action::Never, GUIDE_STEPS.len());
            app.guide_step = next;
            if done && !app.config.guide_done {
                app.config.guide_done = true;
                app.persist();
            }
            Task::none()
        }
    }
}

/* ═════════════════ 视图 ═════════════════ */

/// 「键 — 值」行（参考 `.home-kv-row`：标签居左、值居右）
fn kv<'a>(k: &'a str, v: String) -> Element<'a, Message> {
    row![
        text(k).size(FS_SM).color(TXT3()),
        Space::new().width(Length::Fill),
        text(v).size(FS_SM).color(TXT()),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// 快速预设按钮样式（参考 `.home-preset`：`--panel-2` 底 + `--line` 描边，
/// 悬停 `--acc-faint` 底 + `--acc` 描边；当前选中的规格用同款强调色常驻）。
fn preset_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let (bg, bc) = if active {
            (ACC_FAINT(), ACC())
        } else {
            match status {
                button::Status::Hovered | button::Status::Pressed => (ACC_FAINT(), ACC()),
                _ => (PANEL_2(), LINE()),
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: TXT(),
            border: Border {
                color: bc,
                width: 1.0,
                radius: 10.0.into(),
            },
            shadow: iced::Shadow::default(),
            snap: false,
        }
    }
}

/// 「新手引导」按钮样式（参考 `.home-guide`：`--panel-2` 底 + `--line-strong`
/// 描边 + **99px 全圆角**（药丸），悬停 `--panel-hover` 底 + `--txt-3` 描边）
fn guide_style(_t: &Theme, status: button::Status) -> button::Style {
    let (bg, bc) = match status {
        button::Status::Hovered | button::Status::Pressed => (PANEL_3(), TXT3()),
        _ => (PANEL_2(), LINE_STRONG()),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TXT(),
        border: Border {
            color: bc,
            width: 1.0,
            radius: 15.0.into(),
        },
        shadow: iced::Shadow::default(),
        snap: false,
    }
}

/// 「载入示例 / 重置参数」按钮样式（参考 `.home-start-actions .btn`：
/// `--panel-2` 底 + `--line-strong` 描边，悬停 `--panel-hover` 底 + `--txt-3` 描边）
fn action_btn_style(_t: &Theme, status: button::Status) -> button::Style {
    let (bg, bc) = match status {
        button::Status::Hovered => (PANEL_3(), TXT3()),
        button::Status::Pressed => (PANEL_3(), TXT3()),
        _ => (PANEL_2(), LINE_STRONG()),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TXT(),
        border: Border {
            color: bc,
            width: 1.0,
            radius: R_CARD.into(),
        },
        shadow: iced::Shadow::default(),
        snap: false,
    }
}

/// 卡片小标题（参考 `.home-card-head`：13px / 600 + 7px 图标间距）
fn card_head<'a>(icon_name: &'static str, title: &'a str) -> Element<'a, Message> {
    row![
        icon(icon_name, 15.0, TXT2()),
        text(title).size(FS).color(TXT()).font(bold_font()),
    ]
    .spacing(7)
    .align_y(Alignment::Center)
    .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    // 分步引导当前步骤（Some = 引导进行中，对应区域加高亮描边）
    let g = app.guide_step;

    /* ── 顶部：标题 / 副标 / 新手引导（参考 `.home-head`：三段全部居中，
    标题→副标 9px、副标→引导 12px；引导是 inline-flex，居中而非右上角） ── */
    let head: Element<'_, Message> = container(
        column![
            text(tr("壹鸣AI智能抠图 · 专业证件照工作台"))
                .size(FS_H1)
                .color(TXT())
                .font(bold_font()),
            text(tr("上传一张照片，一键生成透明底图片，证件照智能一键排版，设计师的效率神器。赶快试试吧！"))
                .size(12.0)
                .color(TXT2()),
            // 副标 → 引导多出的 3px（列间距 9 + 3 = 参考的 margin-top:12px）
            container(
                button(
                    container(
                        row![
                            icon("help-circle", 14.0, TXT2()),
                            text(tr("新手引导")).size(FS_SM).color(TXT()),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .height(Length::Fill)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::Home(HomeMessage::StartGuide))
                .padding(pad(0.0, 12.0, 0.0, 12.0))
                .height(Length::Fixed(30.0))
                .style(guide_style),
            )
            .padding(pad(3.0, 0.0, 0.0, 0.0)),
        ]
        .spacing(9)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .into();

    /* ── 左：开始卡（`min-height:300px` 的虚线拖放区 + 双操作按钮） ──
    注意：**不要**给虚线框再套 `center_y(Length::Fill)`——那会把按钮
    `height(Fixed)` 覆盖成 Fill，进而在滚动容器（高度无界）里解析成 ∞，
    整张卡片会被渲染成一个无穷大的矩形而被 GPU 丢弃（表现为整卡消失）。 */
    // 「开始」卡拖放区：点击 = 直达抠图工作台 + 弹文件选择器（无中间过渡页）；
    // 有文件拖拽悬停在窗口上时，虚线框改用强调色描边给出「可松手」反馈
    let drop_border = if app.drop_hover { ACC() } else { LINE_STRONG() };
    let dropzone = button(dashed_box(
        column![
            // `.home-drop-ic{...; margin-bottom:7px}`：图标块与标题之间多 7px
            container(
                container(icon("image", 21.0, TXT2()))
                    .width(Length::Fixed(42.0))
                    .height(Length::Fixed(42.0))
                    .center_x(Length::Fixed(42.0))
                    .center_y(Length::Fixed(42.0))
                    .style(icon_tile(PANEL_3(), 11.0)),
            )
            .padding(pad(0.0, 0.0, 7.0, 0.0)),
            text(tr("点击或拖拽照片到这里"))
                .size(FS)
                .color(TXT())
                .font(bold_font()),
            text(tr("支持 JPG / PNG / WebP，照片仅在本地处理"))
                .size(11.5)
                .color(TXT3()),
        ]
        .spacing(5)
        .align_x(Alignment::Center),
        drop_border,
        R_PANEL,
        Some(PANEL_2()),
    ))
    .on_press(Message::Home(HomeMessage::OpenPhoto))
    .padding(0)
    .width(Length::Fill)
    .height(Length::Fixed(DROP_H))
    .style(|_t, _s| button::Style {
        background: None,
        text_color: TXT(),
        border: Border::default(),
        shadow: iced::Shadow::default(),
        snap: false,
    });

    let start: Element<'_, Message> = container(
        column![
            card_head("image", tr("开始")),
            dropzone,
            row![
                action_btn("image", tr("载入示例"), HomeMessage::LoadSample),
                action_btn("rotate-ccw", tr("重置参数"), HomeMessage::ResetParams),
            ]
            .spacing(10),
        ]
        .spacing(12),
    )
    .padding(pad_all(14.0)) // 参考 `.home-card{padding:14px}`
    .style(move |t| if g == Some(0) { card_hi(t) } else { card(t) })
    .width(Length::Fill)
    .into();
    // 聚焦引导：draw 时上报自身 bounds（挖洞 / 气泡锚定数据源）
    let start: Element<'_, Message> = guide::Tracked::new(Target::StartCard, start).into();

    /* ── 右：快速预设（2 列网格，参考 `.home-presets{gap:10px}`） ── */
    let mut preset_rows: iced::widget::Column<'_, Message> = column![].spacing(10);
    for (row_idx, chunk) in PRESETS.chunks(2).enumerate() {
        let mut r = row![].spacing(10);
        for (j, (ic, title, sub, spec, bg)) in chunk.iter().enumerate() {
            let i = row_idx * 2 + j;
            let active = app.id.spec_idx == *spec && app.id.bg_idx == *bg;
            r = r.push(
                button(
                    row![
                        container(icon(ic, 15.0, TXT2()))
                            .width(Length::Fixed(30.0))
                            .height(Length::Fixed(30.0))
                            .center_x(Length::Fixed(30.0))
                            .center_y(Length::Fixed(30.0))
                            .style(icon_tile(PANEL_3(), 8.0)),
                        column![
                            text(tr(title)).size(12.5).color(TXT()).font(bold_font()),
                            // `.hp-s{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}`：
                            // 原生版没有省略号，用「不换行 + 裁切」等效——保证每张卡都是单行等高。
                            container(
                                text(tr(sub))
                                    .size(11.0)
                                    .color(TXT3())
                                    .wrapping(text::Wrapping::None),
                            )
                            .width(Length::Fill)
                            .clip(true),
                        ]
                        .spacing(1)
                        .width(Length::Fill),
                        icon("chevron-right", 14.0, TXT3()),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::Home(HomeMessage::ApplyPreset(i)))
                .padding(pad(10.0, 11.0, 10.0, 11.0)) // 参考 `.home-preset{padding:10px 26px 10px 11px}`（右侧 26px 是给绝对定位箭头留的位，原生版箭头在流内，用 11px）
                .width(Length::Fill)
                .style(preset_style(active)),
            );
        }
        // 奇数个补一个占位，保证网格对齐
        if chunk.len() == 1 {
            r = r.push(Space::new().width(Length::Fill));
        }
        preset_rows = preset_rows.push(r);
    }
    let preset_card: Element<'_, Message> =
        container(column![card_head("zap", tr("快速预设")), preset_rows].spacing(12))
            .padding(pad_all(14.0))
            .style(move |t| if g == Some(1) { card_hi(t) } else { card(t) })
            .width(Length::Fill)
            .into();
    let preset_card: Element<'_, Message> =
        guide::Tracked::new(Target::Presets, preset_card).into();

    /* ── 右：当前状态（参考 .home-status 的 kv 字段） ── */
    let spec = &ID_SIZES[app.id.spec_idx.min(ID_SIZES.len() - 1)];
    let (px_w, px_h) = spec.px(app.id.dpi);
    let has_modnet = app.work.nn_models.iter().any(|m| m.file == "modnet.onnx");
    let engine = if has_modnet {
        tr("内置算法 · MODNet + 人脸定位")
    } else {
        tr("内置算法 · SmartMatte 色键")
    };
    let paper = &ID_PAPERS[app.id.paper_idx.min(ID_PAPERS.len() - 1)];
    // 参考 `.home-kv-row{padding:8px 2px;border-bottom:1px solid var(--line-soft)}`，
    // 末行无分隔线。iced 的 Border 无「单边」能力，用 1px 细条容器代替。
    let rows: [(&str, String); 4] = [
        // 参考 home.js：`规格`显示「一寸 25×35mm」形式（名称 + 毫米/像素尺寸）
        (tr("证件规格"), format!("{} {}", spec.n, spec.dim_text())),
        (tr("输出像素"), format!("{}×{}px", px_w, px_h)),
        (tr("排版相纸"), paper.n.to_string()),
        (tr("引擎"), engine.to_string()),
    ];
    let mut kv_col: iced::widget::Column<'_, Message> = column![].spacing(0);
    for (i, (k, v)) in rows.into_iter().enumerate() {
        if i > 0 {
            kv_col = kv_col.push(
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(|_t: &Theme| container::Style {
                        background: Some(Background::Color(LINE_SOFT())),
                        ..container::Style::default()
                    }),
            );
        }
        kv_col = kv_col.push(container(kv(k, v)).padding(pad(8.0, 2.0, 8.0, 2.0)));
    }
    let state_card: Element<'_, Message> =
        container(column![card_head("activity", tr("当前状态")), kv_col].spacing(12))
            .padding(pad_all(14.0))
            .style(move |t| if g == Some(2) { card_hi(t) } else { card(t) })
            .width(Length::Fill)
            .into();
    let state_card: Element<'_, Message> = guide::Tracked::new(Target::Status, state_card).into();

    // `.home-cta{height:42px;font-size:13.5px;gap:8px}`；内容用 Fill 容器垂直居中，
    // 避免 Button 把内容放到 padding.top（之前导出按钮偏上就是这个原因）。
    let cta: Element<'_, Message> = button(
        container(
            row![
                text(tr("去制作证件照"))
                    .size(13.5)
                    .color(iced::Color::WHITE)
                    .font(bold_font()),
                icon("arrow-right", 15.0, iced::Color::WHITE),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Navigate(Page::IdPhoto))
    .padding(0)
    .height(Length::Fixed(42.0))
    .width(Length::Fill)
    .style(move |t, s| {
        let mut st = btn_primary(t, s);
        // 引导高亮：强调色描边让 CTA 在遮罩上依然醒目
        if g == Some(3) {
            st.border = Border {
                color: ACC(),
                width: 3.0,
                radius: R_CARD.into(),
            };
        }
        st
    })
    .into();
    let cta: Element<'_, Message> = guide::Tracked::new(Target::MakeBtn, cta).into();

    /* ── 组装（`.home-grid{grid-template-columns:1fr 1fr;gap:14px}`） ── */
    // 行必须显式撑满：iced 的 flex 布局里，`FillPortion` 子元素只有在
    // `compression.width == false`（父级不是 Shrink）时才会按比例平分；
    // 否则 `Limits::resolve` 会让它退化成各自的内在宽度。
    //
    // 响应式：参考项目在窄视口下两列会退化为单列堆叠（`--content-w < 740px`）。
    // iced 拿不到容器宽，只能用窗口宽估算：减去侧栏（展开 227 / 折叠 75）与
    // 外壳留白（10×2 + gap 10），得到内容区宽度，<740 时切单列。
    let est_content_w = app.win_w - if app.nav_collapsed { 105.0 } else { 257.0 };
    let right_col: Element<'_, Message> =
        container(column![preset_card, state_card, cta].spacing(14))
            .width(Length::Fill)
            .into();
    let grid: Element<'_, Message> = if est_content_w >= 740.0 {
        row![container(start).width(Length::FillPortion(1)), right_col,]
            .spacing(14)
            .width(Length::Fill)
            .into()
    } else {
        // 窄窗口：单列堆叠（开始卡 → 快速预设 → 当前状态 → CTA）
        column![container(start).width(Length::Fill), right_col]
            .spacing(14)
            .width(Length::Fill)
            .into()
    };

    // 参考：`.home{flex:1;display:flex;padding:28px 32px}` +
    // `.home-wrap{width:100%;max-width:900px;margin:auto}`
    // → 内容块**限宽 900** 且在页面内**水平 + 垂直居中**。
    // iced 侧对应两点：
    // 1) 限宽：内层容器 `max_width(900)`。scrollable 给内容的 width 上限就是
    //    视口宽，所以 `width(Fill) + max_width(900)` == CSS 的
    //    `width:100% + max-width`（视口更窄时自动收缩，不会溢出）；
    // 2) 居中：`Scrollable` 默认 Shrink×Shrink（自身尺寸=内容尺寸），外层容器
    //    `align_y(Center)` 即等价于 `margin:auto` 的垂直方向——内容超高时
    //    scrollable 高度被 `resolve(Shrink)` clamp 到视口高，边距自动归零 →
    //    顶对齐 + 内部滚动，与 CSS auto 边距的行为一致。
    //
    //    水平方向**不能**只靠外层容器 `align_x`：外壳 `row![nav, content_area]`
    //    是 Shrink 行，链式传播的 `compression.width=true` 会让外层 Fill 容器
    //    解析成**内在宽度**（= scrollable 宽度），align_x 无从居中。
    //    而 scrollable 给内容的 limits 里 `compression.width=false`，所以在
    //    scrollable 内部多包一层 `width(Fill) + align_x(Center)` 的中间容器，
    //    它才能真正撑满视口宽，把限宽 900 的 wrap 水平居中。
    //
    // ⚠️ 方向必须是 **Vertical**：`Direction::Both` 会给内容无界（∞）的
    //    width 上限（scrollable.rs 的 child_limits：is_horizontal →
    //    max.width = f32::INFINITY），内容里所有 `width(Fill)` 容器会按
    //    ∞ 解析或退化，整棵视图树尺寸失真、被 GPU 整体丢弃 → 页面空白。
    //    水平溢出场景（预设副标 nowrap）由 `clip(true)` 裁切，无需横向滚动。
    let home_root: Element<'_, Message> = container(
        scrollable(
            container(
                container(column![head, grid].spacing(26))
                    .max_width(900.0)
                    .width(Length::Fill)
                    .padding(pad(28.0, 32.0, 28.0, 32.0)),
            )
            .width(Length::Fill)
            .align_x(Alignment::Center),
        )
        .direction(Direction::Vertical(
            Scrollbar::new().width(3).scroller_width(3),
        ))
        .style(hidden_scrollbar_style),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_y(Alignment::Center)
    .style(app_bg)
    .into();
    // 聚焦引导坐标系基准（洞绝对坐标 → 遮罩画布相对坐标的换算依据）
    let home_root: Element<'_, Message> = guide::Tracked::new(Target::Root, home_root).into();

    // 分步新手引导：进行中时在首页内容之上叠放遮罩 + 步骤卡
    let base: Element<'_, Message> = home_root;
    if let Some(step) = app.guide_step {
        iced::widget::stack![base, guide_overlay(app, step)]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    } else {
        base
    }
}

/// 引导高亮卡片样式：常规卡 + 强调色描边 + 提升阴影（高亮目标区域用）
fn card_hi(t: &Theme) -> container::Style {
    let mut st = card(t);
    st.border = Border {
        color: ACC(),
        width: 2.0,
        radius: R_CARD.into(),
    };
    st.shadow = shadow_4();
    st
}

/// 分步新手引导：聚焦式（Spotlight）——洞外压暗、目标挖洞高亮，
/// 说明气泡锚定在洞旁（guide::spotlight 负责定位与事件穿透）。
fn guide_overlay<'a>(app: &'a App, step: usize) -> Element<'a, Message> {
    let (target, _, _) = GUIDE_STEPS[step];
    // 洞外扩 6px 呼吸空间，让描边不贴住控件边缘
    let hole_abs = guide::bounds(target).map(|r| Rectangle {
        x: r.x - 6.0,
        y: r.y - 6.0,
        width: r.width + 12.0,
        height: r.height + 12.0,
    });
    guide::spotlight(hole_abs, guide_bubble(app, step))
}

/// 引导说明气泡：徽标 + 标题 + 步数 → 说明 → 进度点 → 操作行
fn guide_bubble<'a>(_app: &'a App, step: usize) -> Element<'a, Message> {
    let (_, title, desc) = GUIDE_STEPS[step];
    let last = step + 1 >= GUIDE_STEPS.len();

    // 头部：当前步徽标 + 标题 + n/4
    let badge = container(
        text(format!("{}", step + 1))
            .size(FS_XS)
            .color(iced::Color::WHITE),
    )
    .width(iced::Length::Fixed(22.0))
    .height(iced::Length::Fixed(22.0))
    .center_x(iced::Length::Fixed(22.0))
    .center_y(iced::Length::Fixed(22.0))
    .style(|_t| container::Style {
        background: Some(iced::Background::Color(ACC())),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 11.0.into(),
        },
        ..container::Style::default()
    });

    let head = row![
        badge,
        text(tr(title)).size(FS_H).color(TXT()).font(bold_font()),
        Space::new().width(iced::Length::Fill),
        text(trf(
            "{}/{} 步",
            &[&format!("{}", step + 1), &GUIDE_STEPS.len().to_string()],
        ))
        .size(FS_XS)
        .color(TXT3()),
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    // 进度点：当前步加长（pill），其余圆点
    let mut dots = row![].spacing(5);
    for i in 0..GUIDE_STEPS.len() {
        let active = i == step;
        dots = dots.push(
            container(Space::new())
                .width(iced::Length::Fixed(if active { 16.0 } else { 6.0 }))
                .height(iced::Length::Fixed(6.0))
                .style(move |_t: &Theme| container::Style {
                    background: Some(iced::Background::Color(if active {
                        ACC()
                    } else {
                        LINE_STRONG()
                    })),
                    border: Border {
                        color: Color::TRANSPARENT,
                        width: 0.0,
                        radius: 3.0.into(),
                    },
                    ..container::Style::default()
                }),
        );
    }

    // 操作行：跳过（=不再自动弹出）··· 上一步 / 下一步（末步为完成）
    let skip_btn = button(text(tr("跳过")).size(FS_SM).color(TXT2()))
        .padding(pad(6.0, 12.0, 6.0, 12.0))
        .style(btn_ghost)
        .on_press(Message::Home(HomeMessage::GuideNever));
    let prev_btn = {
        let enabled = step > 0;
        button(
            text(tr("上一步"))
                .size(FS_SM)
                .color(if enabled { TXT() } else { TXT3() }),
        )
        .padding(pad(6.0, 14.0, 6.0, 14.0))
        .style(btn_ghost)
        .on_press_maybe(enabled.then_some(Message::Home(HomeMessage::GuidePrev)))
    };
    let next_label = if last { tr("完成") } else { tr("下一步") };
    let next_btn = button(
        text(next_label)
            .size(FS_SM)
            .color(iced::Color::WHITE)
            .font(bold_font()),
    )
    .padding(pad(6.0, 18.0, 6.0, 18.0))
    .style(btn_primary)
    .on_press(Message::Home(HomeMessage::GuideNext));

    container(
        column![
            head,
            text(tr(desc)).size(FS_SM).color(TXT2()),
            dots,
            row![
                skip_btn,
                Space::new().width(iced::Length::Fill),
                prev_btn,
                next_btn,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(12),
    )
    .width(iced::Length::Fixed(328.0))
    .padding(pad_all(CARD_PAD))
    .style(|_t| container::Style {
        text_color: Some(TXT()),
        background: Some(iced::Background::Color(PANEL())),
        border: Border {
            color: LINE(),
            width: 1.0,
            radius: R_CARD.into(),
        },
        shadow: shadow_4(),
        snap: false,
    })
    .into()
}

/// 「载入示例 / 重置参数」按钮（参考 `.home-start-actions .btn`：等高、等宽、
/// 图标 + 文字整体居中）
fn action_btn<'a>(ic: &'static str, label: &'a str, msg: HomeMessage) -> Element<'a, Message> {
    button(
        container(
            row![icon(ic, 14.0, TXT2()), text(label).size(FS_SM).color(TXT()),]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Home(msg))
    .padding(pad(0.0, 12.0, 0.0, 12.0))
    .height(Length::Fixed(34.0))
    .width(Length::Fill)
    .style(action_btn_style)
    .into()
}

/// 首页滚动条的完全隐藏样式：任何状态下（含悬停、拖拽）滑块与轨道均透明，
/// 保留滚动能力但不显示任何滚动条，实现「无滚动条」观感。
fn hidden_scrollbar_style(_theme: &Theme, _status: scrollable::Status) -> scrollable::Style {
    let scroller = scrollable::Scroller {
        background: Background::Color(Color::TRANSPARENT),
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
