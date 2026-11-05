//! ONNX 本地推理引擎 —— 神经网络抠图（插件式模型加载）
//!
//! 移植自 `cocole-v0.3.2/src-tauri/src/infer.rs`（已在本机验证可跑通的组合：
//! ort 2.0.0-rc.13 + load-dynamic + 16MB onnxruntime.dll）。
//!
//! ## 插件加载机制
//!
//! - **模型目录解析顺序**：环境变量 `COCOLE_MODEL_DIR` → exe 同级 `models/`
//!   → exe 上两级 `models/`（开发模式 target/debug → 项目根）→
//!   编译期 `CARGO_MANIFEST_DIR/models/` → 当前目录 `models/`。
//! - **扫描规则**：只扫一层（不递归，避免捞进 backup/ 旧权重）、只认 `.onnx`
//!   后缀、跳过 `.tmp`/`~`/`.part` 残留文件。
//! - **输入边长约定**：内置三模型有规格表（BiRefNet=1024 / MODNet=512 /
//!   U²-NetP=320）；其余文件按命名约定 `<名>_<边长>.onnx` / `<名>@<边长>.onnx`
//!   解析（只认末尾 2~4 位数字、64~2048），缺省 512。
//!   —— 用户往目录里放任意导出好的 onnx 都能直接出现在下拉里，无需改代码。
//! - **会话缓存**：模型加载很贵（490MB 的 BiRefNet 约 10~15s），按模型 id
//!   缓存 `Session`，同一模型终身只加载一次；批量处理因此只付一次加载成本。
//! - **执行后端**：体积 >64MB 的模型尝试 DirectML（GPU），注册失败或被
//!   `COCOLE_EP=cpu` 明确禁止时自动回退 CPU；小模型直接 CPU（省掉 GPU 初始化
//!   开销，实测反而更快）。
//!
//! ## 输出格式
//!
//! 推理产出 [0,1] 连续 alpha（软蒙版），双线性放大回原图尺寸后以 0-255 u8
//! 返回；上层 `img::apply_mask` 把它写进 RGBA 的 A 通道，导出时
//! `save_to_memory` 编码为**带 alpha 的 PNG**（半透明/毛发边缘以渐变 alpha
//! 保留，这正是神经网络抠图比色键抠图自然的原因）。

use ort::session::Session;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::core::i18n::trf;

/* ═════════════════ 模型发现 ═════════════════ */

/// 一个可用模型的完整描述（下拉列表条目 + 推理请求的全部信息）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct NnModelInfo {
    /// 模型标识：内置 `nn:<文件名>`；与插件统一（文件名全局唯一性由扫描保证）
    pub id: String,
    /// 下拉显示名，如 `MODNet（512² · 25MB）`
    pub label: String,
    /// 中文展示名，如 `BiRefNet（标准版）`
    pub display_name: String,
    /// 去扩展名的文件主干（展示用）
    pub name: String,
    /// 文件名（不含目录）
    pub file: String,
    /// 绝对路径
    pub path: String,
    /// 模型文件体积（MB，一位小数）
    pub size_mb: f64,
    /// 输入边长（正方形输入）
    pub input_size: usize,
    /// 模型适用类别
    pub category: ModelCategory,
}

/// 模型适用类别：按图像处理任务类型划分，用于设置页分组展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum ModelCategory {
    /// 人像抠图/分割：MODNet、BiRefNet 等，发丝级边缘
    Portrait,
    /// 通用显著性分割：U²-NetP 等，轻量通用
    Salient,
    /// 人脸检测/定位：SCRFD、YuNet 等，不参与抠图
    FaceDetect,
    /// 其他/未分类：用户自行放入的第三方模型
    Other,
}

impl ModelCategory {
    pub fn label(&self) -> &'static str {
        match self {
            ModelCategory::Portrait => "人像抠图",
            ModelCategory::Salient => "通用分割",
            ModelCategory::FaceDetect => "人脸检测",
            ModelCategory::Other => "其他模型",
        }
    }

    pub fn desc(&self) -> &'static str {
        match self {
            ModelCategory::Portrait => "人像发丝级边缘分割，适合证件照、人物抠图",
            ModelCategory::Salient => "通用显著性目标检测，适合商品、物体抠图",
            ModelCategory::FaceDetect => "人脸定位与关键点检测，不参与抠图推理",
            ModelCategory::Other => "用户自行放入的第三方 ONNX 模型",
        }
    }
}

/// 根据模型文件名推断适用类别。
pub fn model_category(file: &str) -> ModelCategory {
    let lower = file.to_ascii_lowercase();
    if lower.contains("face") || lower.contains("yunet") || lower.contains("scrfd") || lower.contains("retinaface") {
        ModelCategory::FaceDetect
    } else if lower.contains("modnet") {
        ModelCategory::Portrait
    } else if lower.contains("birefnet") || lower.contains("ben2") || lower.contains("inspyrenet") || lower.contains("model_fp") || lower.contains("u2net") || lower.contains("salient") || lower.contains("isnet") || lower.contains("rmbg") {
        ModelCategory::Salient
    } else {
        ModelCategory::Other
    }
}

/// 模型中文展示名映射：文件名 → 中文展示名。
/// 新增模型时只需在此处添加映射即可。
pub fn model_display_name(file: &str) -> String {
    let lower = file.to_ascii_lowercase();
    match () {
        _ if lower.contains("birefnet") && lower.contains("general") => "【通用-生活场景】_BiRefNet-general".into(),
        _ if lower.contains("birefnet") => "【通用-生活场景】_BiRefNet-general".into(),
        _ if lower.contains("ben2") => "【通用-电商产品】_BEN2-Base".into(),
        _ if lower.contains("inspyrenet") || lower.contains("model_fp") => "【通用-人像毛发】_InSPyReNet".into(),
        _ if lower.contains("modnet") => "【人像-证件照】_MODNet".into(),
        _ if lower.contains("u2netp") => "【通用-图形文言】_u2netp".into(),
        _ if lower.contains("face") || lower.contains("yunet") || lower.contains("scrfd") => "【工具】人脸检测".into(),
        _ => {
            let stem = file.trim_end_matches(".onnx");
            let display = stem
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches(['_', '@']);
            if display.is_empty() { stem.to_string() } else { display.to_string() }
        }
    }
}

/// 内置模型规格表：目录里的固定文件名 →（展示名，输入边长）。
/// 这些文件名来自官方发布版，权重的输入尺寸是导出时确定的。
fn builtin_spec(file: &str) -> Option<(&'static str, usize)> {
    match file {
        "birefnet_general.onnx" => Some(("BiRefNet-general", 1024)),
        "ben2_base.onnx" => Some(("BEN2-Base", 1024)),
        "model_fp16.onnx" => Some(("InSPyReNet", 1024)),
        "inspyrenet.onnx" => Some(("InSPyReNet", 512)),
        "modnet.onnx" => Some(("MODNet", 512)),
        "u2netp.onnx" => Some(("u2netp", 320)),
        _ => None,
    }
}

