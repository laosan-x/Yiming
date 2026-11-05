//! 设置页：左侧分类导航 + 右侧面板
//!
//! 与原项目一致分七类：模型与推理引擎 / 计算后端 / 模型插件 / 远程后端 /
//! 导出默认值 / 隐私 / 更新日志。

use iced::widget::{button, column, container, pick_list, row, scrollable, text, text_input, Space};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::{Alignment, Color, Element, Length, Task};

use crate::app::{App, UpdateStatus};
use crate::core::image::{ExportFormat, ExportSize};
use crate::core::nn::ModelCategory;
use crate::core::updater;
use crate::message::{Message, ModelKind, SettingsMessage, UpdaterMessage};
use crate::theme::*;
use crate::ui::icons::icon;
use crate::ui::widgets::*;
use crate::ui::widgets::switch as switch_btn;
use crate::core::i18n::{tr, trf};

/// (i18n key, icon) —— 显示名经 `i18n::tr` 翻译
const PANELS: &[(&str, &str)] = &[
    ("panel.engine", "settings"),
    ("panel.backend", "cpu"),
    ("panel.plugin", "puzzle"),
    ("panel.remote", "cloud"),
    ("panel.export", "download"),
    ("panel.hotkey", "keyboard"),
    ("panel.privacy", "lock"),
    ("panel.appearance", "sparkles"),
    ("panel.update", "download"),
    ("panel.changelog", "history"),
];

pub fn update(app: &mut App, msg: SettingsMessage) -> Task<Message> {
    match msg {
        SettingsMessage::SetPanel(i) => app.settings_panel = i,
        SettingsMessage::SetModel(m) => {
            app.config.model = m.id().to_string();
            app.persist();
        }
        SettingsMessage::ToggleAuto(v) => {
            app.config.auto = v;
            app.persist();
        }
        SettingsMessage::SetFormat(f) => {
            app.config.export_format = match f {
                ExportFormat::Png => "png",
                ExportFormat::Jpg => "jpg",
                ExportFormat::Webp => "webp",
            }
            .to_string();
            app.persist();
        }
        SettingsMessage::SetExportSize(s) => {
            app.config.export_size = s.to_u32();
            app.persist();
        }
        SettingsMessage::SetApiUrl(u) => app.config.api_url = u,
        SettingsMessage::SaveApiUrl => {
            app.persist();
            app.toast(trf("后端地址已保存：{}", &[&app.config.api_url]));
        }
        SettingsMessage::PickPluginDir => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                |p| Message::Settings(SettingsMessage::PluginDirPicked(p)),
            );
        }
        SettingsMessage::PluginDirPicked(Some(p)) => {
            app.config.plugin_dir = Some(p.to_string_lossy().to_string());
            app.persist();
            app.toast(trf("插件目录：{}", &[p.to_string_lossy().as_ref()]));
        }
        SettingsMessage::PluginDirPicked(None) => {}
        SettingsMessage::ResetParams => {
            app.work.params = crate::core::matting::MatteParams::default();
            app.persist();
            app.toast(tr("蒙版参数已恢复默认"));
        }
        SettingsMessage::ClearCache => {
            // 模型会话：下一次推理重新加载（首张稍慢），换回显存/内存
            crate::core::nn::release_all();
            // 撤销历史（含快照引用的像素）与色键场
            app.work.history.clear();
            *app.work.fields.lock().unwrap_or_else(|p| p.into_inner()) =
                crate::core::matting::FieldCache::default();
            // 证件照页与工作台的中间缓冲
            app.id.fg_preview = None;
            app.id.prefetch = None;
            app.id.fg_eroded = None;
            app.id.erode_cache_key = None;
            app.toast(tr("已释放模型会话与中间缓存（下次推理首张稍慢）"));
        }
        SettingsMessage::ClearRecent => {
            app.config.recent.clear();
            app.persist();
            app.toast(tr("最近文件已清空"));
        }
        SettingsMessage::RescanModels => {
            let plugin_dir = app.config.plugin_dir.as_deref();
            let (dir, models) = crate::core::nn::scan(plugin_dir);
            app.work.models_dir = dir;
            app.work.nn_models = models;
            app.toast(trf("模型扫描完成：发现 {} 个模型", &[&app.work.nn_models.len().to_string()]));
        }
        SettingsMessage::RefreshDpi => {}
    }
    Task::none()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let mut nav: iced::widget::Column<'_, Message> = column![].spacing(3);
    for (i, (label, icon_name)) in PANELS.iter().enumerate() {
        let active = app.settings_panel == i;
        let ic_color = if active { INK() } else { TXT2() };
        nav = nav.push(
            button(
                row![
                    icon(icon_name, 15.0, ic_color),
                    text(crate::core::i18n::tr(label)).size(FS_SM),
                ]
                .spacing(9)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Settings(SettingsMessage::SetPanel(i)))
            .padding(pad(8.0, 11.0, 8.0, 11.0))
            .width(Length::Fill)
            .style(nav_btn(active)),
        );
    }

    let nav_col: Element<'_, Message> = container(column![text(tr("设置分类")).size(FS_XS).color(TXT3()), nav].spacing(8))
        .padding(pad(14.0, 10.0, 14.0, 10.0))
        .width(Length::Fixed(196.0))
        .height(Length::Fill)
        .style(panel)
        .into();

    let body = match app.settings_panel {
        0 => engine_panel(app),
        1 => backend_panel(app),
        2 => plugin_panel(app),
        3 => remote_panel(app),
        4 => export_panel(app),
        5 => hotkey_panel(app),
        6 => privacy_panel(app),
        7 => appearance_panel(app),
        8 => update_panel(app),
        _ => changelog_panel(),
    };

    // 参考 `.settings-layout{display:flex;gap:28px}`：分类导航与内容区之间 28px
    row![nav_col, body].spacing(28.0).into()
}

