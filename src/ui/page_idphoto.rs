//! 证件照页：上传 → 本地抠图换底 → 规格构图出图 → 相纸排版导出
//!
//! 对应原项目 `#pageId`。左侧预览（单张 / 排版），右侧规格 / 抠图 / 构图 / 预览 / 排版。

use iced::widget::image::Handle;
use iced::widget::{button, column, container, image as img_widget, pick_list, row, scrollable, text, Space, Stack};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::{Alignment, Color, Element, Length, Task, Theme};

use std::time::Duration;

use crate::app::App;
use crate::core::idphoto::{
    compose_single, erode_mask, make_layout, make_layout_with_cap, parse_hex, sheet_text, ID_BGS,
    ID_PAPERS, ID_SIZES, RenderMode,
};
use crate::core::image::{self as imgcore, ExportFormat};
use crate::core::{matting, nn};
use crate::message::{BatchExportEvent, IdMessage, IdView, LoadedPayload, Message};
use crate::theme::*;
use crate::ui::icons::icon;
use crate::ui::widgets::*;
use crate::core::i18n::{tr, trf};

const DEBOUNCE_MS: u64 = 100;

/// 旋转校正上限（度）：证件照头歪很少超过 ±15°，限幅防误拖
const ROT_MAX: i32 = 15;

/// 当前生效底色：自定义 HEX 开启时优先，否则取官方色板。
/// 返回 (颜色, 显示名, 说明文案) —— 名称与说明供侧栏 / meta 使用。
fn resolve_bg(s: &crate::app::IdState) -> ([u8; 3], &'static str, String) {
    if s.use_custom_bg {
        (parse_hex(&s.custom_bg_hex), tr("自定义"), trf("自定义 {}", &[&s.custom_bg_hex]))
    } else {
        let bg = &ID_BGS[s.bg_idx.min(ID_BGS.len() - 1)];
        (parse_hex(bg.c), bg.n, bg.d.to_string())
    }
}

/// 应用旋转校正（0° 时只克隆 Arc 指针，零像素拷贝）
fn apply_rot(fg: &std::sync::Arc<image::RgbaImage>, rot: i32) -> std::sync::Arc<image::RgbaImage> {
    if rot == 0 {
        std::sync::Arc::clone(fg)
    } else {
        std::sync::Arc::new(imgcore::rotate_deg(fg, rot as f32))
    }
}

/// 证件照抠图管线：优先使用用户选中的模型，否则回退 modnet.onnx，都没有则走色键。
fn matte_payload(app: &App, payload: LoadedPayload) -> Task<Message> {
    let model_info = if let Some(ref mid) = app.id.model_id {
        app.work.nn_models.iter().find(|m| &m.id == mid).cloned()
    } else {
        app.work.nn_models.iter().find(|m| m.file == "modnet.onnx").cloned()
    };
    let params = app.work.params.clone();
    crate::ui::page_work::blocking_task(
        move || {
            let (w, h) = (payload.rgba.width() as usize, payload.rgba.height() as usize);
            let fg = if let Some(info) = &model_info {
                match nn::matte(info, payload.rgba.as_raw(), w, h) {
                    Ok(mask) => imgcore::apply_mask(&payload.rgba, &mask),
                    Err(e) => return Err(trf("{} 抠图失败：{}", &[info.name.as_str(), &e])),
                }
            } else {
                let mut cache = matting::FieldCache::default();
                let res = matting::compute_mask(payload.rgba.as_raw(), w, h, &params, &mut cache);
                imgcore::apply_mask(&payload.rgba, &res.mask)
            };
            Ok(Box::new(LoadedPayload {
                rgba: fg,
                path: payload.path.clone(),
                stem: payload.stem.clone(),
            }))
        },
        |r| {
            Message::IdPhoto(IdMessage::MatteDone(match r {
                Some(Ok(v)) => Ok(v),
                Some(Err(e)) => Err(e),
                None => Err(tr("抠图线程异常退出").to_string()),
            }))
        },
    )
}