/// 从文件名主干解析输入边长：`mymatte_1024` / `mymatte@1024` → 1024。
/// 只认末尾一段 2~4 位纯数字（64~2048），避免把 `v2`、`final3` 这类版本号误当边长。
fn parse_input_size_from_stem(stem: &str) -> Option<usize> {
    let tail = stem.rsplit(['_', '@']).next()?;
    if tail.len() < 2 || tail.len() > 4 || !tail.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: usize = tail.parse().ok()?;
    (64..=2048).contains(&n).then_some(n)
}

/// 从 ONNX 模型元数据中读取输入张量的空间维度（H/W）。
///
/// 抠图模型通常输入为 [1,3,H,W] 或 [1,C,H,W]，取最后两维。
/// 如果维度是动态的（-1），则返回 None。
fn read_input_size_from_onnx(path: &PathBuf) -> Option<usize> {
    // 确保 DLL 路径已设置（扫描阶段可能还没调用过 get_session）
    ensure_dylib_path();
    // 注意：ort 2.x 没有「只读元数据」的轻量 API，commit_from_file 会**完整
    // 加载权重**（490MB 模型可达 10s+）。因此调用它的 scan() 必须跑在后台
    // 线程（见 WorkMessage::RescanModels），不能在 UI 线程同步调用。
    let mut builder = Session::builder().ok()?;
    let session = builder.commit_from_file(path).ok()?;
    let inputs = session.inputs();
    let first = inputs.first()?;
    // ort 2.x: Outlet.dtype() 返回 &ValueType，ValueType::Tensor 有 shape 字段
    let shape = match first.dtype() {
        ort::value::ValueType::Tensor { shape, .. } => shape,
        _ => return None,
    };
    // 期望 [1,3,H,W] 或 [1,C,H,W]，取最后两维
    if shape.len() >= 4 {
        let h = shape[shape.len() - 2];
        let w = shape[shape.len() - 1];
        if h > 0 && h == w && (64..=2048).contains(&h) {
            return Some(h as usize);
        }
    }
    None
}

/// 解析模型目录（多级回退，见模块注释）。返回 (目录, 是否存在)。
pub fn models_dir() -> (PathBuf, bool) {
    // 1. 环境变量显式指定（最高优先级）
    if let Some(v) = std::env::var_os("COCOLE_MODEL_DIR") {
        let p = PathBuf::from(v);
        if !p.as_os_str().is_empty() {
            return (p.clone(), p.is_dir());
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("models"));
            // 开发模式：target/debug → 项目根
            candidates.push(dir.join("..").join("..").join("models"));
        }
    }
    // 编译期项目根（开发兜底；release 后路径仍在但通常不存在，无害）
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models"));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("models"));
    }
    for c in candidates {
        if c.is_dir() {
            return (c, true);
        }
    }
    // 都不存在：回退到 exe 同级（错误提示里给出这个路径最直观）
    let fallback = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("models")))
        .unwrap_or_else(|| PathBuf::from("models"));
    (fallback, false)
}

/// 扫描单个目录中的 .onnx 模型文件，返回模型清单。
///
/// 任何一步失败都不 panic：目录缺失返回空清单，
/// 单个文件 metadata 失败只跳过该文件。
fn scan_dir(dir: &PathBuf) -> Vec<NnModelInfo> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            nn_log(&format!("无法读取目录 {}：{e}", dir.display()));
            return Vec::new();
        }
    };
    let mut found: Vec<NnModelInfo> = Vec::new();

    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = file.to_ascii_lowercase();
        if !lower.ends_with(".onnx") || file.starts_with('.') {
            continue; // 格式不支持：非 .onnx 一律不进清单
        }
        let stem = file.strip_suffix(".onnx").unwrap_or(file);
        // 忽略明显的临时/残留文件（.onnx.tmp / 编辑器备份 ~ / 下载中断 .part）
        if stem.ends_with(".tmp") || stem.ends_with('~') || stem.ends_with(".part") {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue, // 拿不到大小（文件被占用等）就跳过，不中断扫描
        };
        let size_mb = meta.len() as f64 / 1048576.0;

        let (name, input_size) = match builtin_spec(&lower) {
            Some((name, size)) => (name.to_string(), size),
            None => {
                // 先尝试从文件名解析
                let size_from_name = parse_input_size_from_stem(stem);
                // 再从 ONNX 模型元数据中读取真实输入维度（最可靠）
                let size_from_model = read_input_size_from_onnx(&path);
                // 优先级：模型元数据 > 文件名 > 默认 512
                let size = size_from_model.or(size_from_name).unwrap_or(512);
                let display = stem
                    .trim_end_matches(|c: char| c.is_ascii_digit())
                    .trim_end_matches(['_', '@']);
                let name = if display.is_empty() { stem.to_string() } else { display.to_string() };
                (name, size)
            }
        };
        let category = model_category(file);
        let display_name = model_display_name(file);
        let label = display_name.clone();
        found.push(NnModelInfo {
            id: format!("nn:{file}"),
            label,
            display_name,
            name,
            file: file.to_string(),
            path: path.to_string_lossy().to_string(),
            size_mb: (size_mb * 10.0).round() / 10.0,
            input_size,
            category,
        });
    }

    found
}

/// 扫描模型目录 + 外置插件目录，生成可用模型清单（内置 + 插件）。
///
/// - `models_dir`：主模型目录（`models/`），启动时自动发现
/// - `plugin_dir`：外置插件目录（用户在设置中指定），可选
/// - 同名文件以主模型目录为准（去重）
/// - 任何一步失败都不 panic：目录缺失只跳过，不中断整体扫描
pub fn scan(plugin_dir: Option<&str>) -> (Option<PathBuf>, Vec<NnModelInfo>) {
    let (dir, exists) = models_dir();
    let mut found: Vec<NnModelInfo> = Vec::new();

    // 1. 扫描主模型目录
    if exists {
        found.extend(scan_dir(&dir));
    }

    // 2. 扫描外置插件目录（如果设置了且与主目录不同）
    if let Some(pdir_str) = plugin_dir {
        let pdir = PathBuf::from(pdir_str);
        if pdir.is_dir() && pdir.canonicalize().ok() != dir.canonicalize().ok() {
            let plugin_models = scan_dir(&pdir);
            // 按文件名去重：主目录已有的不再添加
            let existing_files: std::collections::HashSet<String> =
                found.iter().map(|m| m.file.clone()).collect();
            for m in plugin_models {
                if !existing_files.contains(&m.file) {
                    found.push(m);
                }
            }
        }
    }

    // 排序：大模型（精度优先）在前，同名按字母序
    found.sort_by(|a, b| b.size_mb.total_cmp(&a.size_mb).then(a.name.cmp(&b.name)));
    (if exists { Some(dir) } else { None }, found)
}