/* ═════════════════ 各面板 ═════════════════ */

fn set_row<'a>(
    label: &'a str,
    desc: impl Into<String>,
    right: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        row![
            column![
                text(label).size(FS).color(TXT()),
                text(desc.into()).size(FS_XS).color(TXT3()),
            ]
            .spacing(4)
            .width(Length::Fill),
            right,
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(pad(SP_4, CARD_PAD, SP_4, CARD_PAD))
    .width(Length::Fill)
    .style(panel)
    .into()
}

/* ═════════════════ 大卡片（Section）组件 ═════════════════ */

/// 大卡片内的一行：沿用 `set_row` 的「标签 / 说明 + 右侧控件」结构，
/// 但不带独立边框与阴影——边框由 [`section_card`] 统一提供，行间只用
/// 1px `LINE_SOFT` 分隔，避免「每个设置项都是一张卡」的碎片感。
/// 行内控件、消息与持久化行为完全不变。
fn section_row<'a>(
    label: &'a str,
    desc: impl Into<String>,
    right: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        row![
            column![
                text(label).size(FS).color(TXT()),
                text(desc.into()).size(FS_XS).color(TXT3()),
            ]
            .spacing(4)
            .width(Length::Fill),
            right,
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    // 左右内边距由大卡片的 CARD_PAD 提供，这里只留上下呼吸
    .padding(pad(SP_4, 0.0, SP_4, 0.0))
    .width(Length::Fill)
    .into()
}

/// 模型库内的类别小节：加粗标题 + 描述 + 右侧数量徽标。
/// 刻意不加背景/边框，否则又会退化成「一类一张卡」。
fn section_subhead<'a>(
    label: &'a str,
    desc: &'a str,
    right: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        row![
            column![
                text(label).size(FS).color(TXT()).font(bold_font()),
                text(desc).size(FS_XS).color(TXT3()),
            ]
            .spacing(2)
            .width(Length::Fill),
            right,
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(pad(10.0, 0.0, 6.0, 0.0))
    .width(Length::Fill)
    .into()
}

/// 大卡片：标题（+ 可选右侧主操作）+ 若干行，行间 1px 分隔线。
///
/// 只在视图层做视觉分组：不新增状态、不改消息、不在卡片内嵌套
/// `scrollable`（外层 `panel_wrap` 已是一层垂直滚动，iced 0.14 嵌套
/// 滚动会让 `Length::Fill` 尺寸失真）。
fn section_card<'a>(
    title: &'a str,
    action: Option<Element<'a, Message>>,
    rows: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut head =
        row![text(title).size(FS_H).color(TXT()).font(bold_font())].align_y(Alignment::Center);
    if let Some(a) = action {
        head = head.push(Space::new().width(Length::Fill)).push(a);
    }

    let mut inner: iced::widget::Column<'a, Message> =
        column![head, Space::new().height(8.0)].spacing(0);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            inner = inner.push(
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fixed(1.0))
                    .style(|_t| iced::widget::container::Style {
                        background: Some(iced::Background::Color(LINE_SOFT())),
                        ..iced::widget::container::Style::default()
                    }),
            );
        }
        inner = inner.push(r);
    }

    container(inner)
        .padding(pad_all(CARD_PAD))
        .width(Length::Fill)
        .style(card)
        .into()
}

