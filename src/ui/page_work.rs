//! 抠图工具页：顶部工具条 + 画布 + 底部视图条 + 右侧检查器
//!
//! 对应原项目 `#pageWork`：toolstrip / canvas-wrap / stage-bar / inspector。
//! state 集中在 App，本模块负责自己的 update 分支与 view。

use std::path::PathBuf;

use iced::widget::{button, column, container, mouse_area, pick_list, row, scrollable, stack, text, Space};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::{Alignment, Element, Length, Size, Task};

use crate::app::{App, Drag, WorkState};
use crate::core::history::Snapshot;
use crate::core::image::{self as img, ExportFormat, ExportSize, LoadedImage};
use crate::core::matting::{self, MaskResult, MatteParams, SampleMode};
use crate::core::pyramid::{self as pyramid, Level, PyramidBytes};
use crate::message::*;
use crate::theme::*;
use crate::ui::canvas_view::{self as cv, HANDLE_HIT, Rect, StageKind, StageView};
use iced::Rectangle;
use crate::ui::icons::icon;
use crate::ui::widgets::*;
use crate::core::i18n::{tr, trf};

/// 图片在画布中的屏幕矩形（与画布程序里 `cv::image_rect` 用的是同一套公式）
///
/// 对比分割线的定位基准必须是**图片矩形**：参考项目 `moveHandle()` 明确按
/// `#viewport` 的 rect 换算百分比，滑块本身也是 `.viewport` 的子元素。
fn stage_dest(app: &App, bounds: (f32, f32)) -> iced::Rectangle {
    let (iw, ih) = app
        .work
        .src
        .as_ref()
        .map(|s| (s.width() as f32, s.height() as f32))
        .unwrap_or((0.0, 0.0));
    cv::image_rect(
        (iw, ih),
        app.work.zoom,
        app.work.pan,
        Size::new(bounds.0, bounds.1),
    )
}

/* ═════════════════ 显示金字塔（G2/G3） ═════════════════ */

/// 关键路径耗时打点：`COCOLE_PERF=1` 时输出到 stderr。
/// 全程本地、无遥测；用于量化「异步是否真的搬离 UI 线程」与优化收益。
fn perf_log(stage: &str, t0: std::time::Instant) {
    if std::env::var_os("COCOLE_PERF").is_some() {
        eprintln!("[perf] {} {:.1}ms", stage, t0.elapsed().as_secs_f32() * 1000.0);
    }
}

/// 画布可视区长边（逻辑像素），用于计算预览层目标尺寸
fn viewport_long(w: &WorkState) -> f32 {
    w.stage.0.max(w.stage.1).max(320.0)
}

/// 把后台生成的两级显示字节转成 UI 句柄。
/// 字节是 move 进来的，这里只做 `Handle::from_rgba` 包装（O(1)），
/// 不在 UI 线程做 resize 或整图深拷贝。
fn set_result_handles(w: &mut WorkState, display: PyramidBytes) {
    let (full, preview) = display.into_handles();
    w.result_handle = Some(full);
    w.result_preview = preview;
}

/// 载入 / 旋转 / 裁剪 / 撤销后刷新原图层句柄。
/// 这些是低频操作（本身已含整图变换），直接用 `tex_display` 生成 full 层，
/// 并同步生成一次预览层；缩放高频路径因此永远不再触碰全分辨率纹理。
fn set_orig_handles(w: &mut WorkState, img: &image::RgbaImage) {
    let p = pyramid::build(img, viewport_long(w));
    let (full, preview) = p.into_handles();
    w.orig_handle = Some(full);
    w.orig_preview = preview;
}

/* ═════════════════ 异步小工具 ═════════════════ */

/// 把阻塞工作丢到 tokio 阻塞线程池，完成后映射成 Message。
/// 使用 `spawn_blocking` 复用线程池，避免每次 `thread::spawn` 的创建/销毁开销。
pub(crate) fn blocking_task<T, M>(
    f: impl FnOnce() -> T + Send + 'static,
    map: impl Fn(Option<T>) -> M + Send + 'static,
) -> Task<M>
where
    T: Send + 'static,
    M: Send + 'static,
{
    Task::perform(
        async move { tokio::task::spawn_blocking(f).await.ok() },
        map,
    )
}

/// 后台读图（文件对话框与命令行参数共用同一条链路）。
/// 解码 + 显示金字塔同批在后台完成，UI 线程不再为大图做 resize / 整图拷贝。
fn load_task(path: PathBuf, viewport: f32) -> Task<Message> {
    blocking_task(
        move || {
            let t0 = std::time::Instant::now();
            img::load_file(&path)
                .map(|l| {
                    let display = Box::new(pyramid::build(&l.rgba, viewport));
                    perf_log("decode+pyramid", t0);
                    let mut p = to_payload(l);
                    p.display = Some(display);
                    p
                })
                .map_err(|e| e.to_string())
        },
        |r| {
            Message::Work(WorkMessage::ImageLoaded(match r {
                Some(Ok(p)) => Ok(Box::new(p)),
                Some(Err(e)) => Err(e),
                None => Err(tr("读取线程异常退出").to_string()),
            }))
        },
    )
}

/// 单选图片（首页「开始」卡与工作台「打开图片」共用同一条链路）
pub(crate) async fn pick_image() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .add_filter(tr("图片"), &["png", "jpg", "jpeg", "webp", "bmp"])
        .pick_file()
        .await
        .map(|h| h.path().to_path_buf())
}

async fn pick_images() -> Vec<PathBuf> {
    let picked = rfd::AsyncFileDialog::new()
        .add_filter(tr("图片"), &["png", "jpg", "jpeg", "webp", "bmp"])
        .pick_files()
        .await;
    match picked {
        Some(v) => v.iter().map(|h| h.path().to_path_buf()).collect(),
        None => Vec::new(),
    }
}

async fn pick_save(name: String, bytes: Vec<u8>) -> Option<PathBuf> {
    let handle = rfd::AsyncFileDialog::new().set_file_name(name).save_file().await?;
    handle.write(&bytes).await.ok()?;
    Some(handle.path().to_path_buf())
}

/* ═════════════════ update ═════════════════ */