/* ═════════════════ 预处理 ═════════════════ */

/// 取出 RGBA 里的某个通道，双线性缩放到 S×S 并归一化到 [0,1]。
fn bilinear_resize(rgba: &[u8], src_w: usize, src_h: usize, size: usize, channel: usize) -> Vec<f32> {
    let mut out = vec![0f32; size * size];
    let sx = src_w as f32 / size as f32;
    let sy = src_h as f32 / size as f32;

    for y in 0..size {
        // 采样中心对齐半像素，避免整体偏移半个像素
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (src_h - 1) as f32);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(src_h - 1);
        let wy = fy - y0 as f32;

        for x in 0..size {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (src_w - 1) as f32);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(src_w - 1);
            let wx = fx - x0 as f32;

            let i00 = (y0 * src_w + x0) * 4 + channel;
            let i01 = (y0 * src_w + x1) * 4 + channel;
            let i10 = (y1 * src_w + x0) * 4 + channel;
            let i11 = (y1 * src_w + x1) * 4 + channel;

            let v = rgba[i00] as f32 * (1.0 - wx) * (1.0 - wy)
                + rgba[i01] as f32 * wx * (1.0 - wy)
                + rgba[i10] as f32 * (1.0 - wx) * wy
                + rgba[i11] as f32 * wx * wy;

            out[y * size + x] = v / 255.0;
        }
    }
    out
}

/// RGBA → [1,3,S,S] NCHW float32（RGB，归一化到 [0,1]）。
fn to_nchw(rgba: &[u8], w: usize, h: usize, size: usize) -> Vec<f32> {
    let plane = size * size;
    let mut buf = vec![0f32; 3 * plane];
    for c in 0..3 {
        let plane_data = bilinear_resize(rgba, w, h, size, c);
        buf[c * plane..(c + 1) * plane].copy_from_slice(&plane_data);
    }
    buf
}

/// alpha（模型输出分辨率 aw×ah）→ 原图尺寸的 0-255 软蒙版。
/// 用双线性而不是最近邻：alpha 是连续量，最近邻会在缩放时产生台阶状边缘。
///
/// 源尺寸从输出张量里取，不从模型规格推 —— 万一某个导出版本的输出分辨率
/// 与规格不符，按规格算索引就会越界或错位。
fn alpha_to_mask(alpha: &[f32], aw: usize, ah: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut mask = vec![0u8; dw * dh];
    if alpha.len() < aw * ah || aw == 0 || ah == 0 {
        return mask; // 数据不足时返回全 0，而不是 panic
    }
    let sx = aw as f32 / dw as f32;
    let sy = ah as f32 / dh as f32;

    // 双线性重采样是全分辨率 O(W×H) 的最大单段开销（20MP 单线程数百毫秒）。
    // 输出较大时按行分块多线程并行（std::thread::scope 零新依赖），小图直接串行。
    const PAR_MIN_PIXELS: usize = 1_000_000;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8);
    if dw * dh < PAR_MIN_PIXELS || threads <= 1 {
        for y in 0..dh {
            fill_mask_row(&mut mask[y * dw..], y, alpha, aw, ah, dw, sx, sy);
        }
        return mask;
    }
    let rows_per = dh.div_ceil(threads);
    std::thread::scope(|scope| {
        for (ci, chunk) in mask.chunks_mut(rows_per * dw).enumerate() {
            let start_y = ci * rows_per;
            scope.spawn(move || {
                for local in 0..chunk.len() / dw {
                    // 写入 chunk 的第 local 行 = 全局第 start_y + local 行
                    fill_mask_row(
                        &mut chunk[local * dw..],
                        start_y + local,
                        alpha,
                        aw,
                        ah,
                        dw,
                        sx,
                        sy,
                    );
                }
            });
        }
    });
    mask
}

/// 填充蒙版的第 `y` 行到 `out[0..dw]`（调用方负责传入正确的行切片与全局行号）。
/// 串行/并行两路共用的单行内核。
#[allow(clippy::too_many_arguments)]
    #[allow(clippy::needless_range_loop)] // 热循环按索引写入，迭代器改写不减可读性
fn fill_mask_row(
    out: &mut [u8],
    y: usize,
    alpha: &[f32],
    aw: usize,
    ah: usize,
    dw: usize,
    sx: f32,
    sy: f32,
) {
    let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (ah - 1) as f32);
    let y0 = fy.floor() as usize;
    let y1 = (y0 + 1).min(ah - 1);
    let wy = fy - y0 as f32;

    for x in 0..dw {
        let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (aw - 1) as f32);
        let x0 = fx.floor() as usize;
        let x1 = (x0 + 1).min(aw - 1);
        let wx = fx - x0 as f32;

        let v = alpha[y0 * aw + x0] * (1.0 - wx) * (1.0 - wy)
            + alpha[y0 * aw + x1] * wx * (1.0 - wy)
            + alpha[y1 * aw + x0] * (1.0 - wx) * wy
            + alpha[y1 * aw + x1] * wx * wy;

        out[x] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

/* ═════════════════ 会话管理 ═════════════════ */

/// nn 诊断日志：stderr + 文件镜像。
///
/// release 安装包没有控制台，`eprintln!` 全部不可见——排查「分发包内
/// 模型加载/推理失败」时拿不到任何现场。这里镜像写入
/// `%LOCALAPPDATA%\yiming\cocole-iced\logs\nn.log`（>1MB 自动截断），
/// 用户报障时让 TA 把这个文件发来即可定位。
pub fn nn_log(msg: &str) {
    eprintln!("[nn] {msg}");
    static LOG_FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::path::PathBuf>>> =
        std::sync::OnceLock::new();
    let path = LOG_FILE.get_or_init(|| {
        directories::ProjectDirs::from("cn", "yiming", "cocole-iced").map(|d| {
            let dir = d.data_local_dir().join("logs");
            let _ = std::fs::create_dir_all(&dir);
            std::sync::Mutex::new(dir.join("nn.log"))
        })
    });
    let Some(Ok(path)) = path.as_ref().map(|m| m.lock().map(|p| p.clone())) else {
        return;
    };
    // 简单滚动：超过 1MB 直接重写，避免无限增长
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 1_048_576 {
            let _ = std::fs::remove_file(&path);
        }
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{ts}] [nn] {msg}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write as _;
        let _ = f.write_all(line.as_bytes());
    }
}

/// 会话缓存：模型加载要 10~15 秒（490MB），同一模型不该反复加载。
///
/// 两级锁：`SESSIONS` 全局锁**只保护 HashMap 本身**（增删 / 更新时间戳，
/// 均为微秒级）；真正的 `session.run()` 在每会话独立的 `Mutex<Session>` 上
/// 跑（ort 2.x 的 run 需要 `&mut self`）。推理可达数十秒，若像旧实现那样
/// 在全局锁内跑，UI 线程的 `release` / `release_all` / 空闲清扫会被整段
/// 阻塞，表现为点一下按钮界面冻结几十秒。
///
/// `Arc` 还顺带解决并发释放：在途推理持有一份引用，`release_all` 从表里
/// 移除后会话不会立刻析构（等推理结束自然回收），不会 use-after-free。
struct SessionEntry {
    session: Arc<Mutex<Session>>,
    last_used: std::time::Instant,
}