fn panel_wrap(title: String, content: Element<'_, Message>) -> Element<'_, Message> {
    container(
        scrollable(
            container(column![text(title).size(16.0).color(TXT()), content].spacing(12))
                .padding(pad(22.0, 22.0, 40.0, 22.0)),
        )
        .direction(Direction::Vertical(Scrollbar::new().width(3).scroller_width(3)))
        .style(scrollbar_style),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(app_bg)
    .into()
}

fn engine_panel<'a>(app: &'a App) -> Element<'a, Message> {
    let has_modnet = app.work.nn_models.iter().any(|m| m.file == "modnet.onnx");

    /* ① 引擎配置：可编辑项 + 底部「恢复默认参数」动作 */
    let config_rows: Vec<Element<'a, Message>> = vec![
        section_row(
            tr("抠图模型"),
            tr("经典算法为纯色 / 白底商品图设计，毫秒级；神经网络模型经 ONNX Runtime 本地推理。"),
            container(
                pick_list(
                    ModelKind::ALL,
                    Some(ModelKind::from_id(&app.config.model)),
                    |m| Message::Settings(SettingsMessage::SetModel(m)),
                )
                .width(Length::Fixed(262.0))
                .style(pick_list_style)
                .text_size(FS_SM)
                .menu_style(menu_style),
            )
            .into(),
        ),
        section_row(
            tr("自动重算"),
            tr("调整参数后即时重新推理；关闭后需手动切换模型触发。"),
            switch_btn(
                app.config.auto,
                Message::Settings(SettingsMessage::ToggleAuto(!app.config.auto)),
            ),
        ),
        section_row(
            tr("蒙版参数"),
            tr("取样阈值 / 边缘宽容 / 羽化 / 形态学等参数见「抠图工具」右侧检查器。"),
            container(
                button(text(tr("恢复默认参数")).size(FS_SM))
                    .on_press(Message::Settings(SettingsMessage::ResetParams))
                    .padding(pad(6.0, 12.0, 6.0, 12.0))
                    .style(btn_secondary),
            )
            .into(),
        ),
    ];

    /* ② 运行时状态：只读诊断信息（徽标状态与文案原样保留） */
    let status_rows: Vec<Element<'a, Message>> = vec![
        section_row(
            tr("推理运行时"),
            tr("ONNX Runtime（load-dynamic）：Windows 优先 DirectML GPU 执行，未安装运行库时回退 CPU。"),
            badge(tr("本地推理"), BadgeKind::Ok),
        ),
        section_row(
            "MODNet",
            if has_modnet {
                tr("人像发丝级模型（models/modnet.onnx）已就绪，证件照页自动优先使用。")
            } else {
                tr("未在 models/ 目录找到 modnet.onnx；证件照页将回退 SmartMatte 色键抠图。")
            },
            if has_modnet {
                badge(tr("可用"), BadgeKind::Ok)
            } else {
                badge(tr("未就绪"), BadgeKind::Warn)
            },
        ),
        section_row(
            tr("人脸检测模型"),
            tr("YuNet 权重在原项目中已判定不可用，原生版同样不启用。"),
            badge(tr("已停用"), BadgeKind::Warn),
        ),
        section_row(
            tr("内存提示"),
            tr("蒙版计算按像素流式处理，峰值内存约为图像 RGBA 的 3~4 倍。超大图请先缩小。"),
            Space::new().width(1.0).into(),
        ),
    ];

    /* ③ 模型库：清单摘要 + 按类别分组的模型条目（类别标题降为卡内小节） */
    let mut model_rows: Vec<Element<'a, Message>> = vec![section_row(
        tr("模型清单"),
        match (&app.work.models_dir, app.work.nn_models.len()) {
            (Some(d), n) => trf(
                "目录 {} · 发现 {} 个模型",
                &[&d.display().to_string(), &n.to_string()],
            ),
            (None, _) => {
                tr("models/ 目录不存在，点击「重新扫描」或把 .onnx 放入该目录").to_string()
            }
        },
        Space::new().width(1.0).into(),
    )];
    // 与工作台「引擎」面板同一份数据，类别排序 / 去重 / 过滤逻辑不变
    let mut categories: Vec<ModelCategory> = app
        .work
        .nn_models
        .iter()
        .map(|m| crate::core::nn::model_category(&m.file))
        .collect();
    categories.sort();
    categories.dedup();
    for cat in &categories {
        let models_in_cat: Vec<&crate::core::nn::NnModelInfo> = app
            .work
            .nn_models
            .iter()
            .filter(|m| crate::core::nn::model_category(&m.file) == *cat)
            .collect();
        if models_in_cat.is_empty() {
            continue;
        }
        model_rows.push(section_subhead(
            cat.label(),
            cat.desc(),
            badge(format!("{}", models_in_cat.len()), BadgeKind::Dim),
        ));
        for m in &models_in_cat {
            model_rows.push(section_row(
                &m.label,
                trf(
                    "{} MB · 输入 {}px",
                    &[&format!("{:.1}", m.size_mb), &m.input_size.to_string()],
                ),
                Space::new().width(1.0).into(),
            ));
        }
    }

    // 「重新扫描」提升到模型库卡片头部，仍是原来的 RescanModels 消息
    let rescan_action: Element<'a, Message> = container(
        button(text(tr("重新扫描")).size(FS_SM))
            .on_press(Message::Settings(SettingsMessage::RescanModels))
            .padding(pad(6.0, 12.0, 6.0, 12.0))
            .style(btn_secondary),
    )
    .into();

    let col = column![
        section_card(tr("引擎配置"), None, config_rows),
        section_card(tr("运行时状态"), None, status_rows),
        section_card(tr("模型库"), Some(rescan_action), model_rows),
    ]
    .spacing(10);

    panel_wrap(tr("模型与推理引擎").to_string(), col.into())
}