pub fn update(app: &mut App, msg: WorkMessage) -> Task<Message> {
    match msg {
        /* ── 打开文件 ── */
        WorkMessage::OpenImage => {
            return Task::perform(pick_image(), |p| Message::Work(WorkMessage::FilePicked(p)));
        }
        WorkMessage::OpenBatch => {
            return Task::perform(pick_images(), |ps| Message::Work(WorkMessage::FilesPicked(ps)));
        }
        WorkMessage::FilePicked(None) => return Task::none(),
        WorkMessage::FilePicked(Some(path)) => {
            app.work.loading = true;
            return load_task(path, viewport_long(&app.work));
        }
        WorkMessage::OpenPath(path) => {
            app.work.loading = true;
            return load_task(path, viewport_long(&app.work));
        }
        WorkMessage::FilesPicked(paths) => {
            // 去重：已在队列里的路径跳过
            let mut added = 0usize;
            let mut dup = 0usize;
            for p in paths {
                if app.work.batch.iter().any(|it| it.path == p) {
                    dup += 1;
                    continue;
                }
                app.work.batch.push(crate::app::WorkBatchItem {
                    path: p,
                    status: crate::app::QueueStatus::Pending,
                    err: None,
                });
                added += 1;
            }
            let dup_note = if dup > 0 { trf("（跳过重复 {} 张）", &[&dup.to_string()]) } else { String::new() };
            app.toast(trf("已加入 {} 张到批量队列{}", &[&added.to_string(), &dup_note]));
            return Task::none();
        }
        WorkMessage::ImageLoaded(Ok(mut payload)) => {
            app.work.loading = false;
            // 换图：作废仍在后台编码的旧导出（结果回传时 seq 不匹配会被丢弃），
            // 同时复位忙碌标志，避免旧任务被丢弃后按钮永久禁用
            app.work.export_seq = app.work.export_seq.wrapping_add(1);
            app.work.exporting = false;
            app.work.stem = payload.stem.clone();
            *app.work.fields.lock().unwrap_or_else(|p| p.into_inner()) = matting::FieldCache::default();
            app.work.src_path = payload.path.clone();
            if let Some(p) = &payload.path {
                app.config.push_recent(&p.to_string_lossy(), &payload.stem);
            }
            let (iw, ih) = (payload.rgba.width(), payload.rgba.height());
            if let Some(display) = payload.display.take() {
                // 后台已生成两级字节，这里只做 Handle 包装（O(1)）
                let (full, preview) = display.into_handles();
                app.work.orig_handle = Some(full);
                app.work.orig_preview = preview;
            } else {
                // 兜底：其他链路转发的 payload 未携带显示字节
                set_orig_handles(&mut app.work, &payload.rgba);
            }
            // 原图直接 move 进 Arc（零拷贝），不再额外 clone 一份 48MB 像素
            app.work.src = Some(std::sync::Arc::new(payload.rgba));
            app.work.mask = None;
            app.work.base = None;
            app.work.result = None;
            app.work.result_handle = None;
            app.work.result_preview = None;
            app.work.history.clear();
            app.work.rot_total = 0.0;
            app.work.rot_deg = 0.0;
            app.work.crop = None;
            app.work.pan = (0.0, 0.0);
            // 新图片导入后自动切换到原图视图
            app.work.view_mode = ViewMode::Original;
            fit_to_window(&mut app.work);
            app.toast(trf("已载入 {}（{}×{}）", &[&payload.stem, &iw.to_string(), &ih.to_string()]));
            return Task::none();
        }
        WorkMessage::ImageLoaded(Err(e)) => {
            app.work.loading = false;
            app.toast_err(trf("打开失败：{}", &[&e]));
            return Task::none();
        }

        /* ── 推理 ── */
        WorkMessage::MaskDone(Ok(payload)) => {
            let (iw, ih) = (payload.result.width(), payload.result.height());
            let (elapsed_ms, holes, bg) = (payload.elapsed_ms, payload.holes, payload.bg);
            let fg_ratio =
                payload.fg_count as f32 / (payload.mask.len().max(1)) as f32;

            app.work.busy = false;
            app.work.metrics = crate::app::Metrics { elapsed_ms, fg_ratio, w: iw, h: ih, holes };
            app.work.last_bg = bg;
            // 结果句柄直接用后台线程生成的两级显示字节（UI 线程零 resize）；
            // 全分辨率像素只保留 Arc 一份，供导出使用。
            set_result_handles(&mut app.work, *payload.display);
            app.work.result = Some(std::sync::Arc::new(payload.result));
            app.work.mask = Some(payload.mask);
            app.work.base = Some(payload.base);
            // 抠图完成后自动切换到结果视图
            app.work.view_mode = ViewMode::Result;

            if std::env::var_os("COCOLE_DEBUG_CANVAS").is_some() {
                let empty = Vec::new();
                let a = app.work.result.as_ref().map(|r| r.as_raw()).unwrap_or(&empty);
                let at = |x: usize, y: usize| {
                    a.get((y * iw as usize + x) * 4 + 3).copied().unwrap_or(0)
                };
                eprintln!(
                    "[matte] {}x{} fg={:.3} alpha(100,52)={} (1000,52)={} (700,522)={} (300,522)={}",
                    iw, ih, fg_ratio, at(100, 52), at(1000, 52), at(700, 522), at(300, 522)
                );
            }

            app.config.stats.count += 1;
            app.config.stats.total_ms += elapsed_ms;
            app.persist();
            return Task::batch([
                Task::none(),
                crate::app::schedule_session_sweep(),
            ]);
        }
        WorkMessage::MaskDone(Err(e)) => {
            app.work.busy = false;
            if e != tr("已取消") {
                app.toast_err(trf("抠图失败：{}", &[&e]));
            }
            return Task::none();
        }
        WorkMessage::RecomputeNow(seq) => {
            // G4：只放行最后一次防抖请求。拖动滑块会排出多个定时器，
            // 旧定时器到期时必须直接丢弃，否则每个都会进 spawn_blocking
            // 并推进取消令牌，白白抢占线程池与推理闸门。
            if seq != app.work.debounce_seq || app.work.src.is_none() || !app.config.auto {
                return Task::none();
            }
            if app.work.busy {
                // 上一轮抠图还在跑：顺延一拍重试（seq 不变，仍是最新请求）
                return Task::perform(
                    tokio::time::sleep(std::time::Duration::from_millis(PARAM_DEBOUNCE_MS)),
                    move |_| Message::Work(WorkMessage::RecomputeNow(seq)),
                );
            }
            return recompute(app);
        }
        // ── 画笔 / 橡皮擦松手后的异步合成回传 ──
        // 后台线程已完成 apply_mask + 去色晕 + 像素深拷贝，UI 线程在这里只做
        // 「换句柄」这一步；纹理上传由画布层的 Allocation 预上传机制兜底
        // （见 canvas_view.rs 的 ensure_image_alloc），当帧即可绘制、不闪跳。
        WorkMessage::ResultCommitted(Ok((seq, fg, display))) => {
            // 只采纳最新一次提交：连续涂抹会并发发起多次合成，序号落后的
            // 一律丢弃，避免旧蒙版的结果覆盖新蒙版（参考 RenderDone 防乱序）。
            if seq == app.work.refresh_seq {
                set_result_handles(&mut app.work, *display);
                app.work.result = Some(std::sync::Arc::new(*fg));
            }
            return Task::none();
        }
        WorkMessage::ResultCommitted(Err(e)) => {
            app.toast_err(trf("合成失败：{}", &[&e]));
            return Task::none();
        }

        /* ── 工具与参数 ── */
        WorkMessage::SetTool(t) => {
            app.work.tool = t;
            // 切到保留 / 擦除画笔时自动展开「笔刷属性」面板；切到其他工具时自动收起
            app.work.brush_props_open = matches!(t, Tool::Keep | Tool::Erase);
            if t != Tool::Crop {
                app.work.crop = None;
            } else if app.work.crop.is_none() {
                // 进入裁剪时给一个默认框（图像 80% 居中）
                if let (Some(src), (sw, sh)) = (&app.work.src, app.work.stage) {
                    let (iw, ih) = (src.width() as f32, src.height() as f32);
                    // 计算图片矩形的实际屏幕位置
                    let img_w = iw * app.work.zoom;
                    let img_h = ih * app.work.zoom;
                    let img_x = (sw - img_w) / 2.0 + app.work.pan.0;
                    let img_y = (sh - img_h) / 2.0 + app.work.pan.1;
                    
                    // 默认裁剪框为图片的80%，居中
                    let scale = 0.8_f32.min(1.0);
                    let crop_w = img_w * scale;
                    let crop_h = img_h * scale;
                    
                    app.work.crop = Some(Rect {
                        x: img_x + (img_w - crop_w) / 2.0,
                        y: img_y + (img_h - crop_h) / 2.0,
                        w: crop_w,
                        h: crop_h,
                    });
                }
            }
            return Task::none();
        }
        WorkMessage::SetParam(kind, v) => {
            match kind {
                ParamKind::Thresh => app.work.params.thresh = v,
                ParamKind::Edge => app.work.params.edge = v,
                ParamKind::Feather => app.work.params.feather = v,
                ParamKind::Morph => app.work.params.morph = v,
                ParamKind::Defringe => app.work.params.defringe = v,
                ParamKind::FillNeck => app.work.params.fill_neck = v,
            }
            app.persist();
            return if app.config.auto { request_recompute(app) } else { Task::none() };
        }
        WorkMessage::ToggleFill(v) => {
            app.work.params.fill = v;
            app.persist();
            return if app.config.auto { request_recompute(app) } else { Task::none() };
        }
        WorkMessage::ToggleShadow(v) => {
            app.work.params.shadow = v;
            app.persist();
            return Task::none();
        }
        WorkMessage::ToggleAuto(v) => {
            app.config.auto = v;
            app.persist();
            return Task::none();
        }
        WorkMessage::SetSampleMode(m) => {
            app.work.params.sample = m;
            app.persist();
            return if app.config.auto { recompute(app) } else { Task::none() };
        }
        WorkMessage::SetCustomHex(hex) => {
            if let Some(rgb) = parse_hex_rgb(&hex) {
                app.work.params.custom = rgb;
                app.work.custom_hex = hex;
                if app.config.auto {
                    return recompute(app);
                }
            } else {
                app.work.custom_hex = hex;
            }
            return Task::none();
        }
        WorkMessage::ToggleKeyAll(v) => {
            app.work.params.key_all = v;
            app.persist();
            return if app.config.auto { request_recompute(app) } else { Task::none() };
        }
        WorkMessage::SetModel(id) => {
            // 神经网络模型：校验仍在扫描清单里（文件可能已被删除/改名），
            // 并释放旧会话 —— 同名文件被替换后下次推理会重新加载新权重。
            if id.starts_with("nn:") && !app.work.nn_models.iter().any(|m| m.id == id) {
                app.toast_err(trf(
                    "模型不存在或已移出 models 目录：{}（点「重新扫描」刷新清单）",
                     &[id.trim_start_matches("nn:")],
                ));
                return Task::none();
            }
            crate::core::nn::release(&id);
            app.config.model = id;
            app.persist();
            return if app.config.auto && app.work.src.is_some() {
                recompute(app)
            } else {
                Task::none()
            };
        }
        WorkMessage::DoMatte => {
            if app.work.src.is_none() {
                app.toast(tr("请先导入图片"));
                return Task::none();
            }
            return recompute(app);
        }
        WorkMessage::RescanModels => {
            crate::core::nn::release_all(); // 目录内容可能已变化，全部会话作废
            let plugin_dir = app.config.plugin_dir.as_deref();
            let (dir, models) = crate::core::nn::scan(plugin_dir);
            app.work.models_dir = dir;
            app.work.nn_models = models;
            // 当前选中的神经网络模型若已不在清单里，回退本地智能抠图
            if app.config.model.starts_with("nn:")
                && !app.work.nn_models.iter().any(|m| m.id == app.config.model)
            {
                app.config.model = "local-smart".into();
                app.persist();
            }
            let n = app.work.nn_models.len();
            match n {
                0 => app.toast_err(trf(
                    "models 目录未发现 .onnx 模型（{}）",
                     &[app
                        .work
                        .models_dir
                        .as_ref()
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| tr("目录不存在").to_string())
                        .as_str()],
                )),
                _ => app.toast(trf("发现 {} 个神经网络模型", &[&n.to_string()])),
            }
            return Task::none();
        }

        /* ── 笔刷 ── */
        WorkMessage::SetBrushSize(v) => app.work.brush_size = v.clamp(4, 80),
        WorkMessage::SetBrushSizeStep(d) => app.work.brush_size = (app.work.brush_size + d).clamp(4, 80),
        WorkMessage::SetBrushHardness(v) => {
            let v = v.clamp(0, 100);
            if app.work.tool == Tool::Erase {
                app.work.hardness_erase = v;
            } else {
                app.work.hardness_keep = v;
            }
        }
        WorkMessage::SetBrushOpacity(v) => app.work.brush_opacity = v.clamp(1, 100),
        WorkMessage::SetBrushFlow(v) => app.work.brush_flow = v.clamp(1, 100),
        WorkMessage::CloseBrushProps => app.work.brush_props_open = false,

        /* ── 视图 ── */
        WorkMessage::SetViewMode(m) => {
            app.work.view_mode = m;
            // 进入对比即复位到中线（参考项目 setMode('compare') 语义）
            if m == ViewMode::Compare {
                app.work.compare_at = 0.5;
            }
            // 看图模式（对比/原图）下编辑类工具一律不可用：若正拿着，先切回智能抠图 ——
            // 否则用户面对一个刚变灰的高亮按钮，会以为是工具坏了（参考项目同款注释）。
            // 裁剪框的像素坐标系依附于「结果」视图，一并丢弃（参考项目 exitCropMode(false)）。
            // 若正在涂抹就切到看图模式，先把这一笔提交（异步），否则未提交的蒙版改动会丢失
            if !matches!(m, ViewMode::Result)
                && matches!(app.work.tool, Tool::Keep | Tool::Erase | Tool::Picker | Tool::Crop)
            {
                let mut commit_task = Task::none();
                if app.work.drag == Some(Drag::Paint) {
                    commit_task = refresh_result_async(app);
                }
                app.work.tool = Tool::Auto;
                app.work.crop = None;
                app.work.drag = None;
                app.work.drag_from = None;
                app.work.last_paint = None;
                return commit_task;
            }
        }
        WorkMessage::SetBgPreview(b) => app.work.bg_preview = b,
        WorkMessage::ZoomIn => app.work.zoom = (app.work.zoom * 1.25).min(8.0),
        WorkMessage::ZoomOut => app.work.zoom = (app.work.zoom / 1.25).max(0.05),
        // 滚轮缩放（参考 bind.js:473 applyZoom 的 0.9/1.1 步进）
        WorkMessage::ZoomStep(f) => app.work.zoom = (app.work.zoom * f).clamp(0.05, 8.0),
        WorkMessage::ZoomFit => fit_to_window(&mut app.work),

        /* ── 输出 ── */
        WorkMessage::SetOutColor(c) => {
            app.work.out_color = c;
            app.persist();
        }
        WorkMessage::SetFormat(f) => {
            app.config.export_format = match f {
                ExportFormat::Png => "png",
                ExportFormat::Jpg => "jpg",
                ExportFormat::Webp => "webp",
            }
            .to_string();
            app.persist();
        }
        WorkMessage::SetExportSize(s) => {
            app.config.export_size = s.to_u32();
            app.persist();
        }
        WorkMessage::Export => {
            if app.work.result.is_none() {
                app.toast(tr("还没有可导出的结果"));
                return Task::none();
            }
            // 处理中禁用：抠图未结束或已有导出在跑时忽略，防重复提交
            if app.work.busy || app.work.exporting {
                return Task::none();
            }
            let Some(result) = app.work.result.clone() else {
                return Task::none();
            };
            let bg = out_bg_rgb(&app.work.out_color);
            let size = app.config.export_size();
            let fmt = app.config.export_format();
            let name = img::export_name(&app.work.stem, fmt);
            app.work.export_seq += 1;
            let seq = app.work.export_seq;
            app.work.exporting = true;
            // G1：fit_export + flatten + PNG/WebP 编码整体搬离 UI 线程。
            // 大图这几步要数百毫秒到数秒，留在 update() 会冻结整个界面；
            // result 是 Arc，clone 零拷贝，不在 UI 线程复制 48MB 像素。
            return blocking_task(
                move || {
                    let t0 = std::time::Instant::now();
                    let fitted = img::fit_export(&result, size, bg);
                    let out = encode_in_memory(&fitted, fmt, bg)
                        .map(|bytes| Box::new((name, bytes)))
                        .map_err(|e| trf("编码失败：{}", &[&e]));
                    perf_log("fit_export+encode", t0);
                    out
                },
                move |r| {
                    Message::Work(WorkMessage::ExportPrepared(
                        seq,
                        match r {
                            Some(v) => v,
                            None => Err(tr("编码线程异常退出").to_string()),
                        },
                    ))
                },
            );
        }
        WorkMessage::ExportPrepared(seq, payload) => {
            // 换图 / 重复导出后旧结果作废
            if seq != app.work.export_seq {
                return Task::none();
            }
            app.work.exporting = false;
            let (name, bytes) = match payload {
                Ok(v) => *v,
                Err(e) => {
                    app.toast_err(e);
                    return Task::none();
                }
            };
            return Task::perform(pick_save(name, bytes), |p| {
                Message::Work(WorkMessage::Exported(match p {
                    Some(path) => Ok(path.to_string_lossy().to_string()),
                    None => Err(tr("已取消").to_string()),
                }))
            });
        }
        WorkMessage::Exported(Ok(path)) => {
            app.toast(trf("已导出 {}", &[&path]));
            return Task::none();
        }
        WorkMessage::Exported(Err(e)) => {
            if e != tr("已取消") {
                app.toast_err(trf("导出失败：{}", &[&e]));
            }
            return Task::none();
        }

        /* ── 破坏性编辑 ── */
        WorkMessage::ResetMask => {
            if let Some(base) = app.work.base.clone() {
                app.work.mask = Some(base);
                app.toast(tr("蒙版已重置"));
                // 重置后的整图合成放后台线程，UI 不被 O(宽×高) 的重算阻塞
                return refresh_result_async(app);
            }
            return Task::none();
        }
        WorkMessage::Undo => {
            if undo(app) {
                app.toast(tr("已撤销一步"));
                return refresh_result_async(app);
            }
            app.toast(tr("没有可撤销的操作"));
            return Task::none();
        }
        WorkMessage::RotateNinety(cw) => {
            let Some(src) = app.work.src.clone() else { return Task::none() };
            let mask = app.work.mask.clone();
            app.work.history.push(Snapshot {
                src: src.clone(),
                mask: mask.clone().unwrap_or_default(),
                mask_base: app.work.base.clone().unwrap_or_default(),
                rot_total: app.work.rot_total,
            });
            let rotated = std::sync::Arc::new(img::rotate90(&src, cw));
            app.work.src = Some(std::sync::Arc::clone(&rotated));
            *app.work.fields.lock().unwrap_or_else(|p| p.into_inner()) = matting::FieldCache::default();
            app.work.rot_total = (app.work.rot_total + if cw { 90.0 } else { -90.0 }) % 360.0;
            set_orig_handles(&mut app.work, &rotated);
            app.work.mask = None;
            app.work.base = None;
            app.work.result = None;
            app.work.result_handle = None;
            app.work.result_preview = None;
            fit_to_window(&mut app.work);
            return recompute(app);
        }
        WorkMessage::RotateCW => {
            return update(app, WorkMessage::RotateNinety(true));
        }
        WorkMessage::RotateCCW => {
            return update(app, WorkMessage::RotateNinety(false));
        }
        WorkMessage::SetRotateDeg(d) => app.work.rot_deg = d.clamp(-45.0, 45.0),
        WorkMessage::ApplyRotate => {
            let deg = app.work.rot_deg;
            if deg.abs() < 0.05 {
                return Task::none();
            }
            let Some(src) = app.work.src.clone() else { return Task::none() };
            // 破坏性编辑 → 先打快照
            app.work.history.push(Snapshot {
                src: src.clone(),
                mask: app.work.mask.clone().unwrap_or_default(),
                mask_base: app.work.base.clone().unwrap_or_default(),
                rot_total: app.work.rot_total,
            });
            let rotated = std::sync::Arc::new(img::rotate_deg(&src, deg));
            app.work.src = Some(std::sync::Arc::clone(&rotated));
            *app.work.fields.lock().unwrap_or_else(|p| p.into_inner()) = matting::FieldCache::default();
            app.work.rot_total += deg;
            app.work.rot_deg = 0.0;
            set_orig_handles(&mut app.work, &rotated);
            // 源图尺寸变了，蒙版必须重算
            app.work.mask = None;
            app.work.base = None;
            app.work.result = None;
            app.work.result_handle = None;
            app.work.result_preview = None;
            fit_to_window(&mut app.work);
            app.toast(trf("已旋转 {}°", &[&format!("{:.1}", deg)]));
            return recompute(app);
        }
        WorkMessage::SetCropRatio(r) => app.work.crop_ratio = r,
        WorkMessage::CropCancel => {
            app.work.crop = None;
            app.work.tool = Tool::Auto;
            return Task::none();
        }
        WorkMessage::CropConfirm => {
            return crop_confirm(app);
        }

        /* ── 画布交互 ── */
        WorkMessage::StagePress { screen, img: p, bounds } => {
            app.work.stage = bounds;
            // ── 对比模式：分割线热区拖拽优先于一切工具（参考项目 isOnCompareHandle
            //    语义：否则「选了移动工具就拖不动分割线」「画笔在分割线上乱涂」）。
            //    热区外是纯「看」—— 点哪跳哪会把分割线甩飞，参考项目已刻意移除；
            //    唯有移动工具照常平移（移动画布任何模式都可用）。
            if app.work.view_mode == ViewMode::Compare {
                // 分割线相对**图片矩形**定位（参考 moveHandle：按 viewport rect 取百分比）
                let dest = stage_dest(app, bounds);
                let split = dest.x + dest.width * app.work.compare_at;
                if (screen.x - split).abs() <= HANDLE_HIT {
                    app.work.drag = Some(Drag::Compare);
                    app.work.compare_at =
                        ((screen.x - dest.x) / dest.width.max(1.0)).clamp(0.0, 1.0);
                } else if app.work.tool == Tool::Pan {
                    app.work.drag = Some(Drag::Pan);
                    app.work.drag_from = Some(screen);
                }
                return Task::none();
            }
            match app.work.tool {
                Tool::Pan => {
                    app.work.drag = Some(Drag::Pan);
                    app.work.drag_from = Some(screen);
                }
                Tool::Keep | Tool::Erase => {
                    app.work.drag = Some(Drag::Paint);
                    paint_at(app, p, false);
                }
                Tool::Picker => {
                    pick_color(app, p);
                }
                Tool::Crop => {
                    // 已有裁剪框：控制点上 → 缩放；框内 → 整体拖动；框外 → 重新框选
                    // 先应用吸附功能：如果鼠标接近控制点，吸附到控制点
                    let snap_distance = 15.0;
                    let effective_screen = if let Some(c) = app.work.crop {
                        let r = c.normalize();
                        if r.w > 4.0 && r.h > 4.0 {
                            let handles = [
                                (r.x, r.y),             // 0: 左上
                                (r.x + r.w / 2.0, r.y), // 1: 上中
                                (r.x + r.w, r.y),       // 2: 右上
                                (r.x + r.w, r.y + r.h / 2.0), // 3: 右中
                                (r.x + r.w, r.y + r.h), // 4: 右下
                                (r.x + r.w / 2.0, r.y + r.h), // 5: 下中
                                (r.x, r.y + r.h),       // 6: 左下
                                (r.x, r.y + r.h / 2.0), // 7: 左中
                            ];
                            let mut closest = None;
                            let mut closest_dist = snap_distance;
                            for (i, (hx, hy)) in handles.iter().enumerate() {
                                let dist = ((screen.x - hx).powi(2) + (screen.y - hy).powi(2)).sqrt();
                                if dist < closest_dist {
                                    closest_dist = dist;
                                    closest = Some((i, *hx, *hy));
                                }
                            }
                            if let Some((_, hx, hy)) = closest {
                                iced::Point::new(hx, hy)
                            } else {
                                screen
                            }
                        } else {
                            screen
                        }
                    } else {
                        screen
                    };
                    
                    let hit = app
                        .work
                        .crop
                        .map(|c| c.normalize())
                        .filter(|c| c.w > 4.0 && c.h > 4.0 && c.contains(effective_screen));
                    
                    // 双击检测：在裁剪框内（非控制点）快速点击两次 → 确认裁剪
                    let is_on_handle = hit.as_ref().is_some_and(|c| c.hit_handle(effective_screen, 20.0).is_some());
                    let is_double_click = !is_on_handle && if let Some((last_time, last_pos)) = app.work.last_crop_click {
                        let elapsed = last_time.elapsed().as_millis();
                        let dist = ((screen.x - last_pos.x).powi(2) + (screen.y - last_pos.y).powi(2)).sqrt();
                        elapsed < 400 && dist < 10.0
                    } else {
                        false
                    };
                    
                    if is_double_click {
                        app.work.last_crop_click = None;
                        // 双击裁剪框内 → 确认裁剪
                        if let Some(c) = app.work.crop {
                            let r = c.normalize();
                            if r.w > 4.0 && r.h > 4.0 {
                                return crop_confirm(app);
                            }
                        }
                        return Task::none();
                    }
                    
                    // 记录本次点击，用于下次双击检测
                    app.work.last_crop_click = Some((std::time::Instant::now(), screen));
                    
                    match hit {
                        Some(c) => {
                            // 先检测是否在八个控制点上
                            if let Some(handle_idx) = c.hit_handle(effective_screen, 20.0) {
                                app.work.drag = Some(Drag::CropResize(handle_idx));
                                // 记录裁剪框初始状态
                                app.work.crop_initial = Some(c);
                            } else {
                                app.work.drag = Some(Drag::CropMove);
                                app.work.crop_initial = None;
                            }
                            app.work.drag_from = Some(effective_screen);
                        }
                        None => {
                            app.work.drag = Some(Drag::CropNew);
                            app.work.drag_from = Some(effective_screen);
                            // 不在按下时创建 0×0 裁剪框，等松手时判断是否有实际拖动
                            app.work.crop_initial = None;
                        }
                    }
                }
                Tool::Auto => {}
            }
            return Task::none();
        }
        WorkMessage::StageMove { screen, img: p, bounds } => {
            app.work.stage = bounds;
            // 拖分割线：分割线跟随指针（0..1），其余分支全部跳过
            if app.work.drag == Some(Drag::Compare) {
                let dest = stage_dest(app, bounds);
                app.work.compare_at =
                    ((screen.x - dest.x) / dest.width.max(1.0)).clamp(0.0, 1.0);
                return Task::none();
            }
            app.work.brush_cursor = if matches!(app.work.tool, Tool::Keep | Tool::Erase) {
                Some(screen)
            } else {
                None
            };
            app.work.picker_at = if app.work.tool == Tool::Picker { Some(screen) } else { None };
            
            // 裁剪工具吸附功能：当鼠标接近控制点时吸附到控制点
            let snap_distance = 15.0; // 吸附距离（像素）
            let effective_screen = if app.work.tool == Tool::Crop && app.work.drag.is_none() {
                if let Some(c) = app.work.crop {
                    let r = c.normalize();
                    if r.w > 4.0 && r.h > 4.0 {
                        // 检测是否接近八个控制点
                        let handles = [
                            (r.x, r.y),             // 0: 左上
                            (r.x + r.w / 2.0, r.y), // 1: 上中
                            (r.x + r.w, r.y),       // 2: 右上
                            (r.x + r.w, r.y + r.h / 2.0), // 3: 右中
                            (r.x + r.w, r.y + r.h), // 4: 右下
                            (r.x + r.w / 2.0, r.y + r.h), // 5: 下中
                            (r.x, r.y + r.h),       // 6: 左下
                            (r.x, r.y + r.h / 2.0), // 7: 左中
                        ];
                        let mut closest = None;
                        let mut closest_dist = snap_distance;
                        for (i, (hx, hy)) in handles.iter().enumerate() {
                            let dist = ((screen.x - hx).powi(2) + (screen.y - hy).powi(2)).sqrt();
                            if dist < closest_dist {
                                closest_dist = dist;
                                closest = Some((i, *hx, *hy));
                            }
                        }
                        // 如果接近某个控制点，吸附到该点
                        if let Some((_, hx, hy)) = closest {
                            iced::Point::new(hx, hy)
                        } else {
                            screen
                        }
                    } else {
                        screen
                    }
                } else {
                    screen
                }
            } else {
                screen
            };
            
            // 更新鼠标光标显示（用于裁剪工具的控制点悬停提示）
            if app.work.tool == Tool::Crop && app.work.drag.is_none() {
                if let Some(c) = app.work.crop {
                    let r = c.normalize();
                    if r.w > 4.0 && r.h > 4.0 && r.contains(effective_screen) {
                        // 检查是否在控制点上
                        if r.hit_handle(effective_screen, 20.0).is_some() {
                            app.work.brush_cursor = Some(effective_screen);
                        }
                    }
                }
            }
            
            match app.work.drag {
                Some(Drag::Pan) => {
                    if let Some(from) = app.work.drag_from {
                        app.work.pan.0 += screen.x - from.x;
                        app.work.pan.1 += screen.y - from.y;
                        app.work.drag_from = Some(screen);
                        clamp_pan(&mut app.work);
                    }
                }
                Some(Drag::Paint) => paint_at(app, p, true),
                Some(Drag::CropMove) => {
                    if let Some(from) = app.work.drag_from {
                        let dest = stage_dest(app, app.work.stage);
                        if let Some(c) = app.work.crop.as_mut() {
                            c.x += screen.x - from.x;
                            c.y += screen.y - from.y;
                            *c = clamp_crop_to_image(*c, dest);
                        }
                        app.work.drag_from = Some(screen);
                    }
                }
                Some(Drag::CropResize(handle_idx)) => {
                    if let Some(from) = app.work.drag_from {
                        let dest = stage_dest(app, app.work.stage);
                        // 使用初始状态作为计算基准
                        let initial = app.work.crop_initial.or(app.work.crop).map(|c| c.normalize());
                        if let (Some(r), Some(c)) = (initial, app.work.crop.as_mut()) {
                            let dx = screen.x - from.x;
                            let dy = screen.y - from.y;
                            
                            // 根据控制点编号决定如何调整裁剪框
                            // 0=左上, 1=上中, 2=右上, 3=右中, 4=右下, 5=下中, 6=左下, 7=左中
                            let mut new_x = r.x;
                            let mut new_y = r.y;
                            let mut new_w = r.w;
                            let mut new_h = r.h;
                            
                            match handle_idx {
                                0 => { // 左上：调整x, y, w, h
                                    new_x = r.x + dx;
                                    new_y = r.y + dy;
                                    new_w = r.w - dx;
                                    new_h = r.h - dy;
                                }
                                1 => { // 上中：调整y, h
                                    new_y = r.y + dy;
                                    new_h = r.h - dy;
                                }
                                2 => { // 右上：调整y, w, h
                                    new_y = r.y + dy;
                                    new_w = r.w + dx;
                                    new_h = r.h - dy;
                                }
                                3 => { // 右中：调整w
                                    new_w = r.w + dx;
                                }
                                4 => { // 右下：调整w, h
                                    new_w = r.w + dx;
                                    new_h = r.h + dy;
                                }
                                5 => { // 下中：调整h
                                    new_h = r.h + dy;
                                }
                                6 => { // 左下：调整x, w, h
                                    new_x = r.x + dx;
                                    new_w = r.w - dx;
                                    new_h = r.h + dy;
                                }
                                7 => { // 左中：调整x, w
                                    new_x = r.x + dx;
                                    new_w = r.w - dx;
                                }
                                _ => {}
                            }
                            
                            // 确保最小尺寸
                            new_w = new_w.max(8.0);
                            new_h = new_h.max(8.0);
                            
                            // 应用宽高比约束（在边界限制之前）
                            if let Some(ratio) = app.work.crop_ratio.ratio((new_w.max(1.0), new_h.max(1.0))) {
                                // 根据拖动方向决定以哪个维度为准
                                match handle_idx {
                                    0 | 6 => { // 左侧控制点：以宽度为准，调整高度
                                        new_h = new_w / ratio;
                                        if handle_idx == 0 {
                                            new_y = r.y + r.h - new_h;
                                        }
                                    }
                                    2 | 4 => { // 右侧控制点：以宽度为准，调整高度
                                        new_h = new_w / ratio;
                                    }
                                    1 => { // 上中：以宽度为准，调整高度
                                        new_h = new_w / ratio;
                                        new_y = r.y + r.h - new_h;
                                    }
                                    5 => { // 下中：以宽度为准，调整高度
                                        new_h = new_w / ratio;
                                    }
                                    3 | 7 => { // 左右中：以高度为准，调整宽度
                                        new_w = new_h * ratio;
                                        if handle_idx == 7 {
                                            new_x = r.x + r.w - new_w;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            
                            c.x = new_x;
                            c.y = new_y;
                            c.w = new_w;
                            c.h = new_h;
                            // 限制裁剪框在图片矩形内
                            *c = clamp_crop_to_image(*c, dest);
                        }
                    }
                }
                Some(Drag::CropNew) => {
                    if let Some(from) = app.work.drag_from {
                        let dest = stage_dest(app, app.work.stage);
                        let nx = screen.x - from.x;
                        let mut ny = screen.y - from.y;
                        // 如果还没有裁剪框，创建一个
                        if app.work.crop.is_none() {
                            app.work.crop = Some(Rect { x: from.x, y: from.y, w: 0.0, h: 0.0 });
                        }
                        if let Some(c) = app.work.crop.as_mut() {
                            if let Some(r) = app.work.crop_ratio.ratio((nx.abs().max(1.0), ny.abs().max(1.0))) {
                                ny = nx / r;
                            }
                            c.w = nx;
                            c.h = ny;
                            *c = clamp_crop_to_image(*c, dest);
                        }
                    }
                }
                _ => {}
            }
            return Task::none();
        }
        WorkMessage::StageRelease => {
            let was_paint = app.work.drag == Some(Drag::Paint);
            let was_crop_new = app.work.drag == Some(Drag::CropNew);
            app.work.drag = None;
            app.work.drag_from = None;
            app.work.crop_initial = None;
            app.work.last_paint = None;
            // CropNew：松手时判断是否有实际拖动，有拖动才创建裁剪框
            if was_crop_new {
                // 拖动距离不足 5px 视为点击，不创建裁剪框
                if let Some(c) = app.work.crop {
                    let r = c.normalize();
                    if r.w < 5.0 || r.h < 5.0 {
                        app.work.crop = None;
                    } else {
                        app.work.crop = Some(r);
                    }
                }
            } else if let Some(c) = app.work.crop.as_mut() {
                *c = c.normalize();
            }
            if was_paint {
                return refresh_result_async(app);
            }
            return Task::none();
        }
        WorkMessage::StageLeave => {
            app.work.brush_cursor = None;
            app.work.picker_at = None;
            return Task::none();
        }

        /* ── 批量 ── */
        WorkMessage::BatchAdd => return update(app, WorkMessage::OpenBatch),
        WorkMessage::BatchRun => {
            if app.work.batch.is_empty() {
                app.toast_err(tr("批量队列为空"));
                return Task::none();
            }
            if app.work.batch_running {
                return Task::none();
            }
            // 选输出目录（记住上次目录作为起点体验更顺，rfd 默认即可）
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                |p| Message::Work(WorkMessage::BatchFolderPicked(p)),
            );
        }
        WorkMessage::BatchFolderPicked(Some(dir)) => {
            let s = &mut app.work;
            let jobs: Vec<(usize, PathBuf)> = s
                .batch
                .iter()
                .enumerate()
                .filter(|(_, it)| it.status == crate::app::QueueStatus::Pending)
                .map(|(i, it)| (i, it.path.clone()))
                .collect();
            if jobs.is_empty() {
                app.toast(tr("没有待处理的任务"));
                return Task::none();
            }
            s.batch_running = true;
            s.batch_out_dir = Some(dir.clone());
            let params = s.params.clone();
            let out = app.config.export_format();
            let size = app.config.export_size();
            let bg = out_bg_rgb(&s.out_color);
            let nn_info = nn_info_for(app, &app.config.model);
            return batch_export_stream(jobs, params, out, size, bg, nn_info, dir);
        }
        WorkMessage::BatchFolderPicked(None) => return Task::none(),
        WorkMessage::BatchEvent(ev) => {
            let s = &mut app.work;
            match ev {
                crate::message::WorkBatchEvent::Start { idx } => {
                    if let Some(it) = s.batch.get_mut(idx) {
                        it.status = crate::app::QueueStatus::Running;
                    }
                }
                crate::message::WorkBatchEvent::Done { idx, .. } => {
                    if let Some(it) = s.batch.get_mut(idx) {
                        it.status = crate::app::QueueStatus::Done;
                        it.err = None;
                    }
                }
                crate::message::WorkBatchEvent::Fail { idx, err } => {
                    if let Some(it) = s.batch.get_mut(idx) {
                        it.status = crate::app::QueueStatus::Failed;
                        it.err = Some(err);
                    }
                }
                crate::message::WorkBatchEvent::Finished { ok, fail } => {
                    s.batch_running = false;
                    if fail == 0 {
                        app.toast(trf("批量完成 {} 张", &[&ok.to_string()]));
                    } else {
                        app.toast_err(trf(
                            "批量结束：成功 {} · 失败 {}（可点「重试失败项」）",
                            &[&ok.to_string(), &fail.to_string()],
                        ));
                    }
                    return crate::app::schedule_session_sweep();
                }
            }
            return Task::none();
        }
        WorkMessage::BatchRemove(i) => {
            let s = &mut app.work;
            if s.batch_running {
                app.toast_err(tr("批量处理中，不能移除"));
                return Task::none();
            }
            if i < s.batch.len() {
                s.batch.remove(i);
            }
            return Task::none();
        }
        WorkMessage::BatchClear => {
            let s = &mut app.work;
            if s.batch_running {
                app.toast_err(tr("批量处理中，不能清空"));
                return Task::none();
            }
            s.batch.clear();
            return Task::none();
        }
        WorkMessage::BatchRetryFailed => {
            let s = &mut app.work;
            if s.batch_running {
                return Task::none();
            }
            let mut any = false;
            for it in &mut s.batch {
                if it.status == crate::app::QueueStatus::Failed {
                    it.status = crate::app::QueueStatus::Pending;
                    it.err = None;
                    any = true;
                }
            }
            if !any {
                app.toast(tr("没有失败项可重试"));
                return Task::none();
            }
            app.toast(tr("失败项已重置，请选择输出目录"));
            return update(app, WorkMessage::BatchRun);
        }
    }
    Task::none()
}

/* ═════════════════ 内部逻辑 ═════════════════ */

fn to_payload(l: LoadedImage) -> LoadedPayload {
    LoadedPayload { rgba: l.rgba, path: l.path, stem: l.stem, display: None }
}

fn parse_hex_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// 输出背景色：transparent / checker → None（保留透明），否则解析 HEX
fn out_bg_rgb(c: &str) -> Option<[u8; 3]> {
    match c {
        "transparent" | "checker" => None,
        _ => parse_hex_rgb(c),
    }
}

/// 由模型 id 找到扫描到的神经网络模型信息；色键预设（local-*）返回 None
fn nn_info_for(app: &App, id: &str) -> Option<crate::core::nn::NnModelInfo> {
    id.starts_with("nn:")
        .then(|| app.work.nn_models.iter().find(|m| m.id == id).cloned())
        .flatten()
}

/// 高频参数滑块的防抖重算：拖动期间只更新状态值，停顿 150ms 才真正重算。
/// 与 recompute 的取消令牌配合：堆积的防抖任务到期后，旧后台线程已被新 seq 取消。
const PARAM_DEBOUNCE_MS: u64 = 150;

fn request_recompute(app: &mut App) -> Task<Message> {
    if app.work.src.is_none() {
        return Task::none();
    }
    app.work.debounce_seq += 1;
    let seq = app.work.debounce_seq;
    Task::perform(
        tokio::time::sleep(std::time::Duration::from_millis(PARAM_DEBOUNCE_MS)),
        move |_| Message::Work(WorkMessage::RecomputeNow(seq)),
    )
}

/// 启动一次蒙版计算（后台线程，UI 不阻塞）。
///
/// 路由规则：`config.model` 为 `nn:<文件名>` 时走 ONNX 神经网络推理，
/// 否则走 SmartMatte 色键算法 —— 两条路径产出同一形状的 MaskPayload，
/// 画笔 / 撤销 / 导出等下游完全复用。
fn recompute(app: &mut App) -> Task<Message> {
    let Some(src) = app.work.src.clone() else {
        return Task::none();
    };
    // 视口尺寸在调度时固定，后台线程据此生成显示金字塔（不占 UI 线程）
    let viewport = viewport_long(&app.work);
    // 序列号 + 取消令牌：高频拖动滑块时新请求使旧后台线程提前退出
    app.work.recompute_seq += 1;
    let seq = app.work.recompute_seq;
    let _ = app.work.matte_cancel_tx.send(seq);
    let cancel_rx = app.work.matte_cancel_rx.clone();
    app.work.busy = true;
    let params = app.work.params.clone();
    let defringe = params.defringe;
    let model_id = app.config.model.clone();
    let nn_info = nn_info_for(app, &model_id);
    let last_bg = app.work.last_bg;
    let fields = app.work.fields.clone();
    blocking_task(
        move || {
            let job_t0 = std::time::Instant::now();
            if *cancel_rx.borrow() > seq {
                return Err(tr("已取消").to_string());
            }
            let (w, h) = (src.width() as usize, src.height() as usize);
            if let Some(info) = nn_info {
                // ── 神经网络路径：软蒙版（毛发 / 半透明边缘以连续 alpha 保留） ──
                let t0 = std::time::Instant::now();
                // 推理闸门：大模型一次 CPU 推理数十秒、激活内存以 GB 计，且
                // ORT run 无法中途取消——自动重抠期间若放行并发，多个 1024²
                // 推理叠加会把内存顶到 10GB+ 直到系统 OOM（进程闪退）。
                // 这里只有最新一次请求能通过闸门；过期的在门口凭取消令牌自行退出。
                const NN_GATE_POLL_MS: u64 = 120;
                // RAII 守卫：占闸直到本作用域结束（含 panic 展开），不会泄漏
                let _gate = loop {
                    if *cancel_rx.borrow() > seq {
                        return Err(tr("已取消").to_string());
                    }
                    if let Some(g) = crate::core::nn::try_claim_inference() {
                        break g;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(NN_GATE_POLL_MS));
                };
                // panic 兜底：任何一处索引/解包异常都降级为错误消息，而不是
                // 让 panic 沿 tokio 阻塞线程炸掉整个进程。
                let mask = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    crate::core::nn::matte(&info, src.as_raw(), w, h)
                }));
                let mask = match mask {
                    Ok(r) => r?,
                    Err(_) => {
                        return Err(tr("推理线程异常退出（模型可能不兼容当前图片）").to_string())
                    }
                };
                let fg = img::apply_mask(&src, &mask);
                // base = 当前蒙版快照：画笔「恢复原始」行为与色键路径一致
                let fg_count = mask.iter().filter(|&&v| v > 127).count();
                perf_log("nn_matte+display", job_t0);
                return Ok(Box::new(MaskPayload {
                    base: mask.clone(),
                    mask,
                    display: Box::new(pyramid::build(&fg, viewport)),
                    result: fg,
                    fg_count,
                    holes: 0,
                    elapsed_ms: t0.elapsed().as_millis() as f32,
                    bg: last_bg,
                }));
            }
            // ── 色键路径：SmartMatte v2（grad/dist 场跨参数复用，滑块拖动不再全量重算） ──
            if *cancel_rx.borrow() > seq {
                return Err(tr("已取消").to_string());
            }
            let mut cache = fields.lock().unwrap_or_else(|p| p.into_inner());
            // G4：把取消令牌传进计算循环（连通域 BFS / 羽化 / 形态学各阶段
            // 都会检查），长任务不再「只能等跑完」，换图或拖滑块能真正中止。
            let res: MaskResult = matting::compute_mask_cancellable(
                src.as_raw(),
                w,
                h,
                &params,
                &mut cache,
                &|| *cancel_rx.borrow() > seq,
            )
            .ok_or_else(|| tr("已取消").to_string())?;
            let mut fg = img::apply_mask(&src, &res.mask);
            defringe_img(&mut fg, &res.mask, w, h, defringe);
            perf_log("matte+display", job_t0);
            Ok(Box::new(MaskPayload {
                mask: res.mask,
                base: res.base,
                display: Box::new(pyramid::build(&fg, viewport)),
                result: fg,
                fg_count: res.fg_count,
                holes: res.holes_filled,
                elapsed_ms: res.elapsed_ms,
                bg: res.bg,
            }))
        },
        |r: Option<Result<Box<MaskPayload>, String>>| {
            Message::Work(WorkMessage::MaskDone(match r {
                Some(Ok(v)) => Ok(v),
                Some(Err(e)) => Err(e),
                // 取消不报错：高频拖动滑块时旧请求静默作废
                None => Err(tr("已取消").to_string()),
            }))
        },
    )
}

