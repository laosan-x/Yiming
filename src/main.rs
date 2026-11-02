//! 壹鸣 · 商品抠图工作台 —— iced 原生版入口
//!
//! 纯 Rust：iced（Elm 架构 + wgpu 渲染），没有 WebView / Electron / 任何前端框架。
//!
//! 用法：`cocole-iced [图片路径]`（带路径则启动后直接载入该图并自动抠图）

// 发布版不弹控制台窗口；debug 版保留 stdout，方便看 eprintln 诊断
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod core;
mod message;
mod theme;
mod ui;
pub mod version;

use app::App;

/// 中文显示：iced 内置字体不含 CJK 字形，必须显式加载系统中文字体，
/// 否则界面上的汉字会变成缺字符方块。按候选顺序挑第一个存在的字体文件。
///
/// 原项目 CSS 的标题普遍是 `font-weight:550~750`。iced 不能合成字重，
/// 只能命中系统里真实存在的 Bold 字面，因此这里额外找一个**同族**的
/// Bold 文件：Windows 的 `msyhbd.ttc` 与 `msyh.ttc` 同属 "Microsoft YaHei"
/// 族、只差 weight，注册后 `Family::Name("Microsoft YaHei") + Weight::Bold`
/// 就能命中。找不到时返回 `None`，界面退回常规字重（只损失层级感）。
struct FontSet {
    regular: Option<(Vec<u8>, &'static str)>,
    bold: Option<Vec<u8>>,
}

fn load_cjk_fonts() -> FontSet {
    let candidates: &[(&str, &str, &[&str])] = &[
        (
            "Microsoft YaHei",
            r"C:\Windows\Fonts\msyh.ttc",
            &[r"C:\Windows\Fonts\msyhbd.ttc", r"C:\Windows\Fonts\msyhbd.ttf"],
        ),
        (
            "SimHei",
            r"C:\Windows\Fonts\simhei.ttf",
            &[],
        ),
        (
            "DengXian",
            r"C:\Windows\Fonts\Deng.ttf",
            &[r"C:\Windows\Fonts\Dengb.ttf"],
        ),
        (
            "Noto Sans CJK SC",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            &["/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc"],
        ),
        (
            "PingFang SC",
            "/System/Library/Fonts/PingFang.ttc",
            &["/System/Library/Fonts/PingFang.ttc"],
        ),
    ];

    for (name, path, bold_paths) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            if !bytes.is_empty() {
                let bold = bold_paths.iter().find_map(|p| std::fs::read(p).ok());
                return FontSet { regular: Some((bytes, name)), bold };
            }
        }
    }
    eprintln!("[font] 未找到中文字体，界面中文可能显示为方块");
    FontSet { regular: None, bold: None }
}

fn main() -> iced::Result {
    // 开发用诊断子命令：导出蒙版裸字节，便于与原版 JS 内核逐字节比对
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--dump-mask") {
        std::process::exit(dump_mask(&args[2..]));
    }

    // 强制 wgpu 使用 DX12 后端，避免与 ONNX Runtime 的 DirectML 冲突
    // （两者都用 DirectML 会导致 GPU 资源竞争，纹理创建失败）
    std::env::set_var("WGPU_BACKEND", "dx12");

    let t0 = std::time::Instant::now();
    let mut app = iced::application(App::boot, app::update, app::view)
        .title(App::title)
        .window(app::window_settings())
        .antialiasing(true)
        .subscription(app::subscription);

    let t_font = std::time::Instant::now();
    let fonts = load_cjk_fonts();
    eprintln!("[boot] 字体加载 {:?}", t_font.elapsed());
    if let Some((bytes, name)) = fonts.regular {
        app = app.font(bytes).default_font(iced::Font::with_name(name));
        theme::set_bold_family(fonts.bold.is_some().then_some(name));
    }
    if let Some(bold) = fonts.bold {
        app = app.font(bold);
    }

    eprintln!("[boot] 前置阶段合计 {:?}", t0.elapsed());
    app.run()
}

/// `cocole-iced --dump-mask <in.rgba> <宽> <高> <out.mask> [params.json]`
///
/// 输入是裸 RGBA（`PIL` 的 `Image.convert("RGBA").tobytes()` 即可生成），
/// 输出是 w*h 的 u8 蒙版。配合 `tools/js_dump.mjs` 可做「原版 JS ↔ Rust」逐字节回归。
fn dump_mask(args: &[String]) -> i32 {
    use core::matting::{compute_mask, FieldCache, MatteParams};

    if args.len() < 4 {
        eprintln!("用法: cocole-iced --dump-mask <in.rgba> <宽> <高> <out.mask> [params.json]");
        return 2;
    }
    let (w, h) = match (args[1].parse::<usize>(), args[2].parse::<usize>()) {
        (Ok(w), Ok(h)) if w > 0 && h > 0 => (w, h),
        _ => {
            eprintln!("宽/高无效");
            return 2;
        }
    };
    // 第 5 个参数既可以是 JSON 文件路径，也可以直接是内联 JSON 字符串（以 `{` 开头）
    let params: MatteParams = match args.get(4) {
        Some(raw) => {
            let text = if raw.trim_start().starts_with('{') {
                raw.clone()
            } else {
                match std::fs::read_to_string(raw) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("参数文件读取失败：{e}");
                        return 2;
                    }
                }
            };
            match serde_json::from_str(&text) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("参数无法解析：{e}\n输入：{text}");
                    return 2;
                }
            }
        }
        None => MatteParams::default(),
    };
    let data = match std::fs::read(&args[0]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("读取失败：{e}");
            return 1;
        }
    };
    let need = w * h * 4;
    if data.len() < need {
        eprintln!("裸数据长度不足：{} < {}", data.len(), need);
        return 1;
    }

    let mut cache = FieldCache::default();
    let res = compute_mask(&data[..need], w, h, &params, &mut cache);
    if let Err(e) = std::fs::write(&args[3], &res.mask) {
        eprintln!("写入失败：{e}");
        return 1;
    }
    // 走 serde_json 而不是手写 format!，否则 Windows 路径里的 `\` 会产出非法 JSON 转义
    let out = serde_json::json!({
        "bg": res.bg,
        "fgCount": res.fg_count,
        "ratio": res.fg_count as f64 / res.mask.len().max(1) as f64,
        "holes": res.holes_filled,
        "ms": res.elapsed_ms,
        "out": args[3],
    });
    println!("{out}");
    0
}