/// 会话缓存被并发释放时的哨兵错误（[`run_once`] 据此就地懒重载一次）
const CACHE_LOST: &str = "会话缓存已丢失";
const CACHE_RELEASED: &str = "会话已被释放，请重新执行抠图";

/// 闲置超过此秒数的会话自动释放（下次推理重新加载）。
/// DirectML 的 arena 在推理结束后不归还系统，大模型会话常驻会占满显存；
/// 主动清扫（推理完成 15s 后无新推理即释放）为主，此阈值为兜底口径。
const IDLE_RELEASE_SECS: u64 = 15;

/// 会话世代计数：每次推理开始 +1；延迟清扫任务凭世代号判断
/// 「期间是否有新推理」，有则自动作废（新一轮完成后会重新安排）。
static SESSION_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 当前会话世代号（供 UI 层安排延迟清扫时快照）
pub fn session_generation() -> u64 {
    SESSION_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// 立即释放全部模型会话（显存/内存归还系统；下次推理重新加载）
pub fn sweep_all_sessions() {
    nn_log("会话空闲，释放全部模型会话");
    *SESSIONS.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

static SESSIONS: Mutex<Option<HashMap<String, SessionEntry>>> = Mutex::new(None);

/// 清扫闲置会话（`keep` 为本次要用的模型，不释放）
fn sweep_idle_sessions(map: &mut HashMap<String, SessionEntry>, keep: Option<&str>) {
    let now = std::time::Instant::now();
    let stale: Vec<String> = map
        .iter()
        .filter(|(id, e)| {
            let idle = now.duration_since(e.last_used).as_secs();
            idle > IDLE_RELEASE_SECS && Some(id.as_str()) != keep
        })
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        nn_log(&format!("会话闲置超 {IDLE_RELEASE_SECS}s，释放：{id}"));
        map.remove(&id);
    }
}

/// DML 注册失败后置位，后续会话直接走 CPU（避免每次都白试一遍）。
static DML_FAILED: Mutex<bool> = Mutex::new(false);

/// DML 显存不足（OOM）的模型黑名单：这些模型显存需求超过 GPU 能力，
/// 本次运行内固定走 CPU——但不影响其它模型继续用 GPU。
/// （重启应用后重试：显存占用环境可能已变化）
static DML_OOM_MODELS: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// 执行后端选择：体积 > SMALL_MODEL_MB 的模型尝试 DirectML，
/// 小模型阈值（MB）：低于此值才考虑 GPU，避免大模型撑爆显存。
const SMALL_MODEL_MB: f64 = 64.0;
/// 大模型阈值（MB）：高于此值直接走 CPU，DirectML 显存通常不够。
const LARGE_MODEL_MB: f64 = 300.0;

fn want_dml(info: &NnModelInfo) -> bool {
    let size_mb = info.size_mb;
    // COCOLE_EP=cpu 强制 CPU（调试用）；DML 曾失败也直接走 CPU
    if std::env::var("COCOLE_EP").is_ok_and(|v| v.eq_ignore_ascii_case("cpu")) {
        return false;
    }
    if *DML_FAILED.lock().unwrap_or_else(|p| p.into_inner()) {
        return false;
    }
    // 该模型曾因 OOM 固定走 CPU（显存需求超过 GPU 能力）
    if DML_OOM_MODELS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|set| set.contains(&info.id))
        .unwrap_or(false)
    {
        return false;
    }
    // 小模型：走 GPU（显存够用）
    // 中等模型：走 GPU（尝试）
    // 大模型（>300MB）：直接走 CPU，避免 DirectML 显存不足
    size_mb > SMALL_MODEL_MB && size_mb <= LARGE_MODEL_MB
}

/// 确保 ONNX Runtime DLL 能被加载到正确的位置。
///
/// `ort` 的 load-dynamic 模式默认按**裸文件名** `onnxruntime.dll` 加载，
/// Windows 按标准 DLL 搜索顺序去找，可能命中系统/PATH 里的旧版本（实测会报
/// `Unsupported model IR version: 10, max supported IR version: 9`）。
/// 显式指到项目自带的 DLL 后立刻跑通。DirectML EP 还依赖同目录的
/// `onnxruntime_providers_shared.dll`，需要把该目录追加进 PATH。
///
/// 已设置过 `ORT_DYLIB_PATH` 就不覆盖 —— 使用者仍可指向自己的 ORT（如 GPU 版）。
fn ensure_dylib_path() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some_and(|v| !v.is_empty()) {
        return;
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("ort-libs").join("onnxruntime.dll"));
            candidates.push(dir.join("..").join("..").join("ort-libs").join("onnxruntime.dll"));
            candidates.push(dir.join("onnxruntime.dll"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("ort-libs")
            .join("onnxruntime.dll"),
    );

    for c in candidates {
        if c.exists() {
            let s = c.to_string_lossy().to_string();
            // 含分号等环境变量分隔符时拒绝写入，避免污染其他变量
            if s.contains(';') {
                return;
            }
            // SAFETY：单线程初始化窗口内调用（get_session 首次触碰 ort 之前），
            // 与旧项目 infer.rs 的已验证做法一致。
            unsafe { std::env::set_var("ORT_DYLIB_PATH", &s) };
            if let Some(dll_dir) = c.parent() {
                let dll_dir_str = dll_dir.to_string_lossy().to_string();
                if !dll_dir_str.contains(';') {
                    let current_path = std::env::var("PATH").unwrap_or_default();
                    if !current_path.split(';').any(|p| p == dll_dir_str) {
                        unsafe {
                            std::env::set_var("PATH", format!("{};{}", dll_dir_str, current_path));
                        }
                    }
                }
            }
            return;
        }
    }
}

/// 生成加载失败时的人类可读提示（列出全部已尝试路径）。
fn dll_search_hint() -> String {
    let mut hint = String::from("已尝试：");
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            hint.push_str(&format!("{}、", dir.join("ort-libs").join("onnxruntime.dll").display()));
        }
    }
    hint.push_str(&format!(
        "{}（可用环境变量 ORT_DYLIB_PATH 指向自带的 onnxruntime.dll）",
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ort-libs").join("onnxruntime.dll").display()
    ));
    hint
}