fn backend_panel<'a>(app: &'a App) -> Element<'a, Message> {
    let dpi_info = match app.dpi_detected {
        Some(true) => {
            let scale = app.system_dpi / 96.0;
            trf("系统 DPI {}（缩放 {}%）", &[&app.system_dpi.to_string(), &((scale * 100.0).round() as u32).to_string()])
        }
        Some(false) => tr("检测失败，使用默认值 96").to_string(),
        None => tr("尚未检测").to_string(),
    };
    // 「刷新」保留在 DPI 行内（它只刷新 DPI 探测，放卡头会让人误以为是全局刷新）
    panel_wrap(
        tr("计算后端").to_string(),
        section_card(
            tr("运行环境"),
            None,
            vec![
                section_row(
                    tr("计算后端"),
                    tr("色键抠图在 CPU（纯 Rust + rayon 并行）；ONNX 模型推理由 ONNX Runtime 执行，Windows 下优先 DirectML GPU 执行提供程序，缺失时自动回退 CPU。"),
                    badge("CPU + DirectML", BadgeKind::Ok),
                ),
                section_row(
                    tr("并行度"),
                    trf(
                        "可用并行度 {}（由 std::thread::available_parallelism 探测）",
                        &[&std::thread::available_parallelism().map(|v| v.get()).unwrap_or(1).to_string()],
                    ),
                    Space::new().width(1.0).into(),
                ),
                section_row(
                    tr("屏幕 DPI"),
                    dpi_info,
                    container(
                        button(row![icon("refresh", 13.0, TXT2()), text(tr("刷新")).size(FS_SM)].spacing(5).align_y(Alignment::Center))
                            .on_press(Message::Settings(SettingsMessage::RefreshDpi))
                            .padding(pad(5.0, 10.0, 5.0, 10.0))
                            .style(btn_secondary),
                    )
                    .into(),
                ),
            ],
        ),
    )
}

fn plugin_panel<'a>(app: &'a App) -> Element<'a, Message> {
    /* ── 卡① 模型目录与扫描：目录配置 + 「重新扫描」提升到卡头 ── */
    let dir_rows = vec![
        section_row(
            tr("模型目录"),
            match &app.work.models_dir {
                Some(d) => trf("启动时自动扫描 {}，发现的 .onnx 可在证件照 / 引擎面板直接选用", &[d.display().to_string().as_str()]),
                None => tr("models/ 目录不存在：在工作目录创建 models/ 并放入 .onnx 后点击「重新扫描」").to_string(),
            },
            Space::new().width(1.0).into(),
        ),
        // 记录用的插件目录（保留原设置项）
        section_row(
            tr("外置插件目录"),
            tr("指定额外目录后，点击「重新扫描」即可将外置 .onnx 模型加入清单。同名文件以主模型目录为准。"),
            {
                let dir = app
                    .config
                    .plugin_dir
                    .clone()
                    .unwrap_or_else(|| tr("未设置插件目录").to_string());
                container(
                    row![
                        text(dir).size(FS_SM).color(TXT2()),
                        Space::new().width(8.0),
                        button(text(tr("选择目录…")).size(FS_SM))
                            .on_press(Message::Settings(SettingsMessage::PickPluginDir))
                            .padding(pad(5.0, 10.0, 5.0, 10.0))
                            .style(btn_secondary),
                    ]
                    .align_y(Alignment::Center),
                )
                .into()
            },
        ),
    ];
    let rescan_action: Element<'a, Message> = container(
        button(text(tr("重新扫描")).size(FS_SM))
            .on_press(Message::Settings(SettingsMessage::RescanModels))
            .padding(pad(6.0, 12.0, 6.0, 12.0))
            .style(btn_secondary),
    )
    .into();

    /* ── 卡② 已发现模型：与引擎面板「模型库」同一份数据与行组件 ── */
    let mut model_rows: Vec<Element<'a, Message>> = Vec::new();
    if app.work.nn_models.is_empty() {
        model_rows.push(section_row(
            tr("已发现模型"),
            tr("无 —— 目录为空或尚未扫描"),
            Space::new().width(1.0).into(),
        ));
    } else {
        let mut cats: Vec<ModelCategory> = app
            .work
            .nn_models
            .iter()
            .map(|m| crate::core::nn::model_category(&m.file))
            .collect();
        cats.sort();
        cats.dedup();
        for cat in &cats {
            let in_cat: Vec<&crate::core::nn::NnModelInfo> = app
                .work
                .nn_models
                .iter()
                .filter(|m| crate::core::nn::model_category(&m.file) == *cat)
                .collect();
            if in_cat.is_empty() {
                continue;
            }
            model_rows.push(section_subhead(
                cat.label(),
                cat.desc(),
                badge(format!("{}", in_cat.len()), BadgeKind::Dim),
            ));
            for m in &in_cat {
                model_rows.push(section_row(
                    &m.label,
                    trf("{} MB · 输入 {}px", &[&format!("{:.1}", m.size_mb), &m.input_size.to_string()]),
                    badge("ONNX", BadgeKind::Ok),
                ));
            }
        }
    }

    let col = column![
        section_card(tr("模型目录与扫描"), Some(rescan_action), dir_rows),
        section_card(tr("已发现模型"), None, model_rows),
    ]
    .spacing(10);

    panel_wrap(tr("模型插件").to_string(), col.into())
}

fn remote_panel<'a>(app: &'a App) -> Element<'a, Message> {
    let c = container(
        column![set_row(
            tr("后端地址"),
            tr("POST multipart/form-data，字段名 file，返回 PNG 图片流。仅「RMBG-2.0」模型选项会用到。"),
            container(
                row![
                    text_input("http://127.0.0.1:8000/remove", &app.config.api_url)
                        .on_input(|v| Message::Settings(SettingsMessage::SetApiUrl(v)))
                        .size(FS_SM)
                        .width(Length::Fixed(260.0))
                        .style(text_input_style),
                    Space::new().width(8.0),
                    button(text(tr("保存")).size(FS_SM).color(iced::Color::WHITE))
                        .on_press(Message::Settings(SettingsMessage::SaveApiUrl))
                        .padding(pad(6.0, 14.0, 6.0, 14.0))
                        .style(btn_primary),
                ]
                .align_y(Alignment::Center),
            )
            .into(),
        )]
        .spacing(10),
    )
    .into();
    panel_wrap(tr("远程后端").to_string(), c)
}