pub fn update(app: &mut App, msg: IdMessage) -> Task<Message> {
    let s = &mut app.id;
    match msg {
        IdMessage::OpenPhoto | IdMessage::ChangePhoto => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter(tr("图片"), &["png", "jpg", "jpeg", "webp", "bmp"])
                        .pick_file()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                |p| Message::IdPhoto(IdMessage::FilePicked(p)),
            );
        }
        IdMessage::OpenBatch => {
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter(tr("图片"), &["png", "jpg", "jpeg", "webp", "bmp"])
                        .pick_files()
                        .await
                        .map(|hs| hs.iter().map(|h| h.path().to_path_buf()).collect::<Vec<_>>())
                },
                |ps| Message::IdPhoto(IdMessage::FilesPicked(ps.unwrap_or_default())),
            );
        }
        IdMessage::FilesPicked(paths) => {
            if paths.is_empty() {
                return Task::none();
            }
            // 参考项目 batch：多选导入 → 队列逐张处理（排队中 → 处理中 → 完成/失败）
            s.queue = paths
                .iter()
                .map(|p| crate::app::IdQueueItem {
                    path: p.clone(),
                    stem: p
                        .file_stem()
                        .map(|x| x.to_string_lossy().to_string())
                        .unwrap_or_else(|| "photo".to_string()),
                    status: crate::app::QueueStatus::Pending,
                })
                .collect();
            s.queue[0].status = crate::app::QueueStatus::Running;
            s.queue_active = Some(0);
            s.decode_inflight = Some(0);
            app.toast(trf("已导入 {} 张照片，开始逐张处理", &[&paths.len().to_string()]));
            let first = load_id_path(paths[0].clone());
            // 流水线：立即预取解码第 2 张，与第 1 张的抠图并行
            if paths.len() > 1 {
                let p2 = prefetch_decode(1, paths[1].clone());
                return Task::batch([first, p2]);
            }
            return first;
        }
        IdMessage::FilePicked(Some(path)) => {
            s.decode_inflight = None;
            s.loading = true;
            return load_id_path(path);
        }
        IdMessage::FilePicked(None) => return Task::none(),
        IdMessage::PhotoLoaded(Ok(payload)) => return start_photo(app, *payload),
        #[allow(dead_code)]
        IdMessage::MemoryPhoto(payload) => {
            s.src_path = None;
            s.loading = false;
            let t = imgcore::tex_display(&payload.rgba);
            s.src_handle = Some(Handle::from_rgba(t.w, t.h, t.bytes));
            s.busy = true;
            return matte_payload(app, *payload);
        }
        IdMessage::PhotoLoaded(Err(e)) => {
            s.decode_inflight = None;
            s.loading = false;
            app.toast_err(trf("打开失败：{}", &[&e]));
            return advance_queue(app, false);
        }
        IdMessage::PrefetchLoaded(i, r) => {
            // 只接「下一张待处理项」的预取；与同步解码撞车或槽已占用则丢弃
            let expected = s.queue_active.map(|a| a + 1);
            let racing = s.decode_inflight == Some(i);
            if expected == Some(i) && !racing && s.prefetch.is_none() {
                s.prefetch = Some((i, r));
            }
            return Task::none();
        }
        IdMessage::MatteDone(Ok(payload)) => {
            // 移动语义：payload 里的 fg 直接进 Arc（原实现多一次全分辨率 clone）
            s.fg = Some(std::sync::Arc::new(payload.rgba));
            s.fg_preview = None;
            s.fg_eroded = None;
            s.erode_cache_key = None; // 换照片：预览副本与腐蚀缓存全部作废
            s.busy = false;
            app.toast(tr("抠图完成，已按规格构图"));
            app.persist();
            return Task::batch([
                render(app),
                advance_queue(app, true),
                crate::app::schedule_session_sweep(),
            ]);
        }
        IdMessage::MatteDone(Err(e)) => {
            s.busy = false;
            app.toast_err(trf("抠图失败：{}", &[&e]));
            return advance_queue(app, false);
        }
        IdMessage::SetSpec(i) => {
            s.spec_idx = i.min(ID_SIZES.len() - 1);
            app.persist();
            return debounce_render();
        }
        IdMessage::SetBg(i) => {
            s.bg_idx = i.min(ID_BGS.len() - 1);
            app.persist();
            return debounce_render();
        }
        IdMessage::SetRender(r) => {
            s.render = r;
            s.erode_cache_key = None;
            app.persist();
            return debounce_render();
        }
        IdMessage::SetZoom(v) => {
            s.zoom = v.clamp(80, 200);
            app.persist();
            return debounce_render();
        }
        IdMessage::SetOffY(v) => {
            // 与滑块范围、compute_align 的 off_y 钳制（±45）一致
            s.off_y = v.clamp(-45, 45);
            app.persist();
            return debounce_render();
        }
        IdMessage::SetErode(v) => {
            s.erode = v.clamp(0, 3);
            s.erode_cache_key = None;
            app.persist();
            return debounce_render();
        }
        IdMessage::SetPaper(i) => {
            s.paper_idx = i.min(ID_PAPERS.len() - 1);
            app.persist();
            return debounce_render();
        }
        IdMessage::SetModel(id) => {
            // 空串 = 标签未命中，视为清除显式选择（回退 modnet.onnx）
            s.model_id = if id.is_empty() { None } else { Some(id) };
            let src_path = s.src_path.clone();
            if src_path.is_some() {
                s.busy = true;
                s.loading = true;
            }
            app.persist();
            // 切模型立即生效：有源文件就重跑 载入→抠图 链（后台，UI 不冻结）
            if let Some(p) = src_path {
                return load_id_path(p);
            }
            // 内存来源（示例图等）无原图可重跑，下次载入时生效
            return Task::none();
        }
        IdMessage::ToggleCutline(v) => {
            s.cutline = v;
            app.persist();
            return debounce_render();
        }
        IdMessage::ToggleRealsize(v) => {
            s.realsize = v;
            app.persist();
            return Task::none();
        }
        IdMessage::SetView(v) => {
            // 单张/排版并行渲染后两份 handle 常驻缓存，切换零重算；
            // 若另一视图尚在渲染中，回调到达时自动补上
            s.view = v;
            return Task::none();
        }
        IdMessage::RenderNow => {
            return render(app);
        }
        IdMessage::SetRotate(v) => {
            s.rot = v.clamp(-ROT_MAX, ROT_MAX);
            return debounce_render();
        }
        IdMessage::ToggleCustomBg(v) => {
            s.use_custom_bg = v;
            app.persist();
            return debounce_render();
        }
        IdMessage::SetCustomBg(hex) => {
            s.custom_bg_hex = hex;
            // 输入法逐键触发，防抖避免打字期间连续重渲染；不落盘（完成后再说）
            return debounce_render();
        }
        IdMessage::SetDpi(v) => {
            s.dpi = v;
            app.persist();
            return debounce_render();
        }
        IdMessage::ExportSingle => {
            let Some(fg) = app.id.fg.as_ref().map(std::sync::Arc::clone) else {
                app.toast(tr("还没有可导出的成品"));
                return Task::none();
            };
            // 合成 + 编码全部在后台线程（此前在 update 里同步执行，
            // A4 300DPI 的 PNG 编码可冻结 UI 数秒）
            let spec_idx = app.id.spec_idx;
            let erode = app.id.erode;
            let render_mode = app.id.render;
            let zoom = app.id.zoom;
            let off_y = app.id.off_y;
            let dpi = app.id.dpi;
            let fmt = app.id.export_fmt;
            let rot = app.id.rot;
            let (bg_color, _bg_name, _bg_desc) = resolve_bg(&app.id);
            let stem = app.id.stem.clone();
            return crate::ui::page_work::blocking_task(
                move || {
                    // 导出必须走全分辨率 fg（预览副本只用于滑块渲染链），
                    // 腐蚀缩放比 1.0；一次性成本可接受，不占用预览缓存
                    let fg_eroded = std::sync::Arc::new(compute_eroded(&fg, erode, render_mode, 1.0));
                    let fg_rot = apply_rot(&fg_eroded, rot);
                    let img = compose_single(
                        &fg_rot,
                        &ID_SIZES[spec_idx],
                        bg_color,
                        render_mode,
                        zoom,
                        off_y,
                        dpi,
                    );
                    imgcore::save_to_memory(&img, fmt, None)
                        .map(|bytes| crate::message::ExportReady {
                            name: format!("{}_idphoto.{}", stem, fmt.ext()),
                            bytes,
                        })
                        .map_err(|e| e.to_string())
                },
                |r: Option<Result<crate::message::ExportReady, String>>| {
                    Message::IdPhoto(IdMessage::SaveReady(Box::new(match r {
                        Some(Ok(x)) => Ok(x),
                        Some(Err(e)) => Err(e),
                        None => Err(tr("导出线程异常退出").to_string()),
                    })))
                },
            );
        }
        IdMessage::ExportSheet => {
            let Some(fg) = app.id.fg.as_ref().map(std::sync::Arc::clone) else {
                app.toast(tr("还没有可导出的成品"));
                return Task::none();
            };
            // 单张合成 + 排版 + JPG 编码全部后台线程
            let spec_idx = app.id.spec_idx;
            let erode = app.id.erode;
            let render_mode = app.id.render;
            let zoom = app.id.zoom;
            let off_y = app.id.off_y;
            let dpi = app.id.dpi;
            let paper_idx = app.id.paper_idx;
            let cutline = app.id.cutline;
            let rot = app.id.rot;
            let (bg_color, _n, _d) = resolve_bg(&app.id);
            let stem = app.id.stem.clone();
            return crate::ui::page_work::blocking_task(
                move || {
                    // 导出走全分辨率 fg（腐蚀缩放比 1.0），排版 JPG 编码全后台
                    let fg_eroded = std::sync::Arc::new(compute_eroded(&fg, erode, render_mode, 1.0));
                    let fg_rot = apply_rot(&fg_eroded, rot);
                    let img = compose_single(
                        &fg_rot,
                        &ID_SIZES[spec_idx],
                        bg_color,
                        render_mode,
                        zoom,
                        off_y,
                        dpi,
                    );
                    let (mw, mh) = ID_SIZES[spec_idx].mm();
                    let paper = ID_PAPERS[paper_idx.min(ID_PAPERS.len() - 1)];
                    let (canvas, _plan) = make_layout(&img, &paper, dpi, mw, mh, cutline);
                    imgcore::save_to_memory(&canvas, ExportFormat::Jpg, Some([255, 255, 255]))
                        .map(|bytes| crate::message::ExportReady {
                            name: format!("{}_idphoto-sheet.jpg", stem),
                            bytes,
                        })
                        .map_err(|e| e.to_string())
                },
                |r: Option<Result<crate::message::ExportReady, String>>| {
                    Message::IdPhoto(IdMessage::SaveReady(Box::new(match r {
                        Some(Ok(x)) => Ok(x),
                        Some(Err(e)) => Err(e),
                        None => Err(tr("导出线程异常退出").to_string()),
                    })))
                },
            );
        }
        IdMessage::SaveReady(ready) => {
            match *ready {
                Ok(export) => return save_bytes_task(export.name, export.bytes),
                Err(e) => app.toast_err(trf("导出失败：{}", &[&e])),
            }
        }
        IdMessage::SetExportFormat(f) => {
            s.export_fmt = f;
            app.persist();
        }
        IdMessage::BatchExport => {
            if s.batch_exporting {
                app.toast(tr("批量导出正在进行中，请稍候"));
                return Task::none();
            }
            if s.queue.is_empty() {
                app.toast(tr("队列为空：请先「批量导入」照片"));
                return Task::none();
            }
            return Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_folder()
                        .await
                        .map(|h| h.path().to_path_buf())
                },
                |p| Message::IdPhoto(IdMessage::BatchExportFolder(p)),
            );
        }
        IdMessage::BatchExportFolder(Some(folder)) => {
            let jobs: Vec<(usize, std::path::PathBuf)> = s
                .queue
                .iter()
                .enumerate()
                .map(|(i, q)| (i, q.path.clone()))
                .collect();
            if jobs.is_empty() {
                app.toast(tr("队列为空"));
                return Task::none();
            }
            // 模型与参数快照：批量线程内不回读 App，避免竞争
            let model_info = if let Some(ref mid) = s.model_id {
                app.work.nn_models.iter().find(|m| &m.id == mid).cloned()
            } else {
                app.work.nn_models.iter().find(|m| m.file == "modnet.onnx").cloned()
            };
            let params = app.work.params.clone();
            // 队列徽标全部复位为「排队中」，随后由 BatchEvent 逐张推进
            for q in s.queue.iter_mut() {
                q.status = crate::app::QueueStatus::Pending;
            }
            s.queue_active = None;
            s.batch_exporting = true;
            return batch_export_task(
                jobs,
                model_info,
                params,
                s.spec_idx,
                s.bg_idx,
                s.erode,
                s.render,
                s.zoom,
                s.off_y,
                s.dpi,
                s.rot,
                s.use_custom_bg,
                s.custom_bg_hex.clone(),
                folder,
                s.export_fmt,
            );
        }
        IdMessage::BatchExportFolder(None) => {}
        IdMessage::BatchEvent(ev) => match ev {
            BatchExportEvent::Start { idx } => {
                if let Some(q) = s.queue.get_mut(idx) {
                    q.status = crate::app::QueueStatus::Running;
                }
            }
            BatchExportEvent::Done { idx } => {
                if let Some(q) = s.queue.get_mut(idx) {
                    q.status = crate::app::QueueStatus::Done;
                }
            }
            BatchExportEvent::Fail { idx, err } => {
                if let Some(q) = s.queue.get_mut(idx) {
                    q.status = crate::app::QueueStatus::Failed;
                }
                let stem = s.queue.get(idx).map(|q| q.stem.clone()).unwrap_or_default();
                app.toast_err(trf("「{}」导出失败：{}", &[&stem, &err]));
            }
            BatchExportEvent::Finished { ok, fail, folder } => {
                s.batch_exporting = false;
                if fail == 0 {
                    app.toast(trf("批量导出完成：{} 张 → {}", &[&ok.to_string(), &folder]));
                } else {
                    app.toast_err(trf("批量导出结束：成功 {} · 失败 {} → {}", &[&ok.to_string(), &fail.to_string(), &folder]));
                }
                return crate::app::schedule_session_sweep();
            }
        }
        IdMessage::Saved(Ok(p)) => {
            app.toast(trf("已导出 {}", &[&p]));
            return Task::none();
        }
        IdMessage::Saved(Err(e)) => {
            if e != tr("已取消") {
                app.toast_err(format!("导出失败：{e}"));
            }
            return Task::none();
        }
        IdMessage::BatchProc => {
            // 队列状态一览（批量导入时已自动逐张处理，此按钮只做状态汇总）
            let n = s.queue.len();
            if n == 0 {
                app.toast(tr("请先批量导入照片"));
            } else {
                let (done, fail, pending) = s.queue.iter().fold((0, 0, 0), |(d, f, p), q| {
                    match q.status {
                        crate::app::QueueStatus::Done => (d + 1, f, p),
                        crate::app::QueueStatus::Failed => (d, f + 1, p),
                        _ => (d, f, p + 1),
                    }
                });
                app.toast(trf("队列状态：{} 完成 · {} 失败 · {} 排队中（共 {} 张）", &[&done.to_string(), &fail.to_string(), &pending.to_string(), &n.to_string()]));
            }
        }
        IdMessage::QueueLoad(i) => {
            if i < s.queue.len() {
                // 有正在处理的项则先退回排队
                if let Some(cur) = s.queue_active {
                    if cur < s.queue.len() && s.queue[cur].status == crate::app::QueueStatus::Running {
                        s.queue[cur].status = crate::app::QueueStatus::Pending;
                    }
                }
                s.queue[i].status = crate::app::QueueStatus::Running;
                s.queue_active = Some(i);
                s.decode_inflight = Some(i);
                let path = s.queue[i].path.clone();
                return load_id_path(path);
            }
            return Task::none();
        }
        IdMessage::QueueRemove(i) => {
            if i < s.queue.len() {
                s.queue.remove(i);
                s.queue_active = s.queue_active.and_then(|a| match a.cmp(&i) {
                    std::cmp::Ordering::Equal => None,
                    std::cmp::Ordering::Greater => Some(a - 1),
                    std::cmp::Ordering::Less => Some(a),
                });
            }
            return Task::none();
        }
        IdMessage::RenderDoneWithCache(payload) => {
            if payload.seq != s.render_seq {
                return Task::none();
            }
            if let Some(p) = payload.preview {
                s.fg_preview = Some(p);
            }
            if let (Some(fg_e), Some(key)) = (payload.fg_eroded, payload.erode_key) {
                s.fg_eroded = Some(fg_e);
                s.erode_cache_key = Some(key);
            }
            let (w, h) = (payload.img.width(), payload.img.height());
            let meta = payload.meta;
            // 预览降采样已在后台线程完成（preview_scale），这里直接建 Handle
            match payload.view {
                IdView::Single => {
                    s.out_handle = Some(Handle::from_rgba(w, h, payload.img.as_raw().clone()));
                    s.single_meta = meta.clone();
                }
                IdView::Sheet => {
                    s.sheet_handle = Some(Handle::from_rgba(w, h, payload.img.as_raw().clone()));
                    s.sheet_meta = meta.clone();
                }
            }
            s.meta = meta.clone();
        }
    }
    Task::none()
}