/// 加载（或复用）会话。GPU 自动分配 + 失败回退，见模块注释。
fn get_session(info: &NnModelInfo) -> Result<(), String> {
    // 必须在第一次触碰 ort 之前设好 DLL 路径，否则可能加载到系统里的旧版本
    ensure_dylib_path();

    let mut guard = match SESSIONS.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            nn_log("会话锁已中毒，尝试恢复");
            poisoned.into_inner()
        }
    };
    let map = guard.get_or_insert_with(HashMap::new);
    sweep_idle_sessions(map, Some(&info.id));
    if let Some(e) = map.get_mut(&info.id) {
        e.last_used = std::time::Instant::now();
        return Ok(());
    }

    let path = PathBuf::from(&info.path);
    if !path.exists() {
        return Err(format!(
            "模型文件不存在：{}。请在设置里重新扫描，或把模型放回 models/ 目录",
            path.display()
        ));
    }
    if path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() != Some("onnx") {
        return Err(format!(
            "不支持的模型格式：{}（当前仅支持 ONNX 格式 .onnx）",
            path.display()
        ));
    }

    let use_dml = want_dml(info);
    nn_log(&format!(
        "加载「{}」{:.1}MB（输入 {}²）→ {}",
        info.name,
        info.size_mb,
        info.input_size,
        if use_dml { "GPU(DirectML)" } else { "CPU" }
    ));

    let new_builder = || {
        // load-dynamic 模式下 DLL 在首次调用 ORT API（即这里）才真正加载；
        // 失败时附上已尝试路径，让「找不到 DLL」类错误可以自助排障。
        Session::builder().map_err(|e| {
            trf("创建会话构建器失败：{}\n若为 onnxruntime.dll 加载失败，{}", &[&e.to_string(), dll_search_hint().as_str()])
        })?
            .with_intra_threads(0) // 0 = 用 ORT 默认（通常等于物理核数）
            .map_err(|e| trf("设置线程数失败：{}", &[&e.to_string()]))
    };

    // DirectML EP 在前；注册失败时 ORT 的行为是打日志后静默回退，
    // 这里把注册调用的 Err 显式捕获并置降级标记，后续直接走 CPU。
    let mut builder = if use_dml {
        match new_builder()?.with_execution_providers([ort::ep::DirectML::default().build()]) {
            Ok(b) => {
                nn_log("DirectML EP 注册成功");
                b
            }
            Err(e) => {
                *DML_FAILED.lock().unwrap_or_else(|p| p.into_inner()) = true;
                nn_log(&format!("DirectML EP 注册失败，回退 CPU：{e}"));
                new_builder()?
            }
        }
    } else {
        new_builder()?
    };

    let session = builder
        .commit_from_file(&path)
        .map_err(|e| trf("加载模型失败（{}）：{}\n文件损坏或不是受支持的 ONNX 导出版本时会出现此错误", &[path.display().to_string().as_str(), &e.to_string()]))?;

    map.insert(
        info.id.clone(),
        SessionEntry {
            session: Arc::new(Mutex::new(session)),
            last_used: std::time::Instant::now(),
        },
    );
    Ok(())
}

/// 主动释放某个模型的会话（重扫目录 / 换模型文件时用）。
pub fn release(model_id: &str) {
    let mut guard = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(map) = guard.as_mut() {
        map.remove(model_id);
    }
}