fn export_panel<'a>(app: &'a App) -> Element<'a, Message> {
    panel_wrap(
        tr("导出默认值").to_string(),
        section_card(
            tr("导出默认值"),
            None,
            vec![
                section_row(
                    tr("格式"),
                    tr("PNG 保留透明通道，JPG 体积最小但会强制白底。"),
                    container(
                        pick_list(
                            ExportFormat::all(),
                            Some(app.config.export_format()),
                            |f| Message::Settings(SettingsMessage::SetFormat(f)),
                        )
                        .width(Length::Fixed(262.0))
                        .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
                    )
                    .into(),
                ),
                section_row(
                    tr("导出尺寸"),
                    tr("选择非原始尺寸时，图片会等比缩放并居中放置于方画布。"),
                    container(
                        pick_list(
                            ExportSize::all(),
                            Some(app.config.export_size()),
                            |s| Message::Settings(SettingsMessage::SetExportSize(s)),
                        )
                        .width(Length::Fixed(262.0))
                        .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
                    )
                    .into(),
                ),
            ],
        ),
    )
}

fn hotkey_panel<'a>(app: &'a App) -> Element<'a, Message> {
    use crate::core::hotkey::{self, HotkeyAction};

    let keymap = app.config.keymap.resolve();

    // 单张大卡片：说明行在前，随后每个分组一个小节标题 + 绑定行；
    // 行间 1px 分隔由 section_card 统一绘制，原 PANEL_2 底色小卡取消。
    let mut rows: Vec<Element<'a, Message>> = vec![text(
        tr("快捷键与 Adobe Photoshop 保持一致。文本输入框聚焦时快捷键自动失效。"),
    )
    .size(FS_SM)
    .color(TXT3())
    .into()];

    // 分组标题
    let groups = [
        (tr("工具切换"), vec![
            HotkeyAction::ToolAuto, HotkeyAction::ToolKeep, HotkeyAction::ToolErase,
            HotkeyAction::ToolPicker, HotkeyAction::ToolPan, HotkeyAction::ToolCrop,
        ]),
        (tr("视图"), vec![
            HotkeyAction::ZoomFit, HotkeyAction::ZoomIn, HotkeyAction::ZoomOut,
            HotkeyAction::ViewCompare, HotkeyAction::ViewOriginal, HotkeyAction::ViewResult,
        ]),
        (tr("编辑"), vec![
            HotkeyAction::Undo, HotkeyAction::ResetMask, HotkeyAction::Export,
            HotkeyAction::OpenImage,
            HotkeyAction::RotateCW, HotkeyAction::RotateCCW,
        ]),
        (tr("画笔"), vec![
            HotkeyAction::BrushSizeUp, HotkeyAction::BrushSizeDown,
        ]),
        (tr("其他"), vec![
            HotkeyAction::ToggleNav, HotkeyAction::CropConfirm, HotkeyAction::CropCancel,
        ]),
    ];

    for (group, actions) in &groups {
        rows.push(section_subhead(group, "", Space::new().width(1.0).into()));
        for action in actions {
            let binding = keymap.get(action).map(|(m, k)| hotkey::format_binding(m, k)).unwrap_or_default();
            rows.push(section_row(
                hotkey::action_label(action),
                "",
                text(binding).size(FS_SM).color(TXT2()).into(),
            ));
        }
    }

    panel_wrap(tr("快捷键").to_string(), section_card(tr("快捷键"), None, rows))
}

fn privacy_panel<'a>(app: &'a App) -> Element<'a, Message> {
    let engine_txt = if app.config.api_url.is_empty() {
        tr("未配置").to_string()
    } else {
        app.config.api_url.clone()
    };
    // 卡① 隐私与运行状态：只读 kv 全收拢，便于一眼排障
    let status_rows: Vec<Element<'a, Message>> = vec![
        kv_row(tr("图片上传").to_string(), tr("从不 · 全程本地处理")),
        kv_row(tr("数据存储").to_string(), tr("仅本机配置目录的 config.json")),
        kv_row(tr("后端地址").to_string(), engine_txt.clone()),
        kv_row(tr("会话统计").to_string(), trf("{} 张", &[&app.config.stats.count.to_string()])),
        kv_row(tr("撤销栈深度").to_string(), format!("{} / 3", app.work.history.depth())),
        kv_row(
            tr("中间缓存").to_string(),
            if app.id.fg_preview.is_some() || app.id.prefetch.is_some() {
                tr("持有预览副本 / 预取槽（随使用自动回收）")
            } else {
                tr("空闲")
            },
        ),
    ];

    // 卡② 数据维护：破坏性操作保持独立按钮与明显分隔，不降为行内小控件
    let maintain_rows: Vec<Element<'a, Message>> = vec![container(
        row![
            button(text(tr("释放内存缓存")).size(FS_SM))
                .on_press(Message::Settings(SettingsMessage::ClearCache))
                .padding(pad(6.0, 12.0, 6.0, 12.0))
                .style(btn_secondary),
            button(text(tr("清空最近文件")).size(FS_SM))
                .on_press(Message::Settings(SettingsMessage::ClearRecent))
                .padding(pad(6.0, 12.0, 6.0, 12.0))
                .style(btn_secondary),
        ]
        .spacing(9),
    )
    .into()];

    let col = column![
        section_card(tr("隐私与运行状态"), None, status_rows),
        section_card(tr("数据维护"), None, maintain_rows),
    ]
    .spacing(10);

    panel_wrap(tr("隐私与数据").to_string(), col.into())
}