/// 批量队列推进：标记当前项结果，并取出下一个排队项继续处理。
/// `queue_active` 为 None（非队列来源的载入）时只做无副作用推进。
fn advance_queue(app: &mut App, ok: bool) -> Task<Message> {
    // 块内完成全部状态读写，出块后 s 借用结束（后面要拿 &mut App 调 start_photo）
    let (consumed, fallback) = {
        let s = &mut app.id;
        if let Some(i) = s.queue_active {
            if i < s.queue.len() {
                s.queue[i].status = if ok {
                    crate::app::QueueStatus::Done
                } else {
                    crate::app::QueueStatus::Failed
                };
            }
        }
        s.queue_active = None;
        let next = s.queue.iter().position(|q| q.status == crate::app::QueueStatus::Pending);
        let mut consumed = None;
        let mut fallback = None;
        if let Some(j) = next {
            s.queue[j].status = crate::app::QueueStatus::Running;
            s.queue_active = Some(j);
            // 预取命中：第 j 张已解码完毕，跳过解码直接进抠图
            if let Some((pi, r)) = s.prefetch.take() {
                if pi == j {
                    consumed = Some(r);
                }
            }
            if consumed.is_none() {
                // 预取未到：走同步解码路径（预取结果到达后会因撞车被丢弃）
                s.decode_inflight = Some(j);
                fallback = Some((j, s.queue[j].path.clone()));
            }
        }
        (consumed, fallback)
    };
    if let Some(r) = consumed {
        return match r {
            Ok(p) => start_photo(app, *p),
            Err(e) => {
                app.toast_err(format!("打开失败：{e}"));
                advance_queue(app, false)
            }
        };
    }
    if let Some((_, path)) = fallback {
        return load_id_path(path);
    }
    Task::none()
}

fn to_payload(l: imgcore::LoadedImage) -> LoadedPayload {
    LoadedPayload { rgba: l.rgba, path: l.path, stem: l.stem }
}

/// 后台预取解码：批量流水线的解码级（结果进单槽，解码 N+1 ∥ 抠图 N）
fn prefetch_decode(idx: usize, path: std::path::PathBuf) -> Task<Message> {
    crate::ui::page_work::blocking_task(
        move || {
            imgcore::load_file(&path)
                .map(to_payload)
                .map(Box::new)
                .map_err(|e| e.to_string())
        },
        move |r: Option<Result<Box<LoadedPayload>, String>>| {
            Message::IdPhoto(IdMessage::PrefetchLoaded(
                idx,
                match r {
                    Some(Ok(v)) => Ok(v),
                    Some(Err(e)) => Err(e),
                    None => Err(tr("预取线程异常退出").to_string()),
                },
            ))
        },
    )
}

/// 载入成功的公共后半段：建句柄、发起抠图，并按需预取解码下一张。
/// 批量模式下形成「解码 N+1 ∥ 抠图 N」的流水线。
fn start_photo(app: &mut App, payload: LoadedPayload) -> Task<Message> {
    let s = &mut app.id;
    // 源路径只记引用（切模型重跑链用），内存来源为 None
    s.loading = false;
    s.src_path = payload.path.clone();
    let t = imgcore::tex_display(&payload.rgba);
    s.src_handle = Some(Handle::from_rgba(t.w, t.h, t.bytes));
    s.busy = true;
    s.decode_inflight = None;
    // 非批量来源：清掉过期预取槽（释放 ~80MB）
    if s.queue_active.is_none() {
        s.prefetch = None;
    }
    // 批量模式：预取解码下一张（槽里已命中则不再重复发起）
    let mut prefetch_task: Option<Task<Message>> = None;
    let next = s.queue_active.map(|i| i + 1).filter(|&j| j < s.queue.len());
    if let Some(j) = next {
        let hit = s.prefetch.as_ref().map(|(pi, _)| *pi == j).unwrap_or(false);
        if !hit {
            let p = s.queue[j].path.clone();
            prefetch_task = Some(prefetch_decode(j, p));
        }
    }
    let matte_task = matte_payload(app, payload);
    match prefetch_task {
        Some(t) => Task::batch([matte_task, t]),
        None => matte_task,
    }
}

/// 后台读图并送入抠图链路（选择 / 批量导入首张 / 示例图片共用）
pub fn load_id_path(path: std::path::PathBuf) -> Task<Message> {
    crate::ui::page_work::blocking_task(
        move || {
            // 性能诊断：COCOLE_DIAG=1 时输出解码耗时（导入阶段的主要开销）
            let diag = std::env::var("COCOLE_DIAG").is_ok_and(|v| !v.is_empty() && v != "0");
            let t0 = std::time::Instant::now();
            let r = imgcore::load_file(&path).map_err(|e| e.to_string());
            if diag {
                eprintln!("[diag] 导入解码 {:?} ({})", t0.elapsed(), path.display());
            }
            r
        },
        |r| {
            Message::IdPhoto(IdMessage::PhotoLoaded(match r {
                Some(Ok(l)) => Ok(Box::new(to_payload(l))),
                Some(Err(e)) => Err(e),
                None => Err(tr("读取线程异常退出").to_string()),
            }))
        },
    )
}

/// 从内嵌字节加载示例图片（不依赖运行时文件系统）
pub fn load_id_bytes(data: &'static [u8], name: &'static str) -> Task<Message> {
    crate::ui::page_work::blocking_task(
        move || {
            imgcore::load_bytes(data, name).map_err(|e| e.to_string())
        },
        |r| {
            Message::IdPhoto(IdMessage::PhotoLoaded(match r {
                Some(Ok(l)) => Ok(Box::new(to_payload(l))),
                Some(Err(e)) => Err(e),
                None => Err(tr("读取线程异常退出").to_string()),
            }))
        },
    )
}