/// image 0.25 的 ImageBuffer 没有 as_raw_mut，这里取走缓冲再装回
fn defringe_img(fg: &mut image::RgbaImage, mask: &[u8], w: usize, h: usize, strength: i32) {
    if strength <= 0 {
        return;
    }
    let mut raw = std::mem::replace(fg, image::RgbaImage::new(1, 1)).into_raw();
    matting::defringe(&mut raw, mask, w, h, strength);
    if let Some(back) = image::RgbaImage::from_raw(w as u32, h as u32, raw) {
        *fg = back;
    }
}

/// mask → result 的**异步**合成（与 `recompute` 同一条后台线程链路）。
///
/// 把 O(宽×高) 的 apply_mask + 去色晕 + 16MB 级像素深拷贝整体搬离 UI 线程：
/// UI 线程只负责「接到结果 → 换句柄 → 下一帧绘制」，涂抹松手 / 撤销 / 重置蒙版时
/// 读取与渲染互不阻塞，不再出现整帧停顿。
///
/// `seq` 序列号防乱序：连续涂抹会并发发起多次提交，只有序号与当前
/// `WorkState.refresh_seq` 一致的那次才被 `ResultCommitted` 采纳。
fn refresh_result_async(app: &mut App) -> Task<Message> {
    let (Some(src), Some(mask)) = (app.work.src.clone(), app.work.mask.clone()) else {
        return Task::none();
    };
    let defringe = app.work.params.defringe;
    let viewport = viewport_long(&app.work);
    app.work.refresh_seq += 1;
    let seq = app.work.refresh_seq;
    blocking_task(
        move || {
            let job_t0 = std::time::Instant::now();
            let mut fg = img::apply_mask(&src, &mask);
            defringe_img(
                &mut fg,
                &mask,
                src.width() as usize,
                src.height() as usize,
                defringe,
            );
            // 显示金字塔与合成同批在后台生成：UI 线程只把字节包成 Handle
            let display = Box::new(pyramid::build(&fg, viewport));
            perf_log("composite+pyramid", job_t0);
            (fg, display)
        },
        move |r| {
            Message::Work(WorkMessage::ResultCommitted(match r {
                Some((fg, display)) => Ok((seq, Box::new(fg), display)),
                None => Err(tr("合成线程异常退出").to_string()),
            }))
        },
    )
}