fn changelog_panel<'a>() -> Element<'a, Message> {
    let entries: &[(&str, &str, &str, &[&str])] = &[
        (
            "0.4.4",
            "2026-11-04",
            tr("新功能"),
            &[
                tr("新手引导升级为聚焦式（Spotlight）：遮罩挖洞高亮当前步骤的目标控件，说明气泡锚定在控件旁"),
                tr("引导支持点击穿透：点遮罩空白处翻到下一步，点洞内目标（如「开始」卡）直接触发该操作，边做边学"),
            ],
        ),
        (
            "0.4.3",
            "2026-11-03",
            tr("改进"),
            &[
                tr("更新包下载支持多源镜像回退：GitHub 主源失败自动切换境内可达镜像，下载不再受单源波动影响"),
                tr("安装包下载全程 SHA-256 强校验，内容不符自动换源重下，杜绝损坏或被篡改的安装包落地"),
            ],
        ),
        (
            "0.4.2",
            "2026-11-03",
            tr("修复"),
            &[
                tr("修复证件照批量抠图异常后推理闸门被永久占用的问题（改为 RAII 守卫自动归还）"),
                tr("安装包补齐 onnxruntime_providers_shared.dll（DirectML 执行提供程序依赖）"),
                tr("新增神经网络诊断日志文件，报障时可提供日志直接定位"),
                tr("打赏浮窗：移除内置收款码，改为爱发电 / Ko-fi 渠道直达，分组排版优化"),
            ],
        ),
        (
            "0.4.1",
            "2026-11-03",
            tr("修复"),
            &[
                tr("修复 InSPyReNet 大图抠图时并发推理叠加导致的内存溢出闪退（新增全局推理闸门）"),
                tr("修复 u2net / IS-Net 族模型输出未归一化导致的蒙版全白或全黑（min-max 归一化）"),
                tr("证件照批量抠图与工具页自动重抠不再互相抢占推理资源"),
            ],
        ),
        (
            "0.4.0",
            "2026-10-31",
            tr("新功能"),
            &[
                tr("主题系统：亮色 / 深色 / 跟随系统，支持自定义强调色并持久化"),
                tr("多语言国际化：中文 / English 一键切换，时区感知的本地化日期"),
                tr("通知系统：系统通知 + 应用内消息中心（未读徽标、免打扰时段）"),
                tr("批量队列增强：失败容忍、输出目录选择、并行流水线、失败项重试"),
                tr("证件照：恢复渲染模式与输出 DPI 控件，导航与全界面英文翻译补齐"),
            ],
        ),
        (
            "0.3.2",
            "2026-10-12",
            tr("重构"),
            &[
                tr("iced 原生复刻版：移除 Tauri / WebView / 前端框架，UI 改为 iced 0.14（Elm 架构 + wgpu）"),
                tr("抠图内核 SmartMatte v2 用 Rust 重写，算法与 JS 版逐行对齐"),
                tr("配置持久化改为系统配置目录下的 config.json"),
            ],
        ),
        (
            "0.3.0",
            "2026-10-11",
            tr("新功能"),
            &[
                tr("首页重做为「证件照工作台」落地页：7 种规格快速预设、当前状态卡、照片入口"),
                tr("裁剪工具新增旋转校正与自由⇄锁定宽高比"),
                tr("旋转 / 裁剪接入撤销栈（深度 3，换图清空）"),
            ],
        ),
        (
            "0.1.0",
            "2026-10-10",
            tr("新功能"),
            &[
                tr("新增「证件照」页面：上传 → 抠图换底 → 规格构图出图 → 相纸排版导出"),
                tr("新增 20 种证件规格与 7 种标准底色"),
            ],
        ),
        (
            "0.0.16",
            "2026-10-07",
            tr("重构"),
            &[tr("单文件 index.html（3342 行）拆为模块化工程"), tr("新增 Tauri 2 桌面端与 platform 适配层")],
        ),
    ];

    // 单张大卡片：每个版本是时间线上的一个块（版本行 + 条目列表），
    // 1px 分隔线只落在版本块之间，条目内部不再有独立卡片。
    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    for (v, d, tag, items) in entries {
        let mut inner = column![
            row![
                text(format!("v{v}")).size(FS).color(TXT()),
                text(*d).size(FS_XS).color(TXT3()),
                badge(*tag, BadgeKind::Dim),
            ]
            .spacing(9)
            .align_y(Alignment::Center),
        ]
        .spacing(5);
        for it in *items {
            inner = inner.push(text(format!("· {it}")).size(FS_SM).color(TXT2()));
        }
        rows.push(container(inner).width(Length::Fill).padding(pad(4.0, 0.0, 4.0, 0.0)).into());
    }
    panel_wrap(tr("更新日志").to_string(), section_card(tr("更新日志"), None, rows))
}