/// 防抖触发渲染：滑块拖动期间只更新状态值，延迟 DEBOUNCE_MS 后才真正 render。
/// 每次调用都重新计时，实现「最后一次拖动后 100ms 才渲染」的效果。
fn debounce_render() -> Task<Message> {
    Task::perform(
        tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)),
        |_| Message::IdPhoto(IdMessage::RenderNow),
    )
}

/// 排版预览画布上限（等比缩放口径）：纸/瓦片/间距同乘 sc，瓦片数与布局
/// 和导出完全一致，仅分辨率降低。屏幕上排版预览最终显示约 600×850，
/// 1200 长边已留 2× 余量 —— 相比全尺寸 A4 300DPI（8.7MP）省约 8 倍绘制量。
const LAY_PREVIEW_SIDE: f32 = 1200.0;
const LAY_PREVIEW_PIXELS: f32 = 1_500_000.0;

/// 双视图并行渲染的共享参数（Arc 字段克隆廉价，两视图各持一份）
#[derive(Clone)]
struct RenderJob {
    seq: u64,
    cancel_rx: tokio::sync::watch::Receiver<u64>,
    fg: std::sync::Arc<image::RgbaImage>,
    preview_cached: Option<std::sync::Arc<image::RgbaImage>>,
    fg_eroded_cached: Option<std::sync::Arc<image::RgbaImage>>,
    spec_idx: usize,
    erode: i32,
    render_mode: RenderMode,
    zoom: i32,
    off_y: i32,
    dpi: u32,
    paper_idx: usize,
    cutline: bool,
    rot: i32,
    bg_color: [u8; 3],
}

/// 渲染输出载荷（后台闭包 → UI 回调的结构化传递）
struct RenderOut {
    fg_eroded: std::sync::Arc<image::RgbaImage>,
    view: IdView,
    img: Box<image::RgbaImage>,
    meta: String,
    /// 首次渲染构建的预览工作副本（None = 复用已有）
    preview: Option<std::sync::Arc<image::RgbaImage>>,
    erode_key: (i32, RenderMode, u32, u32),
}

/// 渲染完成回调（两视图共用）：打包 RenderDonePayload
fn render_done(seq: u64, view: IdView, out: Option<RenderOut>) -> Message {
    let out = out.unwrap_or_else(|| RenderOut {
        fg_eroded: std::sync::Arc::new(image::RgbaImage::new(1, 1)),
        view,
        img: Box::new(image::RgbaImage::new(1, 1)),
        meta: String::new(),
        preview: None,
        erode_key: (0, RenderMode::Solid, 0, 0),
    });
    Message::IdPhoto(IdMessage::RenderDoneWithCache(Box::new(
        crate::message::RenderDonePayload {
            seq,
            view: out.view,
            img: out.img,
            meta: out.meta,
            fg_eroded: Some(out.fg_eroded),
            erode_key: Some(out.erode_key),
            preview: out.preview,
        },
    )))
}

/// 按当前参数重算预览（异步后台渲染）。
///
/// 优化点：
/// - **单张 + 排版并行同时渲染**：切视图零等待，滑块拖动双视图同步响应；
///   序列号防乱序，过期帧自动丢弃
/// - 预览工作副本：腐蚀/旋转/合成全部在 ≤1600 长边的小图上跑
/// - 腐蚀缓存：erode/render 不变时复用；排版预览走缩放口径（省 8 倍绘制）
fn render(app: &mut App) -> Task<Message> {
    let Some(fg) = app.id.fg.as_ref().map(std::sync::Arc::clone) else {
        return Task::none();
    };

    app.id.render_seq += 1;
    let seq = app.id.render_seq;

    // 发送新序列号，后台线程通过比较序列号判断是否被取消
    let _ = app.id.cancel_tx.send(seq);

    let job = RenderJob {
        seq,
        cancel_rx: app.id.cancel_rx.clone(),
        fg,
        preview_cached: app.id.fg_preview.clone(),
        fg_eroded_cached: app.id.fg_eroded.clone(),
        spec_idx: app.id.spec_idx,
        erode: app.id.erode,
        render_mode: app.id.render,
        zoom: app.id.zoom,
        off_y: app.id.off_y,
        dpi: app.id.dpi,
        paper_idx: app.id.paper_idx,
        cutline: app.id.cutline,
        rot: app.id.rot,
        bg_color: resolve_bg(&app.id).0,
    };

    Task::batch([spawn_single_render(job.clone()), spawn_sheet_render(job)])
}

fn spawn_single_render(job: RenderJob) -> Task<Message> {
    let RenderJob {
        seq,
        cancel_rx,
        fg,
        preview_cached,
        fg_eroded_cached,
        spec_idx,
        erode,
        render_mode,
        zoom,
        off_y,
        dpi,
        rot,
        bg_color,
        ..
    } = job;
    crate::ui::page_work::blocking_task(
        move || {
            if *cancel_rx.borrow() > seq {
                return None;
            }
            let diag = std::env::var("COCOLE_DIAG").is_ok_and(|v| !v.is_empty() && v != "0");
            let t0 = std::time::Instant::now();
            // 预览工作副本：首次渲染构建（一次性），之后滑块渲染全在小图上跑
            let (work, built_preview) = match preview_cached {
                Some(p) => (p, None),
                None => {
                    let p = std::sync::Arc::new(make_preview(&fg));
                    (p.clone(), Some(p))
                }
            };
            let fg_eroded = match &fg_eroded_cached {
                Some(c)
                    if c.width() == work.width() && c.height() == work.height() =>
                {
                    std::sync::Arc::clone(c)
                }
                _ => std::sync::Arc::new(compute_eroded(
                    &work,
                    erode,
                    render_mode,
                    preview_px_scale(&work, &fg),
                )),
            };
            let t_erode = t0.elapsed();
            if *cancel_rx.borrow() > seq {
                return None;
            }
            let spec = &ID_SIZES[spec_idx];
            let fg_rot = apply_rot(&fg_eroded, rot);
            let t_rot = t0.elapsed() - t_erode;
            let img = compose_single(&fg_rot, spec, bg_color, render_mode, zoom, off_y, dpi);
            let t_compose = t0.elapsed() - t_erode - t_rot;
            if diag {
                eprintln!(
                    "[diag] idphoto single seq={seq} work={}x{}: 腐蚀 {:?} 旋转 {:?} 合成 {:?}",
                    work.width(),
                    work.height(),
                    t_erode,
                    t_rot,
                    t_compose
                );
            }
            let (w, h) = (img.width(), img.height());
            let meta = trf(
                "{} · {} · 输出 {}x{}px @{}DPI",
                &[
                    spec.n,
                    &spec.dim_text(),
                    &w.to_string(),
                    &h.to_string(),
                    &dpi.to_string(),
                ],
            );
            let img = preview_scale(img);
            Some(RenderOut {
                fg_eroded,
                view: IdView::Single,
                img: Box::new(img),
                meta,
                preview: built_preview,
                erode_key: (erode, render_mode, work.width(), work.height()),
            })
        },
        move |r: Option<Option<RenderOut>>| render_done(seq, IdView::Single, r.flatten()),
    )
}

fn spawn_sheet_render(job: RenderJob) -> Task<Message> {
    let RenderJob {
        seq,
        cancel_rx,
        fg,
        preview_cached,
        fg_eroded_cached,
        spec_idx,
        erode,
        render_mode,
        zoom,
        off_y,
        dpi,
        paper_idx,
        cutline,
        rot,
        bg_color,
        ..
    } = job;
    crate::ui::page_work::blocking_task(
        move || {
            if *cancel_rx.borrow() > seq {
                return None;
            }
            let diag = std::env::var("COCOLE_DIAG").is_ok_and(|v| !v.is_empty() && v != "0");
            let t0 = std::time::Instant::now();
            let (work, built_preview) = match preview_cached {
                Some(p) => (p, None),
                None => {
                    let p = std::sync::Arc::new(make_preview(&fg));
                    (p.clone(), Some(p))
                }
            };
            let fg_eroded = match &fg_eroded_cached {
                Some(c)
                    if c.width() == work.width() && c.height() == work.height() =>
                {
                    std::sync::Arc::clone(c)
                }
                _ => std::sync::Arc::new(compute_eroded(
                    &work,
                    erode,
                    render_mode,
                    preview_px_scale(&work, &fg),
                )),
            };
            let t_erode = t0.elapsed();
            if *cancel_rx.borrow() > seq {
                return None;
            }
            let spec = &ID_SIZES[spec_idx];
            let fg_rot = apply_rot(&fg_eroded, rot);
            let img = compose_single(&fg_rot, spec, bg_color, render_mode, zoom, off_y, dpi);
            let (mw, mh) = spec.mm();
            let paper = ID_PAPERS[paper_idx.min(ID_PAPERS.len() - 1)];
            // 排版预览走缩放口径：布局/瓦片数与导出一致，绘制量省约 8 倍
            let (sheet, plan) = make_layout_with_cap(
                &img,
                &paper,
                dpi,
                mw,
                mh,
                cutline,
                LAY_PREVIEW_SIDE,
                LAY_PREVIEW_PIXELS,
            );
            let meta = sheet_text(&paper, &plan);
            let t_layout = t0.elapsed() - t_erode;
            if diag {
                eprintln!(
                    "[diag] idphoto sheet seq={seq} work={}x{}: 腐蚀 {:?} 合成+排版 {:?}",
                    work.width(),
                    work.height(),
                    t_erode,
                    t_layout
                );
            }
            let sheet = preview_scale(sheet);
            Some(RenderOut {
                fg_eroded,
                view: IdView::Sheet,
                img: Box::new(sheet),
                meta,
                preview: built_preview,
                erode_key: (erode, render_mode, work.width(), work.height()),
            })
        },
        move |r: Option<Option<RenderOut>>| render_done(seq, IdView::Sheet, r.flatten()),
    )
}