/// 画笔落点：keep → 255，erase → 0，按硬度做软边
fn paint_at(app: &mut App, p: (f32, f32), cont: bool) {
    let w = &mut app.work;
    let (Some(src), Some(mask)) = (w.src.as_ref(), w.mask.as_mut()) else { return };
    let (iw, ih) = (src.width() as i64, src.height() as i64);
    let r = w.brush_size as f32;
    let hardness = if w.tool == Tool::Erase { w.hardness_erase } else { w.hardness_keep } as f32
        / 100.0;
    let target: u8 = if w.tool == Tool::Erase { 0 } else { 255 };
    let flow = w.brush_flow as f32 / 100.0;

    // 连续拖动时按上次落点插值补点，避免快速涂抹断线
    let from = if cont { w.last_paint } else { None };
    let steps = match from {
        Some(f) => {
            let d = ((p.0 - f.0).powi(2) + (p.1 - f.1).powi(2)).sqrt();
            (d / (r * 0.25).max(1.0)).ceil().clamp(1.0, 64.0) as i32
        }
        None => 1,
    };
    for s in 0..steps {
        let t = if steps <= 1 {
            0.0
        } else {
            s as f32 / (steps - 1) as f32
        };
        let cx = match from {
            Some(f) => f.0 + (p.0 - f.0) * t,
            None => p.0,
        };
        let cy = match from {
            Some(f) => f.1 + (p.1 - f.1) * t,
            None => p.1,
        };
        let x0 = (cx - r).floor() as i64;
        let x1 = (cx + r).ceil() as i64;
        let y0 = (cy - r).floor() as i64;
        let y1 = (cy + r).ceil() as i64;
        for y in y0.max(0)..=y1.min(ih - 1) {
            for x in x0.max(0)..=x1.min(iw - 1) {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if d > r {
                    continue;
                }
                // 硬度：核心实心，外圈线性衰减
                let inner = r * hardness;
                let a = if d <= inner {
                    1.0
                } else {
                    1.0 - (d - inner) / (r - inner).max(0.001)
                };
                let idx = (y as usize) * iw as usize + x as usize;
                let cur = mask[idx] as f32;
                let next = cur + (target as f32 - cur) * a * flow;
                mask[idx] = next.clamp(0.0, 255.0) as u8;
            }
        }
    }
    w.last_paint = Some(p);
    // ⚠️ 性能关键：涂抹过程中**不再**每移动一次就全图重算 + 新建 GPU 纹理
    // （原实现每帧 O(宽×高) 的 apply_mask + 16MB 上传，就是卡顿 + 闪跳的根因）。
    // 蒙版只在笔刷覆盖的小块里增量更新；整图合成与纹理上传推迟到松手时一次性提交
    // （见 `StageRelease` / `SetViewMode` 里的 `refresh_result_async`）。
}