/// 释放全部会话。
pub fn release_all() {
    *SESSIONS.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

/* ═════════════════ 推理主入口 ═════════════════ */

/// NN 推理全局闸门：同一时刻只允许一个 ONNX 推理在跑。
///
/// 大模型（InSPyReNet 等 1024² 档）单次 CPU 推理可达数十秒、激活内存以 GB 计；
/// 自动重抠场景下旧线程无法中途取消，若放行并发叠加会数倍膨胀内存直至
/// 系统 OOM 把进程整个杀掉（表现为闪退）。调用方在占闸期间完成的推理
/// 必须调用 `release_inference` 归还。
static NN_INFERENCE_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 尝试占住推理闸门：`true` = 占住成功，调用方完成后必须调 `release_inference`。
pub fn claim_inference() -> bool {
    std::sync::atomic::AtomicBool::compare_exchange(
        &NN_INFERENCE_BUSY,
        false,
        true,
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Acquire,
    )
    .is_ok()
}

/// 归还推理闸门（与 `claim_inference` 成对使用）。
pub fn release_inference() {
    NN_INFERENCE_BUSY.store(false, std::sync::atomic::Ordering::Release);
}

/// 推理闸门 RAII 守卫：持有期间占住闸门，drop（含 panic 展开栈）时自动归还。
/// 用它替代手动 claim/release 对，杜绝 panic 或提前 return 造成的永久锁死
/// ——闸门一旦泄漏，此后所有神经网络抠图都会失败/挂起，直到重启进程。
pub struct InferenceGuard;

impl Drop for InferenceGuard {
    fn drop(&mut self) {
        release_inference();
    }
}

/// 尝试占住推理闸门；成功返回守卫（drop 时自动归还），失败返回 `None`。
pub fn try_claim_inference() -> Option<InferenceGuard> {
    claim_inference().then_some(InferenceGuard)
}

/// 从输出里定位 alpha 平面，并回传它的真实尺寸（宽、高）。
///
/// BiRefNet 输出 [1,1,1024,1024]；u2netp 另有 6 路 side 输出（d0..d6），
/// 取第一个输出即 d0，实测与主输出等价。
///
/// Shape 的维度是 i64，转 usize 前要过滤非正值 —— 动态维度可能是 -1。
fn extract_alpha(out: &ort::value::ValueRef<'_>) -> Result<(Vec<f32>, usize, usize), String> {
    // 兼容 f32 / f16 输出（BEN2 等新模型输出为 half）；其余类型明确报错
    let (shape, data) = match out.try_extract_tensor::<f32>() {
        Ok((shape, data)) => (shape.clone(), data.to_vec()),
        Err(e32) => match out.try_extract_tensor::<half::f16>() {
            Ok((shape, f16_data)) => {
                let f32_data: Vec<f32> = f16_data.iter().map(|h| h.to_f32()).collect();
                (shape.clone(), f32_data)
            }
            Err(_) => {
                return Err(format!(
                    "模型输出不是 f32/f16 张量（格式不受支持）：{e32}"
                ))
            }
        },
    };

    let dims: Vec<i64> = shape.iter().copied().collect();
    // 一律取最后一对维度：无论 [1,1,H,W] 还是 [H,W]，alpha 都在最后一维里
    let (h, w) = match dims.as_slice() {
        [_, _, hh, ww] | [hh, ww] => (*hh, *ww),
        other => {
            return Err(format!(
                "无法解析输出形状 {other:?}（期望 4 维 [1,1,H,W] 或 2 维 [H,W]）——不是抠图模型？"
            ))
        }
    };
    if w <= 0 || h <= 0 {
        return Err(trf("输出形状非法：{}", &[&format!("{dims:?}")]));
    }
    let (w, h) = (w as usize, h as usize);
    let expected = w * h;
    if data.len() < expected {
        return Err(format!(
            "输出数据长度 {} 小于预期的 {expected}（形状 {dims:?}）",
            data.len()
        ));
    }
    Ok((data[..expected].to_vec(), w, h))
}

/// 输出 alpha 的 min-max 归一化。
///
/// u2net / IS-Net(InSPyReNet) / BiRefNet 这一族的原始输出是**未归一化的
/// logits**，范围可远超 [0,1]（实测出现 -40 ~ 60 量级）——直接按 [0,1]
/// 缩放乘 255 会得到全白/全黑蒙版。rembg 对同一批 onnx 的后处理正是
/// min-max 归一化，这里对齐其行为；范围过小（空输出）时回退全 0。
fn normalize_minmax(alpha: &mut [f32]) {
    if alpha.is_empty() {
        return;
    }
    let mut mi = f32::INFINITY;
    let mut ma = f32::NEG_INFINITY;
    for &v in alpha.iter() {
        if v < mi {
            mi = v;
        }
        if v > ma {
            ma = v;
        }
    }
    let range = ma - mi;
    if !range.is_finite() || range <= 1e-6 {
        alpha.fill(0.0);
        return;
    }
    for v in alpha.iter_mut() {
        *v = (*v - mi) / range;
    }
}

/// 执行推理，DirectML 失败时自动降级到 CPU 并重试。
fn run_with_fallback(
    info: &NnModelInfo,
    input: &[f32],
    size: usize,
) -> Result<(Vec<f32>, usize, usize), String> {
    let used_dml = want_dml(info);
    // 第一次尝试（可能是 GPU）
    match run_once(info, input, size) {
        Ok(result) => {
            // DirectML 静默垃圾守卫：某些 ORT 发行版（如官方 win-x64 CPU 包）
            // 的 DML EP 注册返回"成功"，推理却不报错地输出**恒定值**蒙版
            // （常数经 min-max 归一化后整体归零）。恒定 alpha 对任何输入都
            // 无意义——视为 DML 故障：置降级标记、释放会话、用 CPU 会话
            // 重跑一次（want_dml 此后返回 false，新会话必为 CPU）。
            if used_dml && {
                let mut mi = f32::INFINITY;
                let mut ma = f32::NEG_INFINITY;
                for &v in &result.0 {
                    if v < mi {
                        mi = v;
                    }
                    if v > ma {
                        ma = v;
                    }
                }
                (ma - mi) <= 1e-4
            } {
                nn_log("DirectML 推理输出全零（静默垃圾），降级到 CPU 并重试");
                *DML_FAILED.lock().unwrap_or_else(|p| p.into_inner()) = true;
                release(&info.id);
                get_session(info)?;
                return run_once(info, input, size);
            }
            Ok(result)
        }
        Err(e) => {
            // 只处理 DirectML 相关失败；其它错误原样上抛
            if !e.contains("Dml") && !e.contains("DirectML") {
                return Err(e);
            }

            let is_oom = e.contains("8007000E") || e.to_lowercase().contains("out of memory");
            // 失败会话已不可用，释放它
            release(&info.id);

            if is_oom {
                // OOM = 显存不足，可能是其它闲置会话占着显存。
                // 全量释放腾出显存后重试一次 DML；仍失败则把该模型拉黑（固定 CPU），
                // 但不置全局 DML_FAILED——其它模型继续用 GPU。
                nn_log("DirectML 显存不足（OOM），释放全部会话腾显存后重试");
                release_all();
                get_session(info)?;
                match run_once(info, input, size) {
                    Ok(result) => {
                        nn_log("清扫显存后 DirectML 重试成功");
                        return Ok(result);
                    }
                    Err(e2) => {
                        nn_log(&format!("DirectML 重试仍 OOM，该模型本次运行内固定走 CPU：{e2}"));
                        DML_OOM_MODELS
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .get_or_insert_with(std::collections::HashSet::new)
                            .insert(info.id.clone());
                        release(&info.id);
                        get_session(info)?;
                        return run_once(info, input, size);
                    }
                }
            }

            // 非 OOM 的 DML 失败：能力性问题，全局降级 CPU（避免每次白试）
            nn_log("DirectML 推理失败，降级到 CPU 并重试");
            *DML_FAILED.lock().unwrap_or_else(|p| p.into_inner()) = true;
            get_session(info)?;
            run_once(info, input, size)
        }
    }
}

/// 单次推理执行。
///
/// 会话缓存可能在 `get_session` 与本函数之间的无锁预处理空窗内被并发释放
/// （典型：推理在途时点了「释放内存缓存」，或重新扫描触发单模型 `release`）。
/// 此时按 toast 承诺的「下次推理首张稍慢」就地懒重载会话并重试一次，
/// 而不是把在途推理直接打成「会话缓存已丢失」。
fn run_once(
    info: &NnModelInfo,
    input: &[f32],
    size: usize,
) -> Result<(Vec<f32>, usize, usize), String> {
    match run_once_locked(info, input, size) {
        Err(e) if e == CACHE_LOST || e == CACHE_RELEASED => {
            nn_log("会话缓存被并发释放，就地重载会话后重试本次推理");
            get_session(info)?;
            run_once_locked(info, input, size)
        }
        r => r,
    }
}

/// 在会话锁内执行一次推理；缓存缺失时返回哨兵错误（由 [`run_once`] 懒重载重试）。
fn run_once_locked(
    info: &NnModelInfo,
    input: &[f32],
    size: usize,
) -> Result<(Vec<f32>, usize, usize), String> {
    SESSION_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // 全局锁只用来取句柄：拿到 Arc 立即释放，推理不占用它。
    // 这样 UI 的释放 / 重扫 / 空闲清扫在数十秒的推理期间仍能瞬时完成。
    let handle: Arc<Mutex<Session>> = {
        let mut guard = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        let map = guard.as_mut().ok_or(CACHE_LOST)?;
        sweep_idle_sessions(map, Some(&info.id));
        match map.get_mut(&info.id) {
            Some(e) => {
                e.last_used = std::time::Instant::now();
                Arc::clone(&e.session)
            }
            None => return Err(CACHE_RELEASED.to_string()),
        }
    };
    // 同一模型的并发推理由这把每会话锁串行（全局推理闸门本就只允许一个）。
    let mut session = handle.lock().unwrap_or_else(|p| p.into_inner());

    let input_name = session.inputs()[0].name().to_string();
    // 输入类型兼容：按模型输入元数据决定构造 f32 还是 f16 张量
    // （部分模型如 BEN2 系列的输入本身是 half，直接喂 f32 会被 ORT 拒绝）
    let want_f16 = matches!(
        session.inputs()[0].dtype(),
        ort::value::ValueType::Tensor {
            ty: ort::value::TensorElementType::Float16,
            ..
        }
    );
    let input_shape = vec![1i64, 3, size as i64, size as i64];
    let input_tensor: ort::value::Value = if want_f16 {
        let f16_data: Vec<half::f16> = input.iter().map(|v| half::f16::from_f32(*v)).collect();
        ort::value::Tensor::from_array((input_shape, f16_data))
            .map_err(|e| trf("构造输入张量失败：{}", &[&e.to_string()]))?
            .into()
    } else {
        ort::value::Tensor::from_array((input_shape, input.to_vec()))
            .map_err(|e| format!("构造输入张量失败：{e}"))?
            .into()
    };

    let outputs = session
        .run(ort::inputs![input_name => input_tensor])
        .map_err(|e| trf("推理失败：{}", &[&e.to_string()]))?;

    let picked = outputs
        .iter()
        .next()
        .map(|(_, v)| v)
        .ok_or("模型没有输出（可能不是抠图模型）")?;
    extract_alpha(&picked)
}

/// 抠图主入口：RGBA 像素 → 原图尺寸的 0-255 软蒙版。
///
/// `rgba` 长度必须等于 width×height×4；模型会话按 info.id 缓存。
pub fn matte(info: &NnModelInfo, rgba: &[u8], width: usize, height: usize) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 {
        return Err(trf("图片尺寸非法：{}×{}", &[&width.to_string(), &height.to_string()]));
    }
    if rgba.len() != width * height * 4 {
        return Err(format!(
            "像素数据长度不匹配：收到 {} 字节，{width}×{height} RGBA 应为 {} 字节",
            rgba.len(),
            width * height * 4
        ));
    }
    if !info.path.ends_with(".onnx") && !info.path.to_ascii_lowercase().ends_with(".onnx") {
        return Err(trf("不支持的模型格式：{}（仅支持 .onnx）", &[&info.path]));
    }

    // 性能诊断打点：COCOLE_DIAG=1 时输出各阶段耗时
    let diag = std::env::var("COCOLE_DIAG").is_ok_and(|v| !v.is_empty() && v != "0");
    let t0 = std::time::Instant::now();

    get_session(info)?;

    let size = info.input_size;
    let input = to_nchw(rgba, width, height, size);
    let t_pre = t0.elapsed();

    // 推理 + DirectML 失败自动降级重试
    let output = run_with_fallback(info, &input, size)?;
    let t_infer = t0.elapsed() - t_pre;

    let (mut alpha, aw, ah) = output;
    normalize_minmax(&mut alpha);
    let mask = alpha_to_mask(&alpha, aw, ah, width, height);
    let t_post = t0.elapsed() - t_pre - t_infer;
    if diag {
        nn_log(&format!(
            "[diag] matte {}×{} → {}: 预处理 {:?} · 推理 {:?} · 蒙版上采样 {:?}",
            width,
            height,
            size,
            t_pre,
            t_infer,
            t_post
        ));
    }
    Ok(mask)
}