/* ═════════════════ 检查更新面板（自关于页迁入，逻辑不变） ═════════════════ */

/// 「检查更新」卡：当前版本 + 检查按钮 + 状态反馈 + 自动检查开关 +
/// 「忽略版本」恢复入口。发现新版本时点状态行重新打开对话框。
fn update_card(app: &App) -> Element<'_, Message> {
    let u = &app.updater;

    // 状态行：随 UpdateStatus 摘要变化
    let (status_text, status_color): (String, Color) = match &u.status {
        UpdateStatus::Idle => (tr("从未检查").to_string(), TXT3()),
        UpdateStatus::Checking => (tr("正在检查…").to_string(), TXT2()),
        UpdateStatus::UpToDate => (tr("已是最新版本").to_string(), OK()),
        UpdateStatus::Available(info) => {
            (trf("新版本 v{} 可用，点击查看", &[&info.version]), ACC())
        }
        UpdateStatus::Downloading { received, total, .. } => match total {
            Some(t) => (
                trf("下载中 {}%", &[&format!("{:.0}", *received as f32 / (*t).max(1) as f32 * 100.0)]),
                TXT2(),
            ),
            None => (trf("下载中 {}", &[human_size(*received).as_str()]), TXT2()),
        },
        UpdateStatus::Ready { .. } => (tr("安装包已就绪").to_string(), OK()),
        UpdateStatus::Failed { .. } => (tr("检查/下载失败，点击重试").to_string(), DANGER()),
    };

    // 状态摘要可点：发现新版本 → 打开对话框；失败 → 重试；其余 → 打开对话框兜底
    let open_or_retry = Message::Updater(match &u.status {
        UpdateStatus::Failed { info: Some(_), .. } => UpdaterMessage::StartInstall,
        UpdateStatus::Failed { .. } => UpdaterMessage::Check,
        _ => UpdaterMessage::ShowDialog,
    });

    // 「检查更新」按钮：检查中禁用（无 on_press + 置灰样式）
    let check_btn: Element<'_, Message> = if matches!(u.status, UpdateStatus::Checking) {
        button(text(tr("检查更新")).size(FS).color(TXT3()))
            .padding(pad(6.0, 14.0, 6.0, 14.0))
            .style(btn_secondary)
            .into()
    } else {
        button(text(tr("检查更新")).size(FS))
            .padding(pad(6.0, 14.0, 6.0, 14.0))
            .style(btn_primary)
            .on_press(Message::Updater(UpdaterMessage::Check))
            .into()
    };

    let ver_row = row![
        text(tr("当前版本")).size(FS).color(TXT2()),
        text(format!("v{}", updater::current_version_text()))
            .size(FS)
            .color(TXT())
            .font(bold_font()),
        Space::new().width(10.0),
        button(text(status_text).size(FS_SM).color(status_color))
            .padding(pad(4.0, 10.0, 4.0, 10.0))
            .style(rail_btn(false))
            .on_press(open_or_retry),
        Space::new().width(Length::Fill),
        check_btn,
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let auto_row = row![
        column![
            text(tr("启动时自动检查更新")).size(FS).color(TXT()),
            text(tr("每 24 小时最多检查一次；忽略的版本不再自动提醒"))
                .size(FS_XS)
                .color(TXT3()),
        ]
        .spacing(2),
        Space::new().width(Length::Fill),
        switch_btn(
            app.config.updates.auto_check,
            Message::Updater(UpdaterMessage::ToggleAutoCheck(!app.config.updates.auto_check)),
        ),
    ]
    .align_y(Alignment::Center);

    // 忽略版本恢复入口（仅当存在被忽略版本时显示）
    let skipped_row: Element<'_, Message> =
        if let Some(v) = app.config.updates.skipped_version.clone() {
            row![
                text(trf("已忽略 v{}", &[&v])).size(FS_SM).color(TXT2()),
                Space::new().width(Length::Fill),
                button(text(tr("恢复提醒")).size(FS_SM))
                    .padding(pad(4.0, 10.0, 4.0, 10.0))
                    .style(btn_ghost)
                    .on_press(Message::Updater(UpdaterMessage::ClearSkipped)),
            ]
            .align_y(Alignment::Center)
            .into()
        } else {
            Space::new().height(0.0).into()
        };

    container(column![ver_row, auto_row, skipped_row].spacing(14))
        .padding(pad_all(CARD_PAD))
        .width(Length::Fill)
        .style(card)
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

/// 「更新」面板：承载检查更新卡（自关于页迁入）
fn update_panel<'a>(app: &'a App) -> Element<'a, Message> {
    panel_wrap(tr("检查更新").to_string(), update_card(app))
}

/* ═════════════════ 外观与通知面板 ═════════════════ */