/// 吸管：取图像坐标处的颜色作为自定义取样色
fn pick_color(app: &mut App, p: (f32, f32)) {
    let Some(src) = app.work.src.as_ref() else { return };
    let x = p.0.round() as i64;
    let y = p.1.round() as i64;
    if x < 0 || y < 0 || x >= src.width() as i64 || y >= src.height() as i64 {
        return;
    }
    let px = src.get_pixel(x as u32, y as u32).0;
    app.work.params.custom = [px[0], px[1], px[2]];
    app.work.params.sample = SampleMode::Custom;
    app.work.custom_hex = format!("#{:02X}{:02X}{:02X}", px[0], px[1], px[2]);
    app.persist();
    app.toast(trf("已取样 {}", &[&app.work.custom_hex]));
}

fn fit_to_window(w: &mut WorkState) {
    let Some(src) = w.src.as_ref() else { return };
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    // 参考项目 fitToWindow 的 pad = 56（画布 20px padding + 视觉余量）
    let (sw, sh) = (w.stage.0 - 56.0, w.stage.1 - 56.0);
    if iw <= 0.0 || ih <= 0.0 || sw <= 0.0 || sh <= 0.0 {
        return;
    }
    let z = (sw / iw).min(sh / ih).min(1.0);
    w.zoom = z;
    w.fit_zoom = z;
    w.pan = (0.0, 0.0);
}

/// 裁剪框边界限制：确保裁剪框至少部分在图片矩形内
/// 返回限制后的裁剪框（屏幕坐标）
fn clamp_crop_to_image(crop: Rect, dest: Rectangle) -> Rect {
    let min_size = 8.0;
    let mut r = crop.normalize();
    
    // 确保裁剪框至少有一个最小尺寸
    if r.w < min_size {
        r.w = min_size;
    }
    if r.h < min_size {
        r.h = min_size;
    }
    
    // 限制裁剪框不能超出图片矩形太多（最多允许50px的溢出）
    let margin = 50.0;
    let left = dest.x - margin;
    let top = dest.y - margin;
    let right = dest.x + dest.width + margin;
    let bottom = dest.y + dest.height + margin;
    
    // 限制左上角
    r.x = r.x.max(left);
    r.y = r.y.max(top);
    
    // 限制右下角
    if r.x + r.w > right {
        r.w = right - r.x;
    }
    if r.y + r.h > bottom {
        r.h = bottom - r.y;
    }
    
    // 确保最终尺寸仍然有效
    r.w = r.w.max(min_size);
    r.h = r.h.max(min_size);
    
    r
}

/// 平移边界收敛（参考项目 view.js::clampPan）：
/// 图比视口大 → 最多拖到只剩 15% 可见；图比视口小 → 留 40% 容器余量，
/// 保证图片永远有一部分留在画面里，也保证小图「能拖得动」。
fn clamp_pan(w: &mut WorkState) {
    let Some(src) = w.src.as_ref() else { return };
    let (iw, ih) = (src.width() as f32 * w.zoom, src.height() as f32 * w.zoom);
    let (wrap_w, wrap_h) = (w.stage.0 - 20.0, w.stage.1 - 20.0);
    let slack_x = if iw <= wrap_w {
        wrap_w * 0.4
    } else {
        ((iw - wrap_w * 0.15) / 2.0).max(0.0)
    };
    let slack_y = if ih <= wrap_h {
        wrap_h * 0.4
    } else {
        ((ih - wrap_h * 0.15) / 2.0).max(0.0)
    };
    w.pan.0 = w.pan.0.clamp(-slack_x, slack_x);
    w.pan.1 = w.pan.1.clamp(-slack_y, slack_y);
}

/// 裁剪确认：把屏幕矩形映射到图像坐标并裁切（破坏性，进撤销栈）
fn crop_confirm(app: &mut App) -> Task<Message> {
    let w = &mut app.work;
    let (Some(src), Some(crop)) = (w.src.clone(), w.crop) else {
        return Task::none();
    };
    let c = crop.normalize();
    let (iw, ih) = (src.width() as f32, src.height() as f32);
    let (sw, sh) = w.stage;
    let ox = (sw - iw * w.zoom) / 2.0 + w.pan.0;
    let oy = (sh - ih * w.zoom) / 2.0 + w.pan.1;
    let x = ((c.x - ox) / w.zoom).round().max(0.0) as u32;
    let y = ((c.y - oy) / w.zoom).round().max(0.0) as u32;
    let cw = (c.w / w.zoom).round() as u32;
    let ch = (c.h / w.zoom).round() as u32;
    if cw == 0 || ch == 0 || x >= src.width() || y >= src.height() {
        app.toast_err(tr("裁剪框无效"));
        return Task::none();
    }
    w.history.push(Snapshot {
        src: src.clone(),
        mask: w.mask.clone().unwrap_or_default(),
        mask_base: w.base.clone().unwrap_or_default(),
        rot_total: w.rot_total,
    });
    let cut = img::crop(&src, x, y, cw, ch);
    let (nw, nh) = (cut.width(), cut.height());
    w.src = Some(std::sync::Arc::new(cut.clone()));
    *w.fields.lock().unwrap_or_else(|p| p.into_inner()) = matting::FieldCache::default();
    set_orig_handles(w, &cut);
    w.mask = None;
    w.base = None;
    w.result = None;
    w.result_handle = None;
    w.result_preview = None;
    w.crop = None;
    w.tool = Tool::Auto;
    w.pan = (0.0, 0.0);
    fit_to_window(w);
    app.toast(trf("已裁剪为 {}×{}，正在重新抠图...", &[&nw.to_string(), &nh.to_string()]));
    recompute(app)
}

/// 撤销一步（只回滚状态；结果图的重算由调用方走 `refresh_result_async` 异步完成）
fn undo(app: &mut App) -> bool {
    let Some(snap) = app.work.history.pop() else { return false };
    app.work.src = Some(snap.src.clone());
    set_orig_handles(&mut app.work, &snap.src);
    app.work.mask = Some(snap.mask.clone());
    app.work.base = Some(snap.mask_base.clone());
    app.work.rot_total = snap.rot_total;
    true
}