/* ═════════════════ 冒烟测试（cargo test --offline） ═════════════════ */

#[cfg(test)]
mod tests {
    use super::*;

    /// 白底 + 中央黑色方块 → 跑 U²-NetP（4.4MB，CPU 数秒）。
    /// 断言：扫描发现模型、DLL 加载成功、推理产出有效蒙版（0/255 两态都有，
    /// 且方块中心的 alpha 显著高于角落）。
    #[test]
    fn smoke_u2netp() {
        let (dir, models) = scan(None);
        assert!(dir.is_some(), "models 目录不存在");
        let info = models
            .iter()
            .find(|m| m.file == "u2netp.onnx")
            .expect("models 目录里应有 u2netp.onnx");

        let (w, h) = (320usize, 320usize);
        let mut rgba = vec![255u8; w * h * 4]; // 白底
        for y in 100..220 {
            for x in 100..220 {
                let i = (y * w + x) * 4;
                rgba[i] = 20;
                rgba[i + 1] = 20;
                rgba[i + 2] = 20;
                rgba[i + 3] = 255;
            }
        }
        let mask = matte(info, &rgba, w, h).expect("推理失败");
        assert_eq!(mask.len(), w * h);

        let center = mask[160 * w + 160];
        let corner = mask[8 * w + 8];
        assert!(center > 128, "方块中心应为前景（实际 {center}）");
        assert!(corner < 128, "白色角落应为背景（实际 {corner}）");
        // 软蒙版特征：不是非黑即白的二值图（边缘存在中间值）
        assert!(mask.iter().any(|&v| v > 8 && v < 247), "应存在软边过渡");

        // 回归：缓存被并发释放（如推理在途时点了「释放内存缓存」）后，
        // run_once 应就地懒重载成功，而不是报「会话缓存已丢失」。
        release_all();
        let input = to_nchw(&rgba, w, h, info.input_size);
        let (alpha, aw, ah) =
            run_once(info, &input, info.input_size).expect("释放缓存后 run_once 应懒重载成功");
        assert_eq!(aw * ah, info.input_size * info.input_size);
        assert!(!alpha.is_empty());
    }

    /// 探针：真实运行 model_fp16.onnx（InSPyReNet，本机插件目录 199MB）。
    /// 用于排查「本地打包正常、GitHub 分发包失败」的差异定位；
    /// #[ignore]：依赖本机大模型文件，仅 `cargo test -- --ignored` 手动运行。
    ///
    /// 环境变量注入（排查矩阵用）：
    /// - `PROBE_ORT_DLL`：指定 onnxruntime.dll 路径（对比 CI 包 / 本地包 DLL）
    /// - `PROBE_IMG`：真实照片路径（默认合成图）
    /// - `PROBE_W`/`PROBE_H`：测试图尺寸（默认 640×427）
    #[test]
    #[ignore = "依赖本机插件目录的 199MB 模型文件，仅手动排查时运行"]
    fn probe_inspyrenet_model_fp16() {
        if let Ok(dll) = std::env::var("PROBE_ORT_DLL") {
            std::env::set_var("ORT_DYLIB_PATH", &dll);
            println!("[probe] ORT DLL = {dll}");
        }
        let path = r"D:\helloworld\onnx\model_fp16.onnx";
        let info = NnModelInfo {
            id: "nn:model_fp16.onnx".into(),
            label: "InSPyReNet".into(),
            display_name: "InSPyReNet".into(),
            name: "InSPyReNet".into(),
            file: "model_fp16.onnx".into(),
            path: path.into(),
            size_mb: 189.9,
            input_size: 1024,
            category: ModelCategory::Salient,
        };
        // 测试图：默认白底黑方块；PROBE_IMG 给定时读真实照片并缩放到指定尺寸
        let (w, h): (usize, usize) = match (
            std::env::var("PROBE_W").ok().and_then(|v| v.parse().ok()),
            std::env::var("PROBE_H").ok().and_then(|v| v.parse().ok()),
        ) {
            (Some(w), Some(h)) => (w, h),
            _ => (640, 427),
        };
        let mut rgba = vec![255u8; w * h * 4];
        if let Ok(img_path) = std::env::var("PROBE_IMG") {
            let im = image::open(&img_path).expect("PROBE_IMG 读取失败").to_rgba8();
            let im = image::imageops::resize(&im, w as u32, h as u32, image::imageops::FilterType::Triangle);
            rgba = im.into_raw();
            println!("[probe] IMG = {img_path} ({w}×{h})");
        } else {
            for y in h / 4..h * 3 / 4 {
                for x in w / 4..w * 3 / 4 {
                    let i = (y * w + x) * 4;
                    rgba[i] = 30;
                    rgba[i + 1] = 30;
                    rgba[i + 2] = 30;
                    rgba[i + 3] = 255;
                }
            }
        }
        let t0 = std::time::Instant::now();
        match matte(&info, &rgba, w, h) {
            Ok(mask) => {
                let mn = *mask.iter().min().unwrap();
                let mx = *mask.iter().max().unwrap();
                let fg = mask.iter().filter(|&&v| v > 127).count();
                let soft = mask.iter().filter(|&&v| v > 8 && v < 247).count();
                println!("[probe] OK in {:?}", t0.elapsed());
                println!("[probe] mask range {mn}..{mx}, fg {fg}/{} ({:.1}%), soft {soft}", w * h, fg as f64 / (w * h) as f64 * 100.0);
                // 前景质心（诊断蒙版空间错位）
                let (mut sx, mut sy, mut n) = (0u64, 0u64, 0u64);
                for (idx, &v) in mask.iter().enumerate() {
                    if v > 127 {
                        sx += (idx % w) as u64;
                        sy += (idx / w) as u64;
                        n += 1;
                    }
                }
                if n > 0 {
                    println!(
                        "[probe] fg centroid = ({:.2}, {:.2}) normalized (image center = 0.5, 0.5)",
                        sx as f64 / n as f64 / w as f64,
                        sy as f64 / n as f64 / h as f64
                    );
                }
            }
            Err(e) => panic!("[probe] matte FAILED: {e}"),
        }
    }