/// 预览降采样（> 2MB 原始字节时等比缩小，限制 GPU 纹理体积）。
///
/// 必须在后台线程调用（render 的闭包内）——此前在 RenderDoneWithCache
/// 回调（UI 线程）里 resize，A4 排版 8.7MP 一缩就是上百毫秒，每次渲染
/// 完成都会卡一下 UI。
/// 预览工作副本长边（像素）：滑块渲染链（腐蚀/旋转/合成）全部在
/// 该尺寸上跑，全分辨率 fg 只在导出时使用。1600²≈10MB，单次渲染 <50ms。
const PREVIEW_LONG: u32 = 1600;

/// 生成预览工作副本：长边超过 PREVIEW_LONG 时 Lanczos3 降采样，否则原样克隆
fn make_preview(fg: &image::RgbaImage) -> image::RgbaImage {
    let long = fg.width().max(fg.height());
    if long <= PREVIEW_LONG {
        return fg.clone();
    }
    let scale = PREVIEW_LONG as f32 / long as f32;
    let pw = ((fg.width() as f32 * scale).round() as u32).max(1);
    let ph = ((fg.height() as f32 * scale).round() as u32).max(1);
    image::imageops::resize(fg, pw, ph, image::imageops::FilterType::Lanczos3)
}

/// 预览尺寸相对全分辨率的缩放比（长边比），用于腐蚀像素量换算
fn preview_px_scale(work: &image::RgbaImage, full: &image::RgbaImage) -> f32 {
    let wl = work.width().max(work.height()) as f32;
    let fl = full.width().max(full.height()) as f32;
    (wl / fl).clamp(0.05, 1.0)
}

fn preview_scale(img: image::RgbaImage) -> image::RgbaImage {
    const MAX_SYNC_SIZE: usize = 2 * 1024 * 1024;
    let raw_size = img.width() as usize * img.height() as usize * 4;
    if raw_size <= MAX_SYNC_SIZE {
        return img;
    }
    let scale = (MAX_SYNC_SIZE as f32 / raw_size as f32).sqrt();
    let pw = ((img.width() as f32 * scale).round() as u32).max(1);
    let ph = ((img.height() as f32 * scale).round() as u32).max(1);
    image::imageops::resize(&img, pw, ph, image::imageops::FilterType::Triangle)
}

/// 在后台线程中计算腐蚀后的前景图。
/// `px_scale`：当前工作图相对全分辨率的缩放比 —— 腐蚀像素量按比例换算
/// （预览图 1px ≈ 全分辨率 1/px_scale px），保证预览与导出的收缩视觉一致；
/// erode>0 时至少收 1px，避免预览完全不收缩。
fn compute_eroded(
    fg: &image::RgbaImage,
    erode: i32,
    render_mode: RenderMode,
    px_scale: f32,
) -> image::RgbaImage {
    if erode > 0 && render_mode != RenderMode::Original {
        let (w, h) = (fg.width() as usize, fg.height() as usize);
        let e = (((erode as f32) * px_scale).round() as usize).max(1);
        let mask: Vec<u8> = fg.as_raw().chunks_exact(4).map(|p| p[3]).collect();
        let eroded = erode_mask(&mask, w, h, e);
        imgcore::apply_mask(fg, &eroded)
    } else {
        fg.clone()
    }
}

/// 后台编码完成后：弹保存对话框并写盘（UI 线程只做对话框与 IO）
fn save_bytes_task(name: String, bytes: Vec<u8>) -> Task<Message> {
    let size = if bytes.len() >= 1024 * 1024 {
        format!("{:.1} MB", bytes.len() as f64 / 1048576.0)
    } else {
        format!("{:.0} KB", bytes.len() as f64 / 1024.0)
    };
    Task::perform(
        async move {
            let h = rfd::AsyncFileDialog::new().set_file_name(&name).save_file().await?;
            h.write(&bytes).await.ok()?;
            Some(h.path().to_path_buf())
        },
        move |p| {
            Message::IdPhoto(IdMessage::Saved(match p {
                // 体积随手报给用户（报名系统常有 KB 上限要求）
                Some(path) => Ok(format!("{}（{size}）", path.to_string_lossy())),
                None => Err(tr("已取消").to_string()),
            }))
        },
    )
}

/* ═════════════════ 批量导出 ═════════════════ */

/// 批量导出流水线：后台线程逐张 读图 → 抠图 → 构图合成 → 编码 → 写盘，
/// 进度经 Task::stream 保序回传（与自更新下载任务同一套模式）。
#[allow(clippy::too_many_arguments)]
fn batch_export_task(
    jobs: Vec<(usize, std::path::PathBuf)>,
    model_info: Option<crate::core::nn::NnModelInfo>,
    matte_params: matting::MatteParams,
    spec_idx: usize,
    bg_idx: usize,
    erode: i32,
    render_mode: RenderMode,
    zoom: i32,
    off_y: i32,
    dpi: u32,
    rot: i32,
    use_custom_bg: bool,
    custom_bg_hex: String,
    folder: std::path::PathBuf,
    fmt: ExportFormat,
) -> Task<Message> {
    // 底色解析（与预览 resolve_bg 同语义，线程内自足）
    let bg_color = if use_custom_bg {
        parse_hex(&custom_bg_hex)
    } else {
        parse_hex(ID_BGS[bg_idx.min(ID_BGS.len() - 1)].c)
    };
    Task::stream(iced::stream::channel(8, async move |mut out| {
        use iced::futures::SinkExt;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BatchExportEvent>();
        let handle = std::thread::spawn(move || {
            let mut ok = 0usize;
            let mut fail = 0usize;
            for (idx, path) in jobs {
                let _ = tx.send(BatchExportEvent::Start { idx });
                match run_batch_one(
                    &path,
                    &folder,
                    fmt,
                    model_info.as_ref(),
                    &matte_params,
                    spec_idx,
                    bg_color,
                    erode,
                    render_mode,
                    zoom,
                    off_y,
                    dpi,
                    rot,
                ) {
                    Ok(out_path) => {
                        ok += 1;
                        eprintln!("[batch] ✓ {out_path}");
                        let _ = tx.send(BatchExportEvent::Done { idx });
                    }
                    Err(e) => {
                        fail += 1;
                        let _ = tx.send(BatchExportEvent::Fail { idx, err: e });
                    }
                }
            }
            let _ = tx.send(BatchExportEvent::Finished {
                ok,
                fail,
                folder: folder.to_string_lossy().to_string(),
            });
        });
        while let Some(ev) = rx.recv().await {
            let _ = out.send(Message::IdPhoto(IdMessage::BatchEvent(ev))).await;
        }
        let _ = handle.join();
    }))
}

/// 单张批量项：读图 → 抠图（NN 或色键回退）→ 边缘收缩 → 旋转校正 → 构图合成 → 编码 → 写盘
#[allow(clippy::too_many_arguments)]
fn run_batch_one(
    path: &std::path::Path,
    folder: &std::path::Path,
    fmt: ExportFormat,
    model_info: Option<&crate::core::nn::NnModelInfo>,
    matte_params: &matting::MatteParams,
    spec_idx: usize,
    bg_color: [u8; 3],
    erode: i32,
    render_mode: RenderMode,
    zoom: i32,
    off_y: i32,
    dpi: u32,
    rot: i32,
) -> Result<String, String> {
    let loaded = imgcore::load_file(path).map_err(|e| e.to_string())?;
    let (w, h) = (loaded.rgba.width() as usize, loaded.rgba.height() as usize);

    // 抠图（与单张链路同一套模型选择 / 回退逻辑）
    let fg = if let Some(info) = model_info {
        match nn::matte(info, loaded.rgba.as_raw(), w, h) {
            Ok(mask) => imgcore::apply_mask(&loaded.rgba, &mask),
            Err(e) => return Err(trf("抠图失败：{}", &[&e])),
        }
    } else {
        let mut cache = matting::FieldCache::default();
        let res = matting::compute_mask(loaded.rgba.as_raw(), w, h, matte_params, &mut cache);
        imgcore::apply_mask(&loaded.rgba, &res.mask)
    };

    // 边缘收缩（与 compute_eroded 同语义）
    let fg = if erode > 0 && render_mode != RenderMode::Original {
        let mask: Vec<u8> = fg.as_raw().chunks_exact(4).map(|p| p[3]).collect();
        let eroded = erode_mask(&mask, w, h, erode as usize);
        imgcore::apply_mask(&fg, &eroded)
    } else {
        fg
    };

    let img = compose_single(
        &apply_rot(&std::sync::Arc::new(fg), rot),
        &ID_SIZES[spec_idx],
        bg_color,
        render_mode,
        zoom,
        off_y,
        dpi,
    );
    let bytes = imgcore::save_to_memory(&img, fmt, None).map_err(|e| e.to_string())?;
    let out = folder.join(format!("{}_idphoto.{}", loaded.stem, fmt.ext()));
    std::fs::write(&out, &bytes).map_err(|e| trf("写入失败：{}", &[&e.to_string()]))?;
    Ok(out.to_string_lossy().to_string())
}