/// 编码到内存（导出 / 批量共用）
fn encode_in_memory(
    fg: &image::RgbaImage,
    fmt: ExportFormat,
    bg: Option<[u8; 3]>,
) -> Result<Vec<u8>, String> {
    let bg = if fmt == ExportFormat::Jpg { Some([255, 255, 255]) } else { bg };
    let flat = img::flatten(fg, bg);
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(flat)
        .write_to(&mut buf, fmt_image_format(fmt))
        .map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

fn fmt_image_format(f: ExportFormat) -> image::ImageFormat {
    match f {
        ExportFormat::Png => image::ImageFormat::Png,
        ExportFormat::Jpg => image::ImageFormat::Jpeg,
        ExportFormat::Webp => image::ImageFormat::WebP,
    }
}

/// 批量：顺序解码 → 抠图 → 编码，写到源目录旁的 out/ 子目录。
///
/// `nn_info` 为 Some 时全部图片走神经网络推理 —— 会话跨图片复用（模型只
/// 加载一次），这是批量场景下最大的性能收益。
/// 单张批量项：抠图 → 导出尺寸 → 编码 → 写盘（返回输出文件路径）
fn run_batch_one(
    loaded: &img::LoadedImage,
    params: &MatteParams,
    fmt: ExportFormat,
    size: ExportSize,
    bg: Option<[u8; 3]>,
    nn_info: Option<&crate::core::nn::NnModelInfo>,
    out_dir: &std::path::Path,
) -> Result<String, String> {
    let (w, h) = (loaded.rgba.width() as usize, loaded.rgba.height() as usize);
    let mask = if let Some(info) = nn_info {
        crate::core::nn::matte(info, loaded.rgba.as_raw(), w, h)
            .map_err(|e| format!("{}：{e}", loaded.stem))?
    } else {
        let mut cache = matting::FieldCache::default();
        matting::compute_mask(loaded.rgba.as_raw(), w, h, params, &mut cache).mask
    };
    let mut fg = img::apply_mask(&loaded.rgba, &mask);
    if nn_info.is_none() {
        // 色键路径做去色晕；神经网络路径的软边本身就是抠图结果，不做
        let mut raw = std::mem::replace(&mut fg, image::RgbaImage::new(1, 1)).into_raw();
        matting::defringe(&mut raw, &mask, w, h, params.defringe);
        if let Some(back) = image::RgbaImage::from_raw(w as u32, h as u32, raw) {
            fg = back;
        }
    }
    let fitted = img::fit_export(&fg, size, bg);
    let bytes = encode_in_memory(&fitted, fmt, bg).map_err(|e| format!("{}：{e}", loaded.stem))?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let name = img::export_name(&loaded.stem, fmt);
    let out_path = out_dir.join(name);
    std::fs::write(&out_path, bytes).map_err(|e| e.to_string())?;
    Ok(out_path.display().to_string())
}

/// 批量流水线：解码线程（第 N+1 张）∥ 处理线程（第 N 张），
/// 进度经 Task::stream 逐张回传；单张失败不中断整批（与证件照页同模式）。
fn batch_export_stream(
    jobs: Vec<(usize, PathBuf)>,
    params: MatteParams,
    fmt: ExportFormat,
    size: ExportSize,
    bg: Option<[u8; 3]>,
    nn_info: Option<crate::core::nn::NnModelInfo>,
    out_dir: PathBuf,
) -> Task<Message> {
    Task::stream(iced::stream::channel(16, async move |mut out| {
        use iced::futures::SinkExt;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<crate::message::WorkBatchEvent>();
        // 解码 → 处理 的内部通道
        let (work_tx, mut work_rx) =
            tokio::sync::mpsc::unbounded_channel::<
                Result<(usize, img::LoadedImage), (usize, String)>,
            >();
        // 解码线程：逐张读图（预取），处理线程抠图时它已ahead解码下一张
        std::thread::spawn(move || {
            for (idx, path) in jobs {
                match img::load_file(&path) {
                    Ok(l) => {
                        let _ = work_tx.send(Ok((idx, l)));
                    }
                    Err(e) => {
                        let _ = work_tx.send(Err((idx, format!("{}：{e}", path.display()))));
                    }
                }
            }
            drop(work_tx); // 解码结束，处理线程自然收尾
        });
        // 处理线程：顺序推理（GPU 会话内部串行）+ 编码 + 写盘 + 事件回传
        std::thread::spawn(move || {
            let mut ok = 0usize;
            let mut fail = 0usize;
            while let Some(item) = work_rx.blocking_recv() {
                match item {
                    Ok((idx, loaded)) => {
                        let _ = tx.send(crate::message::WorkBatchEvent::Start { idx });
                        match run_batch_one(&loaded, &params, fmt, size, bg, nn_info.as_ref(), &out_dir) {
                            Ok(path) => {
                                ok += 1;
                                let _ = tx.send(crate::message::WorkBatchEvent::Done { idx, path });
                            }
                            Err(e) => {
                                fail += 1;
                                let _ = tx.send(crate::message::WorkBatchEvent::Fail { idx, err: e });
                            }
                        }
                    }
                    Err((idx, e)) => {
                        fail += 1;
                        let _ = tx.send(crate::message::WorkBatchEvent::Fail { idx, err: e });
                    }
                }
            }
            let _ = tx.send(crate::message::WorkBatchEvent::Finished { ok, fail });
        });
        // 转发事件到 UI
        while let Some(ev) = rx.recv().await {
            let _ = out.send(Message::Work(WorkMessage::BatchEvent(ev))).await;
        }
    }))
}

/* ═════════════════ view ═════════════════ */

pub fn view(app: &App) -> Element<'_, Message> {
    let w = &app.work;

    // G2：按 zoom 选显示层。缩小视图用双线性预降采样的 preview 层
    // （GPU 上传量按面积锐减），放大到 >=100% 时切回全分辨率看细节。
    // 纯函数 + 只读，不含状态写入；低频路径无 preview 时自动回退 full。
    let result_h = match pyramid::select(w.zoom, w.result_preview.is_some()) {
        Level::Preview => w.result_preview.as_ref(),
        Level::Full => w.result_handle.as_ref(),
    };
    let original_h = match pyramid::select(w.zoom, w.orig_preview.is_some()) {
        Level::Preview => w.orig_preview.as_ref(),
        Level::Full => w.orig_handle.as_ref(),
    };

    // ── 顶部工具条 ──
    let has_img = w.src.is_some();
    // 看图模式（对比/原图）下编辑类工具置灰（移动 / 智能抠图不受影响）
    let view_only = w.view_mode != ViewMode::Result;
    let tool = w.tool;

    let toolstrip: Element<'_, Message> = container(
        row![
            // 上传是「动作」而非工具模式（参考 #toolUpload 不带 data-tool）
            rail_button(
                "upload",
                tr("上传图片"),
                false,
                true,
                Some(Message::Work(WorkMessage::OpenImage)),
            ),
            // 智能抠图：用户主动点击才执行，未导入图片时禁用
            rail_button(
                "wand",
                tr("智能抠图（自动）"),
                w.tool == Tool::Auto,
                has_img && !w.busy && !w.exporting,
                Some(Message::Work(WorkMessage::DoMatte)),
            ),
            rail_div(),
            tool_btn("brush", tr("保留画笔 · 修回主体"), Tool::Keep, tool, has_img && !view_only),
            tool_btn("eraser", tr("擦除画笔 · 抹掉残留"), Tool::Erase, tool, has_img && !view_only),
            tool_btn("crop", tr("裁剪"), Tool::Crop, tool, has_img && !view_only),
            tool_btn("pipette", tr("吸管 · 取样背景色"), Tool::Picker, tool, has_img && !view_only),
            tool_btn(
                "move",
                tr("移动画布 · 拖动平移（也可按住空格临时切换）"),
                Tool::Pan,
                tool,
                has_img,
            ),
            rail_div(),
            rail_button(
                "layout-grid",
                tr("批量队列"),
                false,
                true,
                Some(Message::Work(WorkMessage::OpenBatch)),
            ),
            rail_button(
                "rotate-ccw",
                tr("重置蒙版"),
                false,
                has_img && w.mask.is_some(),
                Some(Message::Work(WorkMessage::ResetMask)),
            ),
            // 原生版额外提供撤销按钮（参考仅 Ctrl+Z 快捷键），沿用同一套 .rail-btn 规范
            rail_button(
                "undo-2",
                tr("撤销（Ctrl+Z）"),
                false,
                has_img && w.history.can_undo(),
                Some(Message::Work(WorkMessage::Undo)),
            ),
            rail_div(),
            Space::new().width(Length::Fill),
            // 导出按钮：参考 `.btn`（34 高 / padding 0 15 / gap 7 / 15px 图标 / 12.5px 字 / 白字）
            // 内容要显式垂直居中：iced 的 `Button` 把内容放在 padding 的**顶端**，
            // 纵向 padding 为 0 时图标与文字会整体贴上沿（实测偏上 7.7px）。
            press(
                button(
                    container(
                        row![
                            icon("download", 15.0, iced::Color::WHITE),
                            text(tr("导出"))
                                .size(12.5)
                                .color(iced::Color::WHITE)
                                .font(bold_font()),
                        ]
                        .spacing(7.0)
                        .align_y(Alignment::Center),
                    )
                    .height(Length::Fill)
                    .align_y(Alignment::Center),
                )
                .height(Length::Fixed(34.0))
                .padding(pad(0.0, 15.0, 0.0, 15.0))
                .style(btn_primary),
                has_img && w.result.is_some() && !w.busy && !w.exporting,
                Message::Work(WorkMessage::Export),
            ),
        ]
        .spacing(3.0)
        .align_y(Alignment::Center)
        .padding(pad(0.0, 10.0, 0.0, 10.0)),
    )
    .height(Length::Fixed(STRIP_H))
    .align_y(iced::alignment::Vertical::Center)
    .style(panel)
    .into();

    // ── 画布 ──
    let stage: Element<Message> = if !has_img {
        empty_state()
    } else {
        let kind = match w.view_mode {
            ViewMode::Result => StageKind::Result,
            ViewMode::Compare => StageKind::Compare,
            ViewMode::Original => StageKind::Original,
        };
        let (iw, ih) = w
            .src
            .as_ref()
            .map(|s| (s.width() as f32, s.height() as f32))
            .unwrap_or((0.0, 0.0));
        let geom = cv::StageGeom {
            img_size: (iw, ih),
            zoom: w.zoom,
            pan: w.pan,
        };
        cv::stage_canvas(
            StageView {
                geom,
                result: result_h,
                original: original_h,
                kind,
                compare_at: w.compare_at,
                backdrop: match w.bg_preview {
                    BgPreview::Checker => cv::Backdrop::Checker,
                    BgPreview::White => cv::Backdrop::White,
                    BgPreview::Black => cv::Backdrop::Black,
                },
                tool: w.tool,
            },
            cv::StageOverlay {
                geom,
                kind,
                compare_at: w.compare_at,
                crop: if w.tool == Tool::Crop { w.crop } else { None },
                brush: w
                    .brush_cursor
                    .map(|p| {
                        // 裁剪工具的控制点悬停反馈使用固定屏幕像素半径（12px）
                        // 画笔/橡皮擦工具使用笔刷大小 * 缩放
                        let radius = if w.tool == Tool::Crop {
                            12.0
                        } else {
                            (w.brush_size as f32 * w.zoom).max(1.0)
                        };
                        (p, radius)
                    }),
                picker: w.picker_at,
                painting: w.drag == Some(Drag::Paint),
                erase: w.tool == Tool::Erase,
            },
        )
    };
    // 画布区域：载入/处理中时，加载动画只在画布内居中显示
    let stage_area: Element<'_, Message> = if w.busy || w.loading || w.exporting {
        let (t, sub): (&'static str, &'static str) = if w.loading {
            (tr("正在载入图片…"), "")
        } else if w.exporting {
            (tr("正在导出…"), "")
        } else {
            (tr("正在抠图…"), "SmartMatte")
        };
        container(
            stack![
                stage,
                crate::ui::widgets::busy_overlay(t, sub, w.frame)
            ]
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(20.0)
        .style(stage_bg(BG()))
        .into()
    } else {
        container(stage)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20.0)
            .style(stage_bg(BG()))
            .into()
    };

    // ── 底部视图条 ──
    // 缩放组：单一药丸容器（放大镜 − 百分比 + 放大镜 | 全屏适应），参考截图样式
    let zoom_group: Element<'_, Message> = container(
        row![
            button(icon("minus", 14.0, TXT2()))
                .on_press(Message::Work(WorkMessage::ZoomOut))
                .padding(pad(4.0, 6.0, 4.0, 6.0))
                .style(seg_btn(false)),
            text(format!("{:.0}%", w.zoom * 100.0))
                .size(FS_SM)
                .color(TXT())
                .font(bold_font()),
            button(icon("plus", 14.0, TXT2()))
                .on_press(Message::Work(WorkMessage::ZoomIn))
                .padding(pad(4.0, 6.0, 4.0, 6.0))
                .style(seg_btn(false)),
            container(Space::new().width(Length::Fixed(1.0)).height(Length::Fixed(14.0)))
                .style(|_t| container::Style {
                    background: Some(iced::Background::Color(LINE())),
                    ..container::Style::default()
                }),
            button(icon("maximize", 13.0, TXT2()))
                .on_press(Message::Work(WorkMessage::ZoomFit))
                .padding(pad(4.0, 6.0, 4.0, 6.0))
                .style(seg_btn(false)),
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding(pad(2.0, 4.0, 2.0, 4.0))
    .style(|_t| container::Style {
        text_color: None,
        background: Some(iced::Background::Color(PANEL())),
        border: iced::Border { color: LINE(), width: 1.0, radius: 8.0.into() },
        shadow: iced::Shadow::default(),
        snap: false,
    })
    .into();

    let stage_bar: Element<'_, Message> = container(
        row![
            zoom_group,
            seg_strong(&[
                (tr("原图"), w.view_mode == ViewMode::Original, Message::Work(WorkMessage::SetViewMode(ViewMode::Original))),
                (tr("对比"), w.view_mode == ViewMode::Compare, Message::Work(WorkMessage::SetViewMode(ViewMode::Compare))),
                (tr("结果"), w.view_mode == ViewMode::Result, Message::Work(WorkMessage::SetViewMode(ViewMode::Result))),
            ]),
            Space::new().width(Length::Fill),
            seg_strong(&[
                (BgPreview::Checker.label(), w.bg_preview == BgPreview::Checker, Message::Work(WorkMessage::SetBgPreview(BgPreview::Checker))),
                (BgPreview::White.label(), w.bg_preview == BgPreview::White, Message::Work(WorkMessage::SetBgPreview(BgPreview::White))),
                (BgPreview::Black.label(), w.bg_preview == BgPreview::Black, Message::Work(WorkMessage::SetBgPreview(BgPreview::Black))),
            ]),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .padding(pad(0.0, 12.0, 0.0, 12.0)),
    )
    .height(Length::Fixed(STAGE_BAR_H))
    .align_y(iced::alignment::Vertical::Center)
    .style(panel)
    .into();

    // 左列三张卡（工具条 / 舞台 / 视图条）之间 10px —— 参考 `#pageWork{gap:10px}`。
    // iced 的 column!/row! 默认 spacing 为 0，必须显式给出，否则三张卡会紧贴在一起。
    let left = column![toolstrip, stage_area, stage_bar].spacing(BODY_GAP);

    // 左列与右侧检查器之间同样 10px（参考 `.stage` 与 `.inspector` 同为 `#pageWork` 的
    // flex 子项，间距由 `#pageWork{gap:10px}` 给出）
    let root = row![left, inspector(app)].spacing(BODY_GAP);

    // 「笔刷属性」下拉面板：仅在展开时叠一层浮层，覆盖在内容之上、点击外部收起。
    // 与参考 `.brush-pop{position:absolute;top:calc(100%+8px);left:0}` 对齐（锚在工具栏左下）。
    let base: Element<'_, Message> = match brush_props_popover(w) {
        Some(pop) => stack![root, pop].into(),
        None => root.into(),
    };
    base
}

/* ═════════════════ 工具条 · `.rail-btn` 规范 ═════════════════
 * 工具按钮：`active` 为当前选中工具；`enabled=false` 时置灰不可点
 *（参考项目 applyToolAvailability：看图模式下编辑类工具置灰；无图时全部工具禁用）
 ═════════════════ */

/// 工具条图标按钮边长（参考 `.rail-btn{width:34px;height:34px}`）
const RAIL_BTN: f32 = 34.0;
/// 工具条图标尺寸（参考 `.rail-btn svg{width:19px;height:19px}`）
const RAIL_ICON: f32 = 19.0;

/// 分组分隔线 `.rail-div{width:1px;height:20px;background:var(--line-soft);margin:0 5px}`
///
/// 外层容器只承担左右各 5px 的外边距（工具条自身 `gap:3px`，合计 8px 视觉间距）；
/// 真正画线的 1×20 容器放在内层 —— 否则背景色会把含 padding 的整块区域涂灰。
fn rail_div() -> Element<'static, Message> {
    container(
        container(Space::new().width(Length::Fixed(1.0)).height(Length::Fixed(20.0))).style(|_t| {
            container::Style {
                background: Some(iced::Background::Color(LINE_SOFT())),
                ..container::Style::default()
            }
        }),
    )
    .padding(pad(0.0, 5.0, 0.0, 5.0))
    .into()
}

/// 激活指示条 `.rail-btn.active:before`：16×2.5、圆角 3、居中、距底 3px、ink 色
fn rail_underline() -> Element<'static, Message> {
    container(Space::new().width(Length::Fixed(16.0)).height(Length::Fixed(2.5)))
        .style(|_t| container::Style {
            background: Some(iced::Background::Color(INK())),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 3.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// 悬停提示 `.rail-btn .tip`：ink 底 + 8px 圆角 + shadow-3 + 11.5px 白字
fn tip_body<'a>(tip: &'a str) -> Element<'a, Message> {
    container(text(tip).size(FS_XS).color(iced::Color::WHITE))
        .padding(pad(6.0, 10.0, 6.0, 10.0))
        .style(crate::theme::tooltip_ink)
        .into()
}

/// 工具条图标按钮：34×34 方块 + 19px 图标 + 悬停提示；激活时底部一条 ink 指示条。
///
/// 对应参考 `.rail-btn` 的三个状态：
/// - 常态 `color:var(--txt2)` / 悬停 `background:var(--panel-3)`
/// - 选中 `.active{background:var(--fill);color:var(--ink)}` + `:before` 下划条
/// - 禁用 `:disabled{opacity:.3}`（原生版用 30% 透明度的图标色近似）
fn rail_button<'a>(
    icon_name: &'static str,
    tip: &'a str,
    active: bool,
    enabled: bool,
    msg: Option<Message>,
) -> Element<'a, Message> {
    let color = if !enabled {
        iced::Color { a: 0.30, ..TXT2() }
    } else if active {
        INK()
    } else {
        TXT2()
    };

    let content: Element<'a, Message> = if active && enabled {
        stack![
            container(icon(icon_name, RAIL_ICON, color))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            container(rail_underline())
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::End)
                .padding(pad(0.0, 0.0, 3.0, 0.0)),
        ]
        .into()
    } else {
        container(icon(icon_name, RAIL_ICON, color))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    };

    let b = button(content)
        .width(Length::Fixed(RAIL_BTN))
        .height(Length::Fixed(RAIL_BTN))
        .padding(0)
        .style(move |t: &iced::Theme, s| {
            if enabled {
                rail_btn(active)(t, s)
            } else {
                rail_btn_disabled()(t, s)
            }
        });
    let b = match msg {
        Some(m) if enabled => b.on_press(m),
        _ => b,
    };

    iced::widget::tooltip::Tooltip::new(
        b,
        tip_body(tip),
        iced::widget::tooltip::Position::Bottom,
    )
    .gap(9.0)
    .padding(0.0)
    .style(|_t| container::Style::default())
    .into()
}