/// 强调色下拉选项（hex=None 表示恢复默认蓝）
#[derive(Debug, Clone, PartialEq, Eq)]
struct AccentChoice {
    label: &'static str,
    hex: Option<&'static str>,
}

impl std::fmt::Display for AccentChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(tr(self.label))
    }
}

const ACCENT_CHOICES: &[AccentChoice] = &[
    AccentChoice { label: "默认蓝", hex: None },
    AccentChoice { label: "青碧", hex: Some("#0E9F8A") },
    AccentChoice { label: "紫罗兰", hex: Some("#7C5CE6") },
    AccentChoice { label: "莓红", hex: Some("#D6467C") },
    AccentChoice { label: "琥珀", hex: Some("#D97706") },
    AccentChoice { label: "松石绿", hex: Some("#12A594") },
    AccentChoice { label: "石墨", hex: Some("#3F4650") },
    AccentChoice { label: "绯红", hex: Some("#DC2626") },
];

/// 「外观与通知」面板：主题模式 / 强调色 / 界面语言 / 免打扰时段
fn appearance_panel<'a>(app: &'a App) -> Element<'a, Message> {
    use crate::core::i18n::{self, tr, Lang};
    use crate::theme::ThemeMode;

    // i18n 文案绑定到局部变量（面板 'a 生命周期要求）
    let title_theme = tr("appearance.theme").to_string();
    let title_accent = tr("appearance.accent").to_string();
    let title_lang = tr("appearance.lang").to_string();
    let title_panel = tr("panel.appearance").to_string();

    // 主题模式
    let mode_pick = pick_list(
        ThemeMode::ALL,
        Some(app.theme_mode),
        Message::SetThemeMode,
    )
    .width(Length::Fixed(180.0))
    .text_size(FS_SM)
    .style(pick_list_style)
    .menu_style(menu_style);

    // 强调色：标准下拉（与其它 pick_list 同款样式链），「恢复默认」并入选项
    let accent_selected = ACCENT_CHOICES
        .iter()
        .find(|c| {
            // 旧配置可能把默认蓝存成显式 hex，归一化到 None
            match c.hex {
                None => matches!(
                    app.config.appearance.accent_hex.as_deref(),
                    None | Some("#4176E6") | Some("#4176e6")
                ),
                Some(h) => app.config.appearance.accent_hex.as_deref() == Some(h),
            }
        })
        .cloned();
    let accent_pick = pick_list(
        ACCENT_CHOICES,
        accent_selected,
        |c: AccentChoice| Message::SetAccent(c.hex.map(|h| h.to_string())),
    )
    .width(Length::Fixed(180.0))
    .text_size(FS_SM)
    .style(pick_list_style)
    .menu_style(menu_style);

    // 语言
    let lang_pick = pick_list(Lang::ALL, Some(i18n::lang()), Message::SetLang,)
    .width(Length::Fixed(180.0))
    .text_size(FS_SM)
    .style(pick_list_style)
    .menu_style(menu_style);

    // 免打扰
    let dnd = &app.config.dnd;
    let dnd_row = row![
        column![
            text(tr("appearance.dnd")).size(FS).color(TXT()),
            text(tr("appearance.dnd.hint")).size(FS_XS).color(TXT3()),
        ]
        .spacing(2),
        Space::new().width(Length::Fill),
        switch_btn(
            dnd.enabled,
            Message::ToggleDnd,
        ),
    ]
    .align_y(Alignment::Center);

    let times_row = row![
        text(tr("appearance.dnd.start")).size(FS_SM).color(TXT2()),
        text_input("", &dnd.start)
            .on_input(|s| Message::SetDndTimes(s, dnd.end.clone()))
            .width(Length::Fixed(72.0))
            .size(FS_SM)
            .style(text_input_style),
        text("~").size(FS_SM).color(TXT3()),
        text(tr("appearance.dnd.end")).size(FS_SM).color(TXT2()),
        text_input("", &dnd.end)
            .on_input(|e| Message::SetDndTimes(dnd.start.clone(), e))
            .width(Length::Fixed(72.0))
            .size(FS_SM)
            .style(text_input_style),
        Space::new().width(Length::Fill),
        text("HH:MM 24h").size(FS_XS).color(TXT3()),
    ]
    .spacing(10)
    .align_y(Alignment::Center);

    // 卡① 界面外观：标签与下拉编为一组，分隔线只落在组与组之间
    let look_rows: Vec<Element<'a, Message>> = vec![
        container(column![kv_row(title_theme, ""), container(mode_pick).padding(pad(4.0, 0.0, 10.0, 0.0))]).into(),
        container(column![kv_row(title_accent, ""), container(accent_pick).padding(pad(4.0, 0.0, 10.0, 0.0))]).into(),
        container(column![kv_row(title_lang, ""), container(lang_pick).padding(pad(4.0, 0.0, 10.0, 0.0))]).into(),
    ];

    // 卡② 通知与免打扰：开关行 + 时段行编为一组
    let dnd_rows: Vec<Element<'a, Message>> = vec![container(
        column![dnd_row, times_row].spacing(8),
    )
    .into()];

    let col = column![
        section_card(tr("界面外观"), None, look_rows),
        section_card(tr("通知与免打扰"), None, dnd_rows),
    ]
    .spacing(10);

    panel_wrap(title_panel, col.into())
}