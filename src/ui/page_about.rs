//! 关于页：品牌 + Markdown 内容渲染 + 检查更新
//!
//! 内容由 `content/about/*.md` 驱动（slogan / tools / intro / cards），
//! 与原项目 `public/content/about/` 同约定：
//! - 优先读 exe 同目录 / 工作目录下的 `content/about/`（改完切页即生效，免重启）
//! - 找不到时回退到编译期内嵌默认文案
//! - `{{abThreads}}`、`{{abEngine}}` 占位符由程序回填
//! - 渲染用 iced 内置 `markdown` 组件（pulldown_cmark 子集，链接点击经消息打开）

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::{Alignment, Element, Length};

use crate::app::App;
use crate::message::Message;
use crate::theme::*;
use crate::core::i18n::{tr, trf};

const EMBED_SLOGAN: &str = include_str!("../../content/about/slogan.md");
const EMBED_TOOLS: &str = include_str!("../../content/about/tools.md");
const EMBED_INTRO: &str = include_str!("../../content/about/intro.md");
const EMBED_CARDS: &str = include_str!("../../content/about/cards.md");
const EMBED_ACK: &str = include_str!("../../content/about/ack.md");

/// 读取单个内容文件：exe 同目录 → 工作目录 → 内嵌默认
fn read_content_md(name: &str) -> String {
    let rel = std::path::Path::new("content/about").join(name);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Ok(s) = std::fs::read_to_string(dir.join(&rel)) {
                return s;
            }
        }
    }
    if let Ok(s) = std::fs::read_to_string(&rel) {
        return s;
    }
    match name {
        "slogan.md" => EMBED_SLOGAN.to_string(),
        "tools.md" => EMBED_TOOLS.to_string(),
        "intro.md" => EMBED_INTRO.to_string(),
        "cards.md" => EMBED_CARDS.to_string(),
        "ack.md" => EMBED_ACK.to_string(),
        _ => String::new(),
    }
}

/// 载入并解析关于页 Markdown（解析 + 占位符回填），存入 App 供 view 使用
pub fn load_markdown(app: &mut App) {
    use iced::widget::markdown;

    let threads = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let engine = if app.config.model.starts_with("nn:") {
        trf("神经网络 · {}", &[app.config.model.trim_start_matches("nn:")])
    } else {
        tr("SmartMatte v2 色键").to_string()
    };
    let fill = |s: String| {
        s.replace("{{abThreads}}", &threads)
            .replace("{{abEngine}}", &engine)
    };

    // slogan：非「作者：」行 = 大字；「作者：xxx」行 = 署名小字
    let slogan_src = fill(read_content_md("slogan.md"));
    let mut slogan_big = String::from(tr("把重复的事交给机器，把判断留给人。"));
    let mut author: Option<String> = None;
    for line in slogan_src.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(a) = t.strip_prefix(tr("作者：")) {
            author = Some(a.trim().to_string());
        } else {
            slogan_big = t.to_string();
        }
    }

    // 正文：tools → intro → cards → ack 依序拼成一篇 Markdown；空文件给兜底提示
    let mut doc = String::new();
    let sections: [(&str, Option<&str>); 4] = [
        ("tools.md", None),
        ("intro.md", Some(tr("核心能力"))),
        ("cards.md", None),
        ("ack.md", None),
    ];
    let bodies: Vec<String> = sections
        .iter()
        .map(|(name, _)| fill(read_content_md(name)).trim().to_string())
        .collect();
    // 分隔线插在每个非空 section 之后，最后一个非空 section 不加
    let last_non_empty = bodies.iter().rposition(|b| !b.is_empty());
    for (i, ((_, heading), body)) in sections.iter().zip(bodies.iter()).enumerate() {
        doc.push_str(body);
        doc.push_str("\n\n");
        if let Some(h) = heading {
            if body.is_empty() {
                doc.push_str(&format!("> {h}{}\n\n", tr("内容暂缺")));
            }
        }
        if !body.is_empty() && Some(i) != last_non_empty {
            doc.push_str("---\n\n");
        }
    }

    let items: Vec<markdown::Item> = markdown::parse(&doc).collect();
    app.about_md = items;
    app.about_slogan = slogan_big;
    app.about_author = author;
}

pub fn view(app: &App) -> Element<'_, Message> {
    // ── 大字标语（slogan.md 驱动）：无卡片装饰的纯文字展示区 ──
    let hero: Element<'_, Message> = column![
        text(&app.about_slogan)
            .size(30.0)
            .color(TXT())
            .font(bold_font()),
        match &app.about_author {
            Some(a) => {
                let el: Element<'_, Message> =
                    text(trf("作者：{}", &[a])).size(FS_SM).color(TXT2()).into();
                el
            }
            None => Space::new().height(0.0).into(),
        },
    ]
    .spacing(10)
    .align_x(Alignment::Center)
    .width(Length::Fill)
    .into();

    // ── Markdown 正文（tools / intro / cards，官方 markdown 组件渲染）──
    // 字号对齐应用字号体系（想调整改这里）：
    // 正文 = FS(13)，H1 = 22 / H2 = FS_H1(18) / H3 = FS_H(14)，其余标题随 H3
    let md_theme = match crate::theme::resolved() {
        crate::theme::Resolved::Light => iced::Theme::Light,
        crate::theme::Resolved::Dark => iced::Theme::Dark,
    };
    let mut md_settings = iced::widget::markdown::Settings::with_text_size(FS, md_theme);
    md_settings.h1_size = 22.0.into();
    md_settings.h2_size = FS_H1.into();
    md_settings.h3_size = FS_H.into();
    md_settings.h4_size = FS_H.into();
    md_settings.h5_size = FS.into();
    md_settings.h6_size = FS.into();
    // 正文统一主题灰（TXT2()）：markdown 的标题/正文 span 继承外层 text_color，
    // 链接（强调色）与行内代码（白字黑底）有自己的显式色不受影响
    let md_view = container(
        iced::widget::markdown::view(&app.about_md, md_settings)
            .map(Message::OpenUrl),
    )
    .width(Length::Fill)
    .style(|_t| container::Style {
        text_color: Some(TXT2()),
        ..container::Style::default()
    });

    let body = column![
        hero,
        container(
            column![
                row![
                    text(tr("壹鸣抠图 · 使用说明")).size(FS_H).color(TXT()),
                    Space::new().width(Length::Fill),
                    button(text(tr("重新加载内容")).size(FS_XS).color(TXT2()))
                        .padding(pad(4.0, 10.0, 4.0, 10.0))
                        .style(btn_ghost)
                        .on_press(Message::AboutReload),
                ]
                .align_y(Alignment::Center),
                md_view,
            ]
            .spacing(14),
        )
        .padding(pad_all(CARD_PAD + SP_2))
        .width(Length::Fill)
        .style(card),
        Space::new().height(12.0),
    ]
    .spacing(22);

    container(
        scrollable(
            container(body)
                .padding(pad(28.0, 26.0, 40.0, 26.0))
                .center_x(Length::Fill),
        )
        .direction(Direction::Vertical(Scrollbar::new().width(3).scroller_width(3)))
        .style(scrollbar_style),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(app_bg)
    .into()
}