/// 「笔刷属性」下拉面板：仅在展开时渲染。
///
/// 结构对齐参考 `bind.js` 的 `brushPop`（`.brush-pop{position:absolute;top:calc(100%+8px);left:0}`）：
/// - 底层 `mouse_area` 覆盖整块区域，点击外部 → `CloseBrushProps`（参考 document 监听的「点击卡片外收起」）。
/// - 上层是卡片本体，自身区域拦截事件，所以点在滑块 / 卡片内不会误触收起。
/// - 卡片锚在工具栏左下（`top = STRIP_H + 8`，`left:0`），与参考 `left:0` 一致。
///
/// 卡片内四项直接复用 `slider_row`，与右侧检查器的「笔刷属性」小节共用同一套消息，
/// 故两种入口修改的是**同一份状态**、实时生效；硬度按当前工具取 `hardness_keep` / `hardness_erase`，
/// 两种画笔模式各自独立保存（参考 `state.brush.hardness` 的嵌套结构）。
fn brush_props_popover<'a>(w: &WorkState) -> Option<Element<'a, Message>> {
    if !w.brush_props_open {
        return None;
    }
    let hard = if w.tool == Tool::Erase {
        w.hardness_erase
    } else {
        w.hardness_keep
    };
    // 羽化宽度 = 半径 × (1 − 硬度)，与参考 `updateBrushHint()` 同义
    let feather = (w.brush_size as f32 * (1.0 - hard as f32 / 100.0)).round();
    let hint = if feather > 0.5 {
        trf("羽化 {}px", &[&feather.to_string()])
    } else {
        tr("无羽化").to_string()
    };

    let card: Element<'a, Message> = container(
        column![
            // 卡片标题 `.bp-head`
            strong(tr("笔刷属性"), FS_XS, TXT()),
            column![
                slider_row(
                    tr("大小"),
                    w.brush_size,
                    4..=80,
                    " px",
                    |v| Message::Work(WorkMessage::SetBrushSize(v)),
                ),
                slider_row(
                    tr("硬度"),
                    hard,
                    0..=100,
                    "",
                    |v| Message::Work(WorkMessage::SetBrushHardness(v)),
                ),
                slider_row(
                    tr("不透明"),
                    w.brush_opacity,
                    1..=100,
                    "",
                    |v| Message::Work(WorkMessage::SetBrushOpacity(v)),
                ),
                slider_row(
                    tr("流量"),
                    w.brush_flow,
                    1..=100,
                    "",
                    |v| Message::Work(WorkMessage::SetBrushFlow(v)),
                ),
            ]
            .spacing(11),
            // 羽化提示 `.bp-hint`
            text(hint).size(FS_XS).color(TXT3()),
        ]
        .spacing(10),
    )
    .padding(pad(12.0, 13.0, 10.0, 13.0))
    .style(|_t| container::Style {
        background: Some(iced::Background::Color(PANEL())),
        border: iced::Border {
            color: LINE(),
            width: 1.0,
            radius: R_CARD.into(),
        },
        shadow: shadow_3(),
        ..container::Style::default()
    })
    .width(Length::Fixed(250.0))
    .into();

    let layer: Element<'a, Message> = stack![
        // 底层：点击工具栏以下任意空白处收起面板（工具栏本身仍保持可交互）
        container(
            mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(Message::Work(WorkMessage::CloseBrushProps)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(pad(STRIP_H + 8.0, 0.0, 0.0, 0.0)),
        // 上层：卡片锚在工具栏左下（与参考 `.brush-pop{top:calc(100% + 8px);left:0}` 对齐）
        container(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(pad(STRIP_H + 8.0, 0.0, 0.0, 0.0))
            .align_x(Alignment::Start)
            .align_y(Alignment::Start),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into();

    Some(layer)
}

/// 工具按钮（`.rail-btn[data-tool]`）：点击切换当前工具，激活态由 `active` 决定
fn tool_btn<'a>(
    icon_name: &'static str,
    tip: &'a str,
    tool: Tool,
    active: Tool,
    enabled: bool,
) -> Element<'a, Message> {
    let on = active == tool && enabled;
    rail_button(
        icon_name,
        tip,
        on,
        enabled,
        Some(Message::Work(WorkMessage::SetTool(tool))),
    )
}

/// 检查器内的「宽高比」分段（自由 / 1:1 / 4:3 …）
///
/// 弱分段（`.seg`）：选中态是白底 + 近黑字 + 半像素描边的小胶囊，与工具条的强分段
/// （近黑实心）刻意区分。选项较多，故 `row` 允许换行。标签转成 `String` 持有，
/// 返回 `Element<'static>` 不再借用传入切片，避免临时数组导致的借用错误。
fn seg_group<T: PartialEq + Copy>(
    items: &[(T, &str)],
    selected: T,
    on_select: impl Fn(T) -> Message,
) -> Element<'static, Message> {
    let mut r = row![].spacing(2);
    for (val, label) in items {
        let active = *val == selected;
        r = r.push(
            button(strong(label.to_string(), FS_XS, if active { INK() } else { TXT2() }))
                .on_press(on_select(*val))
                .padding(pad(5.0, 11.0, 5.0, 11.0))
                .style(seg_soft_btn(active)),
        );
    }
    // 选项较多，允许换行（iced 的 `Row::wrap()` 必须在 push 全部子项之后调用）
    // 外层轨道：`--fill` 底 + `--r-sm` 圆角 + 2px 内边距，对应 `.seg`
    let wrapped: Element<'static, Message> = r.wrap().into();
    container(wrapped)
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