    /// alpha_to_mask 并行路径与串行逐行结果必须逐字节一致
    /// （行分块多线程改造的回归锁）
    #[test]
    /// logits 越界的输出必须被 min-max 拉回 [0,1]，否则蒙版全白（v0.4.0 闪退/异常修复）。
    fn normalize_minmax_maps_logits_to_unit_range() {
        // 超出 [0,1] 的原始 logits
        let mut a = vec![-40.0f32, -10.0, 0.0, 20.0, 60.0];
        normalize_minmax(&mut a);
        assert!((a[0] - 0.0).abs() < 1e-5);
        assert!((a[4] - 1.0).abs() < 1e-5);
        assert!(a.iter().all(|v| (0.0..=1.0).contains(v)));

        // 常数输出（空范围）→ 全 0，不产生 NaN
        let mut c = vec![0.5f32; 9];
        normalize_minmax(&mut c);
        assert!(c.iter().all(|v| *v == 0.0));

        // 已在 [0,1] 的输出同样成立
        let mut b = vec![0.0f32, 0.5, 1.0];
        normalize_minmax(&mut b);
        assert!((b[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn alpha_to_mask_parallel_matches_serial() {
        // 非平凡 alpha：伪随机渐变，覆盖 0..1 全域
        let (aw, ah) = (64usize, 48usize);
        let alpha: Vec<f32> = (0..aw * ah)
            .map(|i| (((i * 37 + 11) % 97) as f32 / 97.0).clamp(0.0, 1.0))
            .collect();
        let (dw, dh) = (1200usize, 900usize); // >1M 像素，必然走并行路径
        let sx = aw as f32 / dw as f32;
        let sy = ah as f32 / dh as f32;

        let mut expect = vec![0u8; dw * dh];
        for y in 0..dh {
            fill_mask_row(&mut expect[y * dw..], y, &alpha, aw, ah, dw, sx, sy);
        }
        let got = alpha_to_mask(&alpha, aw, ah, dw, dh);
        assert_eq!(expect, got, "并行蒙版与串行参考不一致");
    }

    /// 阶段耗时测量（#[ignore]，release 口径）：
    ///   导入阶段 = JPEG/PNG 解码（imgcore::load_file）
    ///   抠图阶段 = nn::matte（含预处理/推理/蒙版上采样，session 预热后）
    /// 对 tools/test-portrait*.jpg 逐张测量并输出两阶段占比，
    /// 同时每张跑两次 matte 验证「抠图工具与证件照同函数同速」。
    /// 运行：cargo test --release diag_stage_timing -- --ignored --nocapture
    #[test]
    #[ignore]
    fn diag_stage_timing() {
        std::env::set_var("COCOLE_DIAG", "1");
        let (_dir, models) = scan(None);
        let info = models
            .iter()
            .find(|m| m.file == "modnet.onnx")
            .expect("无 modnet.onnx");
        let tools = concat!(env!("CARGO_MANIFEST_DIR"), "/tools");
        let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(tools)
            .expect("tools 目录不存在")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("test-portrait") && n.ends_with(".jpg"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();
        assert!(!paths.is_empty());

        // 会话预热（首次推理含 EP 初始化，单独报告）
        let warm = vec![128u8; 64 * 64 * 4];
        let t = std::time::Instant::now();
        let _ = matte(info, &warm, 64, 64);
        println!("—— 会话预热（EP 初始化，仅首次）：{:?} ——", t.elapsed());

        println!(
            "{:<22} {:>11} {:>9} {:>9} {:>9} {:>8}",
            "图片", "尺寸", "解码ms", "抠图ms", "apply ms", "解码占比"
        );
        for path in &paths {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            // 导入阶段：解码
            let t = std::time::Instant::now();
            let loaded = crate::core::image::load_file(path).expect("解码失败");
            let t_decode = t.elapsed();
            let (w, h) = (loaded.rgba.width() as usize, loaded.rgba.height() as usize);
            let mp = w as f32 * h as f32 / 1e6;

            // 抠图阶段：matte 跑两次取第二次（暖会话），再叠加 apply_mask
            let t = std::time::Instant::now();
            let mask0 = matte(info, loaded.rgba.as_raw(), w, h).expect("抠图失败");
            let t_cold = t.elapsed();
            let t = std::time::Instant::now();
            let mask = matte(info, loaded.rgba.as_raw(), w, h).expect("抠图失败");
            let t_matte = t.elapsed();
            let t = std::time::Instant::now();
            let _fg = crate::core::image::apply_mask(&loaded.rgba, &mask);
            let t_apply = t.elapsed();

            let total = t_decode.as_secs_f32() + t_matte.as_secs_f32() + t_apply.as_secs_f32();
            let dec_pct = t_decode.as_secs_f32() / total * 100.0;
            println!(
                "{:<22} {:>4.1}MP@{}x{} {:>9.0?} {:>9.0} {:>9.1} {:>7.0}%",
                name,
                mp,
                w,
                h,
                t_decode,
                t_matte.as_millis(),
                t_apply.as_millis(),
                dec_pct
            );
            println!(
                "    └ 冷启动 matte（含会话创建部分成本）：{:?}",
                t_cold
            );
            let _ = mask0;
        }
    }
}