/* ═════════════════ view ═════════════════ */

pub fn view(app: &App) -> Element<'_, Message> {
    let s = &app.id;
    let has = s.fg.is_some();

    let head = column![
        text(tr("证件照")).size(22.0).font(bold_font()).color(TXT()),
        hint(tr("上传或拍摄人像照片，本地抠图换底、按规格出图，并可排版到相纸直接冲印。图片不上传服务器。")),
    ]
    .spacing(6);

    let body: Element<Message> = if !has {
        // 空状态（对应 .idp-empty / .dropzone.idp-drop）：虚线拖放区 +
        // 选择照片 / 拍摄双入口。拍摄依赖摄像头采集，原生版暂缺（置灰 + 说明）。
        //
        // 拖放区整块可点（此前只有下方「选择照片」小按钮绑定了选择器，
        // 大面积虚线区是纯装饰容器，点击无效）；有文件拖拽悬停在窗口上时
        // 虚线框改用强调色描边，给出「可松手」反馈（drop_hover 由全局
        // window::events() 订阅维护）。
        let drop_border = if app.drop_hover { ACC() } else { LINE_STRONG() };
        let drop_inner = column![
            icon("upload", 30.0, TXT3()),
            text(tr("点击选择，或把照片拖到这里")).size(FS).color(TXT()),
        ]
        .spacing(8)
        .align_x(Alignment::Center);
        let dropzone = button(
            container(dashed_box(drop_inner, drop_border, R_PANEL, None))
                .width(Length::Fixed(460.0))
                .height(Length::Fixed(200.0)),
        )
        .on_press(Message::IdPhoto(IdMessage::OpenPhoto))
        .padding(0)
        .style(|_t, _s| button::Style {
            background: None,
            text_color: TXT(),
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            snap: false,
        });
        container(
            column![
                dropzone,
                row![
                    button(
                        row![
                            icon("image-plus", 14.0, Color::WHITE),
                            text(tr("选择照片")).size(FS_SM).color(Color::WHITE),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::IdPhoto(IdMessage::OpenPhoto))
                    .padding(pad(9.0, 16.0, 9.0, 16.0))
                    .style(btn_primary),
                    // 拍摄依赖摄像头采集（原生版暂缺）：置灰禁用，不再伪装成
                    // 可点的主按钮误导用户
                    button(
                        row![
                            icon("camera", 14.0, TXT3()),
                            text(tr("拍摄")).size(FS_SM).color(TXT3()),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .padding(pad(9.0, 16.0, 9.0, 16.0))
                    .style(btn_secondary),
                ]
                .spacing(9)
                .align_y(Alignment::Center),
                hint(tr("拍摄依赖摄像头采集（原生版暂未提供），可先用本地照片。")),
                hint(tr("建议正脸、光线均匀、肩部放松；换底由内置 MODNet 人像模型在本地完成。")),
            ]
            .spacing(14)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        row![preview(app), side(app)].spacing(18).into()
    };

    let mut page_col = column![
        container(head).padding(pad(22.0, 24.0, 12.0, 24.0)),
        container(body).padding(pad(0.0, 24.0, 24.0, 24.0)),
    ]
    .spacing(10);
    // 批量队列面板（参考项目 batch 面板：状态徽标 + 载入 / 移除）
    if !s.queue.is_empty() {
        page_col = page_col.push(queue_panel(app));
    }

    let base = container(page_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(app_bg);

    // 推理中覆盖层（参考 `.processing` + `.proc-card`）：本地 ONNX 抠图期间
    // 在整页之上叠一层半透明遮罩 + 居中「正在抠图…」卡片，同时屏蔽误操作。
    // 不 busy 时不 push，Stack 只有基底层，零额外开销。
    let mut root = Stack::new()
        .width(Length::Fill)
        .height(Length::Fill)
        .push(base);
    if s.busy || s.loading {
        let (t, sub) = if s.loading {
            (tr("正在载入图片…"), "")
        } else {
            (tr("正在抠图…"), "MODNet")
        };
        root = root.push(crate::ui::widgets::busy_overlay(t, sub, s.frame));
    }
    root.into()
}


/// 批量队列面板（参考项目 batch 项：文件名 + 状态徽标 + 载入 / 移除）
fn queue_panel<'a>(app: &'a App) -> Element<'a, Message> {
    use crate::app::QueueStatus;
    let s = &app.id;
    let mut col = column![
        row![
            icon("layers", 15.0, TXT2()),
            text(tr("批量队列")).size(FS_H).color(TXT()).font(bold_font()),
            text(trf("{} 项", &[&s.queue.len().to_string()])).size(FS_SM).color(TXT3()),
            Space::new().width(Length::Fill),
            // 批量导出入口：一键把整个队列按当前规格/底色/格式导出到所选文件夹
            if s.batch_exporting {
                let badge: Element<'_, Message> = container(
                    text(tr("批量导出中…")).size(FS_XS).color(TXT3()),
                )
                .padding(pad(4.0, 9.0, 4.0, 9.0))
                .into();
                badge
            } else {
                button(text(tr("批量导出")).size(FS_XS).color(Color::WHITE))
                    .on_press(Message::IdPhoto(IdMessage::BatchExport))
                    .padding(pad(4.0, 9.0, 4.0, 9.0))
                    .style(btn_primary)
                    .into()
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(8);

    for (i, q) in s.queue.iter().enumerate() {
        let (fg, bg) = match q.status {
            QueueStatus::Pending => (TXT3(), PANEL_3()),
            QueueStatus::Running => (WARN(), WARN_SOFT()),
            QueueStatus::Done => (OK(), OK_SOFT()),
            QueueStatus::Failed => (DANGER(), DANGER_SOFT()),
        };
        col = col.push(
            container(
                row![
                    text(&q.stem).size(FS).color(TXT()),
                    Space::new().width(Length::Fill),
                    container(text(q.status.label()).size(FS_XS).color(fg))
                        .padding(pad(3.0, 8.0, 3.0, 8.0))
                        .style(badge_style(fg, bg)),
                    button(text(tr("载入")).size(FS_XS))
                        .on_press(Message::IdPhoto(IdMessage::QueueLoad(i)))
                        .padding(pad(4.0, 9.0, 4.0, 9.0))
                        .style(btn_secondary),
                    button(text(tr("移除")).size(FS_XS))
                        .on_press(Message::IdPhoto(IdMessage::QueueRemove(i)))
                        .padding(pad(4.0, 9.0, 4.0, 9.0))
                        .style(btn_secondary),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding(pad(SP_3, CARD_PAD, SP_3, CARD_PAD))
            .width(Length::Fill)
            .style(panel),
        );
    }

    container(col).padding(pad(0.0, 24.0, 0.0, 24.0)).width(Length::Fill).into()
}

/// 工具栏黑底小按钮（`.btn.btn-sm`：12px 白字 + 14px 白图标，高 30）
fn tool_btn<'a>(ic: &'static str, label: &'a str, msg: Message) -> Element<'a, Message> {
    button(
        row![
            icon(ic, 14.0, Color::WHITE),
            text(label.to_string()).size(FS_SM).color(Color::WHITE),
        ]
        .spacing(7)
        .align_y(Alignment::Center),
    )
    .on_press(msg)
    .padding(pad(7.0, 12.0, 7.0, 12.0))
    .style(btn_primary)
    .into()
}

/// 预览舞台样式（`.idp-stage`：`--panel-3` 衬底 + `--line` 描边 + 12px 圆角）
fn stage_style(_t: &Theme) -> container::Style {
    container::Style {
        text_color: Some(TXT()),
        background: Some(iced::Background::Color(PANEL_3())),
        border: iced::Border { color: LINE(), width: 1.0, radius: 12.0.into() },
        shadow: iced::Shadow::default(),
        snap: false,
    }
}

fn preview<'a>(app: &'a App) -> Element<'a, Message> {
    let s = &app.id;
    let spec = &ID_SIZES[s.spec_idx.min(ID_SIZES.len() - 1)];

    // 工具栏（`.idp-toolbar`）：导入 / 批量导入 / 队列状态（黑底主按钮一行）
    let toolbar = row![
        tool_btn("image-plus", tr("导入"), Message::IdPhoto(IdMessage::OpenPhoto)),
        tool_btn("images", tr("批量导入"), Message::IdPhoto(IdMessage::OpenBatch)),
        tool_btn("layers", tr("队列状态"), Message::IdPhoto(IdMessage::BatchProc)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    // 预览页签（`.seg`：按钮 flex:1 通栏平分）
    let tabs = seg_fill(&[
        (tr("单张"), s.view == IdView::Single, Message::IdPhoto(IdMessage::SetView(IdView::Single))),
        (tr("排版"), s.view == IdView::Sheet, Message::IdPhoto(IdMessage::SetView(IdView::Sheet))),
    ]);

    // 舞台内容：单张 → out_handle；排版 → sheet_handle
    let handle = if s.view == IdView::Sheet { s.sheet_handle.as_ref() } else { s.out_handle.as_ref() };
    let (img_elem, scale_note): (Element<Message>, String) = match handle {
        Some(h) => {
            let mut img =
                img_widget::Image::new(h.clone()).content_fit(iced::ContentFit::Contain);
            
            if s.realsize {
                let screen_dpi = s.screen_dpi;
                
                if s.view == IdView::Single {
                    let (mw, mh) = spec.mm();
                    let (raw_w, raw_h) = (mw * screen_dpi / 25.4, mh * screen_dpi / 25.4);
                    let sc = (640.0 / raw_h).min(1.0);
                    img = img
                        .width(Length::Fixed((raw_w * sc).max(40.0)))
                        .height(Length::Fixed((raw_h * sc).max(40.0)));
                    let note = if sc >= 0.999 {
                        tr("真实尺寸 1:1").to_string()
                    } else {
                        trf("真实尺寸（屏显 {}%）", &[&((sc * 100.0).round() as i32).to_string()])
                    };
                    (img.into(), note)
                } else {
                    let paper = ID_PAPERS[s.paper_idx.min(ID_PAPERS.len() - 1)];
                    let paper_w_px = paper.w * screen_dpi / 25.4;
                    let paper_h_px = paper.h * screen_dpi / 25.4;
                    
                    let max_display_w = 900.0;
                    let max_display_h = 700.0;
                    
                    let needs_scaling = paper_w_px > max_display_w || paper_h_px > max_display_h;
                    
                    let (display_w, display_h, display_scale) = if needs_scaling {
                        let scale_w = max_display_w / paper_w_px;
                        let scale_h = max_display_h / paper_h_px;
                        let sc = scale_w.min(scale_h);
                        (
                            (paper_w_px * sc).max(40.0),
                            (paper_h_px * sc).max(40.0),
                            sc,
                        )
                    } else {
                        (paper_w_px, paper_h_px, 1.0)
                    };
                    
                    img = img
                        .width(Length::Fixed(display_w))
                        .height(Length::Fixed(display_h));
                    
                    let note = if display_scale >= 0.999 {
                        trf("相纸真实尺寸 1:1（{}×{}mm）", &[&paper.w.to_string(), &paper.h.to_string()])
                    } else {
                        trf(
                            "相纸 {}×{}mm（屏显 {}%）",
                            &[
                                &paper.w.to_string(),
                                &paper.h.to_string(),
                                &((display_scale * 100.0).round() as i32).to_string(),
                            ],
                        )
                    };
                    (img.into(), note)
                }
            } else {
                (img.into(), String::new())
            }
        }
        None => (hint(tr("暂无预览")), String::new()),
    };

    // meta 文案行（`.idp-meta`）：真实尺寸比例并入同一行（参考 renderPreview 的拼接）
    let meta_line = if scale_note.is_empty() {
        s.meta.clone()
    } else {
        format!("{} · {}", s.meta, scale_note)
    };

    // 舞台（`.idp-stage`）：浅灰衬底，图片居中、meta 文案固定在底部
    let stage = container(
        column![
            container(img_elem)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            text(meta_line).size(FS_SM).color(TXT3()),
        ]
        .spacing(12)
        .align_x(Alignment::Center)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(22)
    .style(stage_style);

    column![toolbar, tabs, stage]
        .spacing(12)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/* ─────────── 动作区按钮 ─────────── */

/// 主按钮高度。参考项目 `.btn` 为 34px、`.btn-sm` 为 30px；
/// 按用户要求「适当降低高度」与底部小按钮统一，故不再用 `.btn` 的 34px。
const BTN_H: f32 = 30.0;
/// 小按钮高度（对应参考 `.btn-sm{height:30px}`，与主按钮等高）
const BTN_H_SM: f32 = 30.0;
/// 动作区左右内边距（与 `.sec{padding:16px}` 对齐，使按钮与规格/抠图等小节内容左右齐边）
const ACTION_PAD_X: f32 = 16.0;

/// 动作区按钮：内容在按钮内**水平 + 垂直居中**。
///
/// iced 的 `button` 走 `layout::padded`，会把子节点放到 padding 的**左上角**
/// （`content.move_to((padding.left, padding.top))`），按钮一旦被 `width/height`
/// 撑开，标签就会贴左上角。这里用 `container` 撑满按钮并把内容居中修正，
/// 同时固定高度 —— 对应参考 CSS `.btn{display:inline-flex;align-items:center;
/// justify-content:center;height:34px}`，也让同一行的两个小按钮等高。
fn action_btn<'a>(
    content: impl Into<Element<'a, Message>>,
    height: f32,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let inner = container(content.into())
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center);

    let btn = button(inner)
        .padding(0.0)
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .style(btn_primary);

    match on_press {
        Some(msg) => btn.on_press(msg).into(),
        None => btn.into(),
    }
}

fn side<'a>(app: &'a App) -> Element<'a, Message> {
    let s = &app.id;

    // 规格下拉：按用途分组前缀（国内通用/出入境/考试报名/证件办理），
    // 20 项平铺时找「研究生考试」这类条目要扫全表
    let spec_labels: Vec<String> =
        ID_SIZES.iter().map(|sp| format!("{}｜{}", sp.g, sp.n)).collect();
    let selected_spec_label =
        spec_labels.get(s.spec_idx.min(ID_SIZES.len() - 1)).cloned();
    let spec_labels_cb = spec_labels.clone();

    // 渲染模式下拉：5 种换底方式（参考 index.html `#idRender`，标签随语言翻译）
    let render_labels: Vec<String> =
        RenderMode::ALL.iter().map(|m| tr(m.label()).to_string()).collect();
    let render_labels_cb = render_labels.clone();
    let selected_render = tr(s.render.label()).to_string();
    let render_pick = pick_list(
        render_labels,
        Some(selected_render),
        move |label| {
            let idx = render_labels_cb.iter().position(|l| *l == label).unwrap_or(0);
            Message::IdPhoto(IdMessage::SetRender(RenderMode::ALL[idx]))
        },
    )
    .width(Length::Fill)
    .style(pick_list_style)
    .text_size(FS_SM)
    .menu_style(menu_style);

    // 当前底色显示（自定义 HEX 优先）
    let (bg_color_now, bg_name, bg_desc) = resolve_bg(s);
    let selected_swatch =
        if s.use_custom_bg { "" } else { ID_BGS[s.bg_idx.min(ID_BGS.len() - 1)].c };
    // 自定义底色行：开启时显示色块 + HEX 输入；关闭时显示当前底色说明
    let custom_bg_row: Element<'_, Message> = if s.use_custom_bg {
        let swatch =
            iced::Color::from_rgb8(bg_color_now[0], bg_color_now[1], bg_color_now[2]);
        row![
            container(Space::new().width(Length::Fixed(26.0)).height(Length::Fixed(26.0)))
                .width(Length::Fill)
                .style(move |_t: &Theme| container::Style {
                    background: Some(iced::Background::Color(swatch)),
                    border: iced::Border { color: LINE(), width: 1.0, radius: R_SM.into() },
                    ..container::Style::default()
                }),
            iced::widget::text_input("#RRGGBB", &s.custom_bg_hex)
                .on_input(|v| Message::IdPhoto(IdMessage::SetCustomBg(v)))
                .size(FS_SM)
                .width(Length::Fill)
                .style(text_input_style),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        text(bg_desc).size(FS_XS).color(TXT3()).into()
    };

    let spec_sec = section(
        tr("规格"),
        None,
        column![
            column![
                field_label(tr("证件规格"), "", ""),
                pick_list(
                    spec_labels,
                    selected_spec_label,
                    move |label| {
                        let idx =
                            spec_labels_cb.iter().position(|l| *l == label).unwrap_or(0);
                        Message::IdPhoto(IdMessage::SetSpec(idx))
                    },
                )
                .width(Length::Fill)
                .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
            ]
            .spacing(7),
            column![
                field_label(tr("相纸"), "", ""),
                pick_list(ID_PAPERS, Some(ID_PAPERS[s.paper_idx.min(ID_PAPERS.len() - 1)]), |p| {
                    Message::IdPhoto(IdMessage::SetPaper(index_of_paper(&p)))
                })
                .width(Length::Fill)
                .style(pick_list_style).text_size(FS_SM).menu_style(menu_style),
            ]
            .spacing(7),
            column![
                field_label(tr("底色"), bg_name, ""),
                swatch_grid(ID_BG_COLORS, selected_swatch, |c| {
                    Message::IdPhoto(IdMessage::SetBg(index_of_bg(&c)))
                }),
                // 自定义底色：开关 + HEX 输入（与工作台自定义颜色同一套交互）
                switch_row(
                    tr("自定义底色"),
                    s.use_custom_bg,
                    Message::IdPhoto(IdMessage::ToggleCustomBg(!s.use_custom_bg)),
                    None,
                ),
                custom_bg_row,
            ]
            .spacing(7),
            // 渲染：换底方式 + 随模式联动的说明（参考 index.html `#idRender`/`#idRenderDesc`）
            column![
                field_label(tr("渲染"), "", ""),
                render_pick,
                hint(tr(s.render.desc())),
            ]
            .spacing(7),
        ]
        .spacing(13),
    );

    // 抠图区：模型下拉列表，读取 nn_models 动态生成（过滤掉人脸检测模型）
    let matte_models: Vec<&crate::core::nn::NnModelInfo> = app.work.nn_models.iter()
        .filter(|m| {
            let lower = m.file.to_ascii_lowercase();
            !lower.contains("face") && !lower.contains("yunet")
        })
        .collect();
    // 选中显示「实际生效的模型」：显式选择 → modnet.onnx 回退 → 第一个，
    // 与 matte_payload 的回退链完全一致（默认 model_id=None 时不再显示空白）
    let effective_model = app.id.model_id.as_ref()
        .and_then(|mid| matte_models.iter().find(|m| &m.id == mid).copied())
        .or_else(|| matte_models.iter().find(|m| m.file == "modnet.onnx").copied())
        .or_else(|| matte_models.first().copied());
    let model_labels: Vec<String> = matte_models.iter().map(|m| m.label.clone()).collect();
    let selected_label = effective_model.map(|m| m.label.clone());
    let model_hint = if matte_models.is_empty() {
        tr("未发现模型 · 将使用色键回退").to_string()
    } else {
        tr("人像专用 · 发丝级边缘").to_string()
    };
    let model_id_map: Vec<(String, String)> = matte_models.iter()
        .map(|m| (m.label.clone(), m.id.clone())).collect();
    let model_pick = pick_list(
        model_labels,
        selected_label,
        move |label| {
            let id = model_id_map.iter()
                .find(|(l, _)| l == &label)
                .map(|(_, id)| id.clone())
                .unwrap_or_default();
            Message::IdPhoto(IdMessage::SetModel(id))
        },
    )
    .width(Length::Fill)
    .style(pick_list_style)
    .text_size(FS_SM)
    .menu_style(menu_style);
    let matte_sec = section(
        tr("抠图"),
        None,
        column![
            field_label(tr("模型"), "", ""),
            model_pick,
            hint(&model_hint),
        ]
        .spacing(7),
    );

    let compose_sec = section(
        tr("构图"),
        None,
        column![
            hint(tr("拖动滑块调整构图；旋转校正经顺时针为正")),
            slider_row(tr("垂直位置"), s.off_y, -45..=45, "%", |v| Message::IdPhoto(
                IdMessage::SetOffY(v)
            )),
            slider_row(tr("缩放"), s.zoom, 80..=200, "%", |v| Message::IdPhoto(
                IdMessage::SetZoom(v)
            )),
            // 旋转校正：头歪修正（正 = 顺时针）
            slider_row(tr("旋转校正"), s.rot, -ROT_MAX..=ROT_MAX, "°", |v| Message::IdPhoto(
                IdMessage::SetRotate(v)
            )),
            slider_row(tr("边缘收缩"), s.erode, 0..=3, "px", |v| Message::IdPhoto(
                IdMessage::SetErode(v)
            )),
            hint(tr("换底后压掉发丝外的原背景残边")),
        ]
        .spacing(13),
    );

    // 预览区（参考 sec「预览」：真实尺寸开关 + 完整说明）
    let preview_sec = section(
        tr("预览"),
        None,
        column![
            switch_row(
                tr("真实尺寸（显示器 DPI）"),
                s.realsize,
                Message::IdPhoto(IdMessage::ToggleRealsize(!s.realsize)),
                None,
            ),
            hint(tr("按显示器物理 DPI 把规格还原成屏幕上的实际毫米大小；屏幕放不下时等比缩小并标出比例。导出分辨率由「输出 DPI」决定。")),
        ]
        .spacing(11),
    );

    let sheet_sec = section(
        tr("排版"),
        None,
        column![
            switch_row(
                tr("裁剪线"),
                s.cutline,
                Message::IdPhoto(IdMessage::ToggleCutline(!s.cutline)),
                None,
            ),
            // 输出 DPI：排版/单张导出的打印分辨率（毫米制规格换算像素；像素制规格固定）
            column![
                field_label(tr("输出 DPI"), "", ""),
                pick_list(vec![300u32, 600], Some(s.dpi), |v| {
                    Message::IdPhoto(IdMessage::SetDpi(v))
                })
                .width(Length::Fill)
                .style(pick_list_style)
                .text_size(FS_SM)
                .menu_style(menu_style),
                hint(tr("排版与单张导出的打印分辨率；仅影响毫米制规格，像素制规格尺寸固定不变。")),
            ]
            .spacing(7),
        ]
        .spacing(11),
    );

    // 动作区（`.idp-actions`）：导出格式选择 + 两个导出主按钮 + 重新选图 / 重新拍摄小按钮行。
    // 左右各留 16px，与其它小节的 `.sec` 内边距一致 —— 按钮不再贴住侧栏内缘。
    let fmt_row = row![
        text(tr("单张格式")).size(FS_SM).color(TXT2()),
        Space::new().width(Length::Fill),
        pick_list(ExportFormat::all(), Some(s.export_fmt), |f| {
            Message::IdPhoto(IdMessage::SetExportFormat(f))
        })
        .width(Length::Fixed(210.0))
        .style(pick_list_style)
        .text_size(FS_SM)
        .menu_style(menu_style),
    ]
    .align_y(Alignment::Center);
    let actions = container(
        column![
            fmt_row,
            action_btn(
                text(tr("导出单张 PNG")).size(FS_SM).color(Color::WHITE),
                BTN_H,
                Some(Message::IdPhoto(IdMessage::ExportSingle)),
            ),
            action_btn(
                text(tr("导出排版 JPG")).size(FS_SM).color(Color::WHITE),
                BTN_H,
                Some(Message::IdPhoto(IdMessage::ExportSheet)),
            ),
            row![
                action_btn(
                    text(tr("重新选图")).size(FS_SM).color(Color::WHITE),
                    BTN_H_SM,
                    Some(Message::IdPhoto(IdMessage::ChangePhoto)),
                ),
                // 摄像头采集未实现：置灰禁用（避免「点了没反应」）
                action_btn(
                    row![
                        icon("camera", 14.0, TXT3()),
                        text(tr("重新拍摄")).size(FS_SM).color(TXT3()),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                    BTN_H_SM,
                    None,
                ),
            ]
            .spacing(9),
        ]
        .spacing(8),
    )
    .padding(pad(2.0, ACTION_PAD_X, 2.0, ACTION_PAD_X));

    container(
        scrollable(
            column![spec_sec, matte_sec, compose_sec, preview_sec, sheet_sec, actions]
                .spacing(10)
                .padding(pad(2.0, 2.0, 20.0, 2.0)),
        )
        .direction(Direction::Vertical(Scrollbar::new().width(3).scroller_width(3)))
        .style(scrollbar_style),
    )
    .width(Length::Fixed(INSPECTOR_W))
    .height(Length::Fill)
    .style(panel)
    .into()
}

/// 7 种底色的色板（'static，避免把局部变量借进 Element）
const ID_BG_COLORS: &[(&str, Option<iced::Color>)] = &[
    ("#FFFFFF", Some(BG_WHITE)),
    ("#438EDB", Some(BG_BLUE)),
    ("#0066CC", Some(BG_CIVIL)),
    ("#64C5FF", Some(BG_CET)),
    ("#DCDCDC", Some(BG_GRAY)),
    ("#FF0000", Some(BG_RED)),
    ("#CC0000", Some(BG_ARCHIVE)),
];
const BG_WHITE: iced::Color = iced::Color::WHITE;
const BG_BLUE: iced::Color = iced::Color::from_rgb8(0x43, 0x8E, 0xDB);
const BG_CIVIL: iced::Color = iced::Color::from_rgb8(0x00, 0x66, 0xCC);
const BG_CET: iced::Color = iced::Color::from_rgb8(0x64, 0xC5, 0xFF);
const BG_GRAY: iced::Color = iced::Color::from_rgb8(0xDC, 0xDC, 0xDC);
const BG_RED: iced::Color = iced::Color::from_rgb8(0xFF, 0x00, 0x00);
const BG_ARCHIVE: iced::Color = iced::Color::from_rgb8(0xCC, 0x00, 0x00);

fn index_of_bg(c: &str) -> usize {
    ID_BGS.iter().position(|b| b.c == c).unwrap_or(0)
}
fn index_of_paper(p: &crate::core::idphoto::IdPaper) -> usize {
    ID_PAPERS.iter().position(|x| x.k == p.k).unwrap_or(0)
}