fn empty_state<'a>() -> Element<'a, Message> {
    let icon_box = container(icon("image", 34.0, INK()))
        .width(76.0)
        .height(76.0)
        .center_x(Length::Fixed(76.0))
        .center_y(Length::Fixed(76.0))
        .style(|_t| container::Style {
            text_color: Some(INK()),
            background: Some(iced::Background::Color(PANEL())),
            border: iced::Border { color: LINE(), width: 1.0, radius: 22.0.into() },
            shadow: shadow_2(),
            snap: false,
        });
    container(
        column![
            icon_box,
            text(tr("拖入商品图，开始抠图")).size(18.0).color(TXT()),
            hint(tr("内置本地智能引擎，纯本地推理，图片不上传服务器。支持白底图、纯色背景、模特实拍等电商场景。")),
            button(text(tr("选择图片")).size(FS_SM).color(iced::Color::WHITE))
                .on_press(Message::Work(WorkMessage::OpenImage))
                .padding(pad(9.0, 18.0, 9.0, 18.0))
                .style(btn_primary),
            hint(tr("PNG / JPG / WEBP · 本地推理 · 隐私安全")),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

/* ═════════════════ 右侧检查器 ═════════════════ */

fn inspector<'a>(app: &'a App) -> Element<'a, Message> {
    let w = &app.work;

    // 推理引擎
    // 模型下拉 = 色键预设 + models/ 目录扫描到的神经网络模型（插件式发现）
    // 过滤掉人脸检测模型（仅用于人脸检测，不参与抠图）
    // 按类别分组：通用商品 → 人像抠图 → 通用分割 → 其他
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ModelItem {
        label: String,
        id: String,
        category: u8, // 排序用：0=通用商品, 1=人像, 2=通用分割, 3=其他
    }

    let mut items: Vec<ModelItem> = Vec::new();

    // 1. 色键预设（SmartMatte v2）—— 通用商品
    for m in ModelKind::ALL {
        items.push(ModelItem {
            label: m.label().to_string(),
            id: m.id().to_string(),
            category: 0,
        });
    }

    // 2. 神经网络模型 —— 按类别分组
    for m in &w.nn_models {
        let lower = m.file.to_ascii_lowercase();
        if lower.contains("face") || lower.contains("yunet") {
            continue; // 跳过人脸检测模型
        }
        let cat_order = match m.category {
            crate::core::nn::ModelCategory::Salient => 1,
            crate::core::nn::ModelCategory::Portrait => 2,
            crate::core::nn::ModelCategory::Other => 3,
            crate::core::nn::ModelCategory::FaceDetect => continue,
        };
        items.push(ModelItem {
            label: m.label.clone(),
            id: m.id.clone(),
            category: cat_order,
        });
    }

    // 按类别排序（同类别内保持原有顺序）
    items.sort_by_key(|item| item.category);

    let item_labels: Vec<String> = items.iter().map(|item| item.label.clone()).collect();
    let current_label = items
        .iter()
        .find(|item| item.id == app.config.model)
        .map(|item| item.label.clone())
        .unwrap_or_else(|| ModelKind::Smart.label().to_string());

    let engine = section(
        tr("推理引擎"),
        Some(badge("LOCAL", BadgeKind::Ok)),
        column![
            column![
                field_label(tr("模型"), "SmartMatte v2", ""),
                pick_list(
                    item_labels,
                    Some(current_label),
                    move |l| {
                        // label → id（下拉项文案与模型 id 解耦）
                        let id = items
                            .iter()
                            .find(|item| item.label == l)
                            .map(|item| item.id.clone())
                            .unwrap_or_else(|| "local-smart".into());
                        Message::Work(WorkMessage::SetModel(id))
                    },
                )
                .width(Length::Fill)
                .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
            ]
            .spacing(7),
            button(
                row![
                    icon("rotate-cw", 12.0, TXT2()),
                    text(trf(
                        "重新扫描模型（{}）",
                        &[&w.nn_models.len().to_string()],
                    ))
                    .size(FS_XS)
                    .color(TXT2()),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Work(WorkMessage::RescanModels))
            .padding(pad(4.0, 6.0, 4.0, 6.0))
            .style(btn_secondary),
            if w.nn_models.is_empty() {
                hint(match &w.models_dir {
                    Some(d) => trf("models/ 未发现 .onnx（{}）", &[d.display().to_string().as_str()]),
                    None => tr("未找到 models 目录，把 .onnx 模型放入后点重新扫描").to_string(),
                })
            } else {
                column![].into()
            },
            switch_row(
                tr("自动重算（改参数即时生效）"),
                app.config.auto,
                Message::Work(WorkMessage::ToggleAuto(!app.config.auto)),
                None,
            ),
        ]
        .spacing(11),
    );

    // 蒙版参数
    let sample_label = match w.params.sample {
        SampleMode::Auto => tr("自动四角"),
        SampleMode::Corner => tr("仅左上角"),
        SampleMode::Custom => tr("自定义"),
    };
    // 自定义颜色面板（当选择tr("自定义"))背景取样时显示，放在边缘阈值上方）
    let custom_color_el: Option<Element<'_, Message>> = if w.params.sample == SampleMode::Custom {
        let swatch_color = parse_hex_rgb(&w.custom_hex)
            .map(|[r, g, b]| iced::Color::from_rgb8(r, g, b))
            .unwrap_or(CHECKER_B());
        let picker_active = w.tool == Tool::Picker;
        let picker_icon_color = if picker_active { INK() } else { TXT2() };
        let picker_btn: Element<'_, Message> = iced::widget::tooltip::Tooltip::new(
            button(icon("pipette", 11.0, picker_icon_color))
                .width(Length::Fixed(20.0))
                .height(Length::Fixed(20.0))
                .padding(0)
                .on_press(Message::Work(WorkMessage::SetTool(Tool::Picker)))
                .style(rail_btn(picker_active)),
            tip_body(tr("吸管取样 · 点击画布吸取颜色")),
            iced::widget::tooltip::Position::Bottom,
        )
        .into();
        Some(
            column![
                field_label(tr("自定义颜色"), "", ""),
                row![
                    container(Space::new().width(Length::Fixed(26.0)).height(Length::Fixed(26.0)))
                        .style(move |_t| container::Style {
                            background: Some(iced::Background::Color(swatch_color)),
                            border: iced::Border { color: LINE(), width: 1.0, radius: R_SM.into() },
                            ..container::Style::default()
                        }),
                    iced::widget::text_input("#RRGGBB", &w.custom_hex)
                        .on_input(|v| Message::Work(WorkMessage::SetCustomHex(v)))
                        .size(FS_SM)
                        .width(Length::Fill)
                        .style(text_input_style),
                    picker_btn,
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                pick_list(
                    KeyMode::ALL,
                    Some(if w.params.key_all { KeyMode::All } else { KeyMode::Connected }),
                    |k| Message::Work(WorkMessage::ToggleKeyAll(k == KeyMode::All)),
                )
                .width(Length::Fill)
                .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
            ]
            .spacing(7)
            .into(),
        )
    } else {
        None
    };

    let mut mask_col = column![
        column![
            field_label(tr("背景取样"), sample_label, ""),
            pick_list(
                SampleMode::ALL,
                Some(w.params.sample),
                |m| Message::Work(WorkMessage::SetSampleMode(m)),
            )
            .width(Length::Fill)
            .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
        ]
        .spacing(7),
    ]
    .spacing(13);

    // 插入自定义颜色面板（在边缘阈值上方）
    if let Some(el) = custom_color_el {
        mask_col = mask_col.push(el);
    }

    mask_col = mask_col
        .push(slider_row(
            tr("边缘阈值"),
            w.params.thresh,
            1..=100,
            " /100",
            |v| Message::Work(WorkMessage::SetParam(ParamKind::Thresh, v)),
        ))
        .push(slider_row(
            tr("轮廓保护"),
            w.params.edge,
            0..=100,
            " /100",
            |v| Message::Work(WorkMessage::SetParam(ParamKind::Edge, v)),
        ))
        .push(slider_row(
            tr("边缘羽化"),
            w.params.feather,
            0..=100,
            " /100",
            |v| Message::Work(WorkMessage::SetParam(ParamKind::Feather, v)),
        ))
        .push(slider_row(
            tr("蒙版收缩 / 扩张"),
            w.params.morph,
            -8..=8,
            " px",
            |v| Message::Work(WorkMessage::SetParam(ParamKind::Morph, v)),
        ))
        .push(slider_row(
            tr("去色晕 Defringe"),
            w.params.defringe,
            0..=100,
            "%",
            |v| Message::Work(WorkMessage::SetParam(ParamKind::Defringe, v)),
        ))
        .push(switch_row(
            tr("填充内部孔洞"),
            w.params.fill,
            Message::Work(WorkMessage::ToggleFill(!w.params.fill)),
            None,
        ))
        .push(switch_row(
            tr("保留投影"),
            w.params.shadow,
            Message::Work(WorkMessage::ToggleShadow(!w.params.shadow)),
            None,
        ));

    if w.params.fill {
        mask_col = mask_col.push(slider_row(
            tr("最小通道宽"),
            w.params.fill_neck,
            0..=14,
            " px",
            |v| Message::Work(WorkMessage::SetParam(ParamKind::FillNeck, v)),
        ));
    }
    let mask_sec = section(tr("蒙版参数"), None, mask_col);

    // 输出
    let out_sec = section(
        tr("输出"),
        None,
        column![
            column![
                field_label(tr("背景"), "", ""),
                swatch_grid(OUT_COLORS, &w.out_color, |c| Message::Work(WorkMessage::SetOutColor(c)))
            ]
            .spacing(7),
            column![
                field_label(tr("格式"), "", ""),
                pick_list(
                    ExportFormat::all(),
                    Some(app.config.export_format()),
                    |f| Message::Work(WorkMessage::SetFormat(f)),
                )
                .width(Length::Fill)
                .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
            ]
            .spacing(7),
            column![
                field_label(tr("导出尺寸"), "", ""),
                pick_list(
                    ExportSize::all(),
                    Some(app.config.export_size()),
                    |s| Message::Work(WorkMessage::SetExportSize(s)),
                )
                .width(Length::Fill)
                .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
            ]
            .spacing(7),
        ]
        .spacing(13),
    );

    // 批量（单项状态徽标 + 移除/清空/重试 + 进度防重入）
    let batch_running = w.batch_running;
    let done_n = w.batch.iter().filter(|it| it.status == crate::app::QueueStatus::Done).count();
    let fail_n = w.batch.iter().filter(|it| it.status == crate::app::QueueStatus::Failed).count();
    let pending_n = w.batch.iter().filter(|it| it.status == crate::app::QueueStatus::Pending).count();
    let badge_txt = if batch_running {
        format!("{}/{}", done_n + fail_n, w.batch.len())
    } else {
        w.batch.len().to_string()
    };

    // 队列项列表：文件名 + 状态（失败可展开明细），超长滚动
    let mut batch_list = column![].spacing(4);
    for (i, it) in w.batch.iter().enumerate() {
        let name = it
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let (status_txt, status_color) = match it.status {
            crate::app::QueueStatus::Pending => (tr("排队中"), TXT3()),
            crate::app::QueueStatus::Running => (tr("处理中"), ACC()),
            crate::app::QueueStatus::Done => (tr("完成"), OK()),
            crate::app::QueueStatus::Failed => (tr("失败"), DANGER()),
        };
        // 失败项把原因放进文件名行尾（悬停语义由 toast 承担，点行查看太重）
        let name_txt = if it.status == crate::app::QueueStatus::Failed {
            let e = it.err.clone().unwrap_or_default();
            format!("{name} — {}", e.chars().take(40).collect::<String>())
        } else {
            name
        };
        let mut item = row![
            text(status_txt).size(FS_XS).color(status_color),
            text(name_txt).size(FS_XS).color(TXT2()),
            Space::new().width(Length::Fill),
        ]
        .spacing(7)
        .align_y(Alignment::Center);
        // 移除按钮：非处理中的项才可移除
        if !batch_running && it.status != crate::app::QueueStatus::Running {
            item = item.push(
                button(text("×").size(FS_XS).color(TXT3()))
                    .padding(pad(1.0, 5.0, 1.0, 5.0))
                    .style(btn_secondary)
                    .on_press(Message::Work(WorkMessage::BatchRemove(i))),
            );
        }
        batch_list = batch_list.push(item);
    }
    let batch_list_el: Element<'_, Message> = if w.batch.is_empty() {
        hint(tr("暂无任务，点击「添加」选择多张图"))
    } else if w.batch.len() > 6 {
        // 超过 6 项给固定高度滚动区，避免侧栏无限拉长
        container(
            scrollable(batch_list)
                .direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().width(3).scroller_width(3))),
        )
        .height(Length::Fixed(190.0))
        .width(Length::Fill)
        .into()
    } else {
        batch_list.into()
    };

    let batch_sec = section(
        tr("批量队列"),
        Some(badge(badge_txt, if batch_running { BadgeKind::Ink } else { BadgeKind::Dim })),
        column![
            batch_list_el,
            // 次级行：队列管理动作（条件显示，换行也不破坏层次）
            row![
                press(
                    button(container(text(tr("添加")).size(FS_SM)).width(Length::Fill).align_x(Alignment::Center))
                        .width(Length::Fill)
                        .padding(pad(6.0, 8.0, 6.0, 8.0))
                        .style(btn_secondary),
                    !batch_running,
                    Message::Work(WorkMessage::BatchAdd),
                ),
                press(
                    button(container(text(tr("重试失败项")).size(FS_SM)).width(Length::Fill).align_x(Alignment::Center))
                        .width(Length::Fill)
                        .padding(pad(6.0, 8.0, 6.0, 8.0))
                        .style(btn_secondary),
                    fail_n > 0 && !batch_running,
                    Message::Work(WorkMessage::BatchRetryFailed),
                ),
                press(
                    button(container(text(tr("清空")).size(FS_SM)).width(Length::Fill).align_x(Alignment::Center))
                        .width(Length::Fill)
                        .padding(pad(6.0, 8.0, 6.0, 8.0))
                        .style(btn_secondary),
                    !w.batch.is_empty() && !batch_running,
                    Message::Work(WorkMessage::BatchClear),
                ),
            ]
            .spacing(8)
            .width(Length::Fill),
            // 主按钮：通栏铺满，带待处理数量（证件照页导出按钮同款层次）
            press(
                button(
                    container(
                        text(
                            if batch_running {
                                tr("处理中…").to_string()
                            } else {
                                trf("开始处理（{}）", &[&pending_n.to_string()])
                            }
                        )
                        .size(FS_SM)
                        .color(iced::Color::WHITE),
                    )
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
                )
                .width(Length::Fill)
                .padding(pad(7.0, 12.0, 7.0, 12.0))
                .style(btn_primary),
                pending_n > 0 && !batch_running,
                Message::Work(WorkMessage::BatchRun),
            ),
        ]
        .spacing(11),
    );

    // 笔刷属性（保留/擦除工具下显示）
    let brush_sec = if matches!(w.tool, Tool::Keep | Tool::Erase) {
        let hard = if w.tool == Tool::Erase { w.hardness_erase } else { w.hardness_keep };
        Some(section(
            tr("笔刷属性"),
            None,
            column![
                slider_row(tr("大小"), w.brush_size, 4..=80, " px", |v| Message::Work(WorkMessage::SetBrushSize(v))),
                slider_row(tr("硬度"), hard, 0..=100, "", |v| Message::Work(WorkMessage::SetBrushHardness(v))),
                slider_row(tr("不透明"), w.brush_opacity, 1..=100, "", |v| Message::Work(WorkMessage::SetBrushOpacity(v))),
                slider_row(tr("流量"), w.brush_flow, 1..=100, "", |v| Message::Work(WorkMessage::SetBrushFlow(v))),
            ]
            .spacing(13),
        ))
    } else {
        None
    };

    // 裁剪 / 旋转（裁剪工具下显示）
    let crop_sec = if w.tool == Tool::Crop {
        Some(section(
            tr("裁剪与旋转"),
            Some(badge(format!("{}/3", w.history.depth()), BadgeKind::Dim)),
            column![
                column![
                    field_label(tr("宽高比"), w.crop_ratio.label(), ""),
                    seg_group(
                    &CROP_RATIOS.iter().map(|(r, l)| (*r, tr(l))).collect::<Vec<_>>(),
                    w.crop_ratio,
                    |r| {
                        Message::Work(WorkMessage::SetCropRatio(r))
                    },
                ),
                ]
                .spacing(7),
                row![
                    button(
                        row![icon("rotate-ccw", 14.0, TXT2()), text("90°").size(FS_SM)]
                            .spacing(4)
                            .align_y(Alignment::Center)
                    )
                    .on_press(Message::Work(WorkMessage::RotateNinety(false)))
                    .padding(pad(6.0, 10.0, 6.0, 10.0))
                    .width(Length::Fill)
                    .style(btn_secondary),
                    button(
                        row![icon("rotate-cw", 14.0, TXT2()), text("90°").size(FS_SM)]
                            .spacing(4)
                            .align_y(Alignment::Center)
                    )
                    .on_press(Message::Work(WorkMessage::RotateNinety(true)))
                    .padding(pad(6.0, 10.0, 6.0, 10.0))
                    .width(Length::Fill)
                    .style(btn_secondary),
                ]
                .spacing(8),
                slider_row(tr("旋转校正"), w.rot_deg as i32, -45..=45, "°", |v| {
                    Message::Work(WorkMessage::SetRotateDeg(v as f32))
                }),
                press(
                    button(text(tr("应用旋转")).size(FS_SM))
                        .padding(pad(6.0, 12.0, 6.0, 12.0))
                        .style(btn_secondary)
                        .width(Length::Fill),
                    w.rot_deg.abs() > 0.05,
                    Message::Work(WorkMessage::ApplyRotate),
                ),
                hint(tr("在画布上拖动框选；拖框内可整体移动，拖右下角可缩放。")),
                row![
                    press(
                        button(text(tr("取消")).size(FS_SM))
                            .padding(pad(6.0, 12.0, 6.0, 12.0))
                            .style(btn_secondary)
                            .width(Length::Fill),
                        true,
                        Message::Work(WorkMessage::CropCancel),
                    ),
                    press(
                        button(text(tr("应用裁剪")).size(FS_SM).color(iced::Color::WHITE))
                            .padding(pad(6.0, 12.0, 6.0, 12.0))
                            .style(btn_primary)
                            .width(Length::Fill),
                        w.crop.is_some(),
                        Message::Work(WorkMessage::CropConfirm),
                    ),
                ]
                .spacing(8),
            ]
            .spacing(11),
        ))
    } else {
        None
    };

    // 小节之间用 1px 细分隔线（`.sec` 的 border-bottom），最后一个小节下方不留线
    let mut col = column![engine];
    col = col.push(rule());
    col = col.push(mask_sec);
    col = col.push(rule());
    col = col.push(out_sec);
    if let Some(c) = crop_sec {
        col = col.push(rule());
        col = col.push(c);
    }
    if let Some(b) = brush_sec {
        col = col.push(rule());
        col = col.push(b);
    }
    col = col.push(rule());
    col = col.push(batch_sec);

    container(
        scrollable(container(col).padding(pad(0.0, 0.0, SP_4, 0.0)))
            .direction(Direction::Vertical(Scrollbar::new().width(3).scroller_width(3)))
            .style(scrollbar_style),
    )
    .width(Length::Fixed(INSPECTOR_W))
    .height(Length::Fill)
    .style(panel)
    .into()
}

/// 裁剪宽高比选项（供 seg_group 使用）
const CROP_RATIOS: [(CropRatio, &str); 7] = [
    (CropRatio::Free, "自由"),
    (CropRatio::Square, "1:1"),
    (CropRatio::R4x3, "4:3"),
    (CropRatio::R3x2, "3:2"),
    (CropRatio::R16x9, "16:9"),
    (CropRatio::R9x16, "9:16"),
    (CropRatio::Orig, "原始"),
];

/// 输出背景色板（'static，避免把局部变量借进 Element）
const OUT_COLORS: &[(&str, Option<iced::Color>)] = &[
    ("transparent", None),
    ("#FFFFFF", Some(iced::Color::WHITE)),
    ("#F5F5F7", Some(OUT_F5)),
    ("#000000", Some(iced::Color::BLACK)),
    ("#E8F0FF", Some(OUT_E8)),
    ("#FFF4E5", Some(OUT_F4)),
];
const OUT_F5: iced::Color = iced::Color::from_rgb8(0xF5, 0xF5, 0xF7);
const OUT_E8: iced::Color = iced::Color::from_rgb8(0xE8, 0xF0, 0xFF);
const OUT_F4: iced::Color = iced::Color::from_rgb8(0xFF, 0xF4, 0xE5);

/// 条件启用按钮（iced 0.14 没有 on_press_if，这里显式二选一）
fn press<'a>(
    b: iced::widget::Button<'a, Message>,
    enabled: bool,
    msg: Message,
) -> iced::widget::Button<'a, Message> {
    if enabled {
        b.on_press(msg)
    } else {
        b
    }
}