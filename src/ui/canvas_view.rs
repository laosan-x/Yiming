//! 工作台画布：棋盘格 + 图像 + 对比分割线 + 裁剪框 + 笔刷光标
//!
//! 对应原项目 `#canvasWrap` 里三层 canvas（#view / #viewOriginal / #cropOverlay）
//! 叠加出的效果。
//!
//! # 为什么要拆成两个 `Canvas`（而不是同一个 Frame 画完）
//!
//! iced 的栅格化顺序在 **单个 render layer 内是写死的**：
//!
//! ```text
//! for layer in layers {
//!     quads → triangles(即 canvas 的 fill/stroke) → primitives → images → text
//! }
//! ```
//!
//! （见 `iced_wgpu::Renderer::draw` 的 render 循环、`iced_tiny_skia` 同构）
//!
//! 也就是说：只要分割线/圆钮/角标底/裁剪遮罩和图片处在同一个 layer，
//! **它们一定被图片盖住**——哪怕把它们放进同一个 `Program::draw` 返回的第二个
//! `Geometry` 也没用（`Renderer::draw_geometry` 只是把 mesh 追加进 layer 的
//! triangles 批、把 image 追加进 layer 的 images 批，批次的渲染先后仍是全局固定的）。
//! 只有 text 段在 images 之后，所以旧实现里「角标文字浮在图上有、药丸底却消失」
//! 正是这个原因。
//!
//! 唯一可靠的做法是**换 layer**：`iced::widget::stack![base, overlay]` 会把
//! 索引 >0 的子元素包进 `renderer.with_layer()`，于是覆盖层成为独立 layer，
//! 其 triangles 在上一个 layer 的 images 之后提交 —— 稳定置顶。
//!
//! 因此这里定义两个 Program：`StageView`（衬底 + 图片，负责全部指针事件）与
//! `StageOverlay`（纯覆盖层，不接收事件）。

use std::cell::RefCell;

use iced::advanced::image::{Allocation, Id, Renderer as ImageRenderer};
use iced::mouse;
use iced::widget::canvas::{self, Cache, Canvas, Frame, Geometry, Path, Stroke};
use iced::widget::image::Handle;
use iced::widget::stack;
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, Vector};

use crate::message::{Message, Tool};
use crate::theme::{ACC, CHECKER_A, CHECKER_B, CHECKER_CELL, INK, LINE, TXT2, TXT3};
use crate::core::i18n::tr;

/// 屏幕坐标下的矩形（裁剪框）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn normalize(&self) -> Rect {
        Rect {
            x: self.x.min(self.x + self.w),
            y: self.y.min(self.y + self.h),
            w: self.w.abs(),
            h: self.h.abs(),
        }
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }
    /// 检测点击位置是否在八个控制点之一（返回 0..7，None 表示不在控制点上）
    /// 控制点编号：0=左上, 1=上中, 2=右上, 3=右中, 4=右下, 5=下中, 6=左下, 7=左中
    pub fn hit_handle(&self, p: Point, handle_size: f32) -> Option<usize> {
        let r = self.normalize();
        let half = handle_size / 2.0;
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
        for (i, (hx, hy)) in handles.iter().enumerate() {
            if p.x >= hx - half && p.x <= hx + half && p.y >= hy - half && p.y <= hy + half {
                return Some(i);
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Result,
    Compare,
    Original,
}

/// 图片矩形的衬底（对应参考 `.viewport > canvas.checker` 与底底预览切换）
///
/// 注意：它只作用在**图片矩形**内；画布其余部分始终是 `--bg`
/// （参考 `.canvas-wrap{background:var(--bg)}`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    Checker,
    White,
    Black,
}

/// 画布几何：图片矩形的位置只由「图片尺寸 + zoom + pan + 画布尺寸」决定，
/// 底层与覆盖层两个 canvas 共用同一份，保证覆盖物严格贴着图片（`.viewport`）。
#[derive(Debug, Clone, Copy)]
pub struct StageGeom {
    pub img_size: (f32, f32),
    pub zoom: f32,
    pub pan: (f32, f32),
}

impl StageGeom {
    /// 图片在画布中的屏幕矩形（等价参考项目的 `.viewport`）
    pub fn dest(&self, bounds: Size) -> Rectangle {
        image_rect(self.img_size, self.zoom, self.pan, bounds)
    }

    /// 屏幕坐标 → 图像坐标（画布内部做居中 + 平移 + 缩放）
    fn to_img(self, p: Point, bounds: Rectangle) -> (f32, f32) {
        let dest = self.dest(bounds.size());
        (
            (p.x - dest.x) / self.zoom.max(0.0001),
            (p.y - dest.y) / self.zoom.max(0.0001),
        )
    }

    /// 对比分割线的屏幕 x（相对**图片矩形**，参考 `moveHandle()` 用 viewport rect 换算）
    pub fn split_x(&self, dest: Rectangle, compare_at: f32) -> f32 {
        dest.x + dest.width * compare_at
    }
}

/// 图片在画布中的屏幕矩形（`zoom` / `pan` 之后的落点）
///
/// 等价于参考项目的 `.viewport` —— 棋盘格、对比分割线、对比角标、裁剪遮罩
/// 全都挂在它上面（都是 `.viewport` 的子元素），因此这些覆盖物一律以它为锚，
/// **而不是**以整个画布为锚。
pub fn image_rect(img_size: (f32, f32), zoom: f32, pan: (f32, f32), bounds: Size) -> Rectangle {
    let (iw, ih) = img_size;
    let dw = iw * zoom;
    let dh = ih * zoom;
    Rectangle::new(
        Point::new(
            (bounds.width - dw) / 2.0 + pan.0,
            (bounds.height - dh) / 2.0 + pan.1,
        ),
        Size::new(dw, dh),
    )
}

/// 对图像句柄做同步预上传并持有 `Allocation`，保证其下一帧立即绘制。
///
/// 消除闪跳的机制：iced_wgpu 的 `upload_raster` 对 ≥`MAX_SYNC_SIZE`(2MB) 的图像
/// 首次引用时**跳过当帧绘制**、丢给后台 Worker 异步上传；而 `load_image` 走同步
/// 路径（上传 + `device.poll(Wait)`），完成后句柄在缓存里即为 Device 状态，
/// `upload_raster` 当帧直接命中。持有 `Allocation` 还保证纹理不会被提前回收。
///
/// 仅当句柄 id 与已缓存的不同时才重新上传 —— 平移 / 缩放 / 切视图模式都不改变
/// 句柄 id，因此这些高频操作不会触发重复上传。
fn ensure_image_alloc(
    renderer: &Renderer,
    handle: &Handle,
    slot: &RefCell<Option<(Id, Allocation)>>,
) {
    let id = handle.id();
    let mut cur = slot.borrow_mut();
    if cur.as_ref().map(|(i, _)| *i) != Some(id) {
        // load_image 由 `iced::core::image::Renderer` trait 提供（同步上传 + 返回 Allocation）
        *cur = ImageRenderer::load_image(renderer, handle).ok().map(|a| (id, a));
    }
}

/// 底层：图片矩形衬底 + 图像（不画任何覆盖物）
pub struct StageView<'a> {
    pub geom: StageGeom,
    pub result: Option<&'a Handle>,
    pub original: Option<&'a Handle>,
    pub kind: StageKind,
    /// 对比模式分割线位置（0..1，相对图片宽度）
    pub compare_at: f32,
    /// 图片矩形衬底（透明底 / 白底 / 黑底）
    pub backdrop: Backdrop,
    /// 当前工具：滚轮行为分支用（智能抠图/看图 → 缩放，画笔 → 笔刷大小）
    pub tool: Tool,
}

/// 底层画布的缓存状态：`geometry::Cache` 缓存「棋盘格 + 全幅图」几何，
/// 仅当显示内容变化时才重画；`sig` 记录上次重画时的关键字段，用于判定失效。
///
/// 这是消除「画笔 / 橡皮擦卡顿 + 闪跳」的核心改动：之前 state 是 `()`（零缓存），
/// 每帧都重栅格化整块画布（见 `StageView::draw` 注释）。
pub struct StageViewState {
    cache: Cache<Renderer>,
    /// 上次重画时的签名；用 `RefCell` 是为了在 `draw(&self, state: &State)` 这个
    /// 只给 `&State` 的接口里也能更新（geometry::Cache 自身也是内部可变性）。
    sig: RefCell<Option<StageViewSig>>,
    /// 结果图句柄的显式预上传分配（id, Allocation）。
    ///
    /// ⚠️ 这是消除「轻微闪跳」的核心修复：iced_wgpu 对 ≥2MB 的图像
    /// （真实照片必然超过 `MAX_SYNC_SIZE = 2MB`）首次出现时**不**当帧上传，
    /// 而是交给后台 Worker 异步上传 —— 这一帧该图像被跳过不绘制，结果图
    /// 「空一帧再弹出」，这就是闪跳（见 `iced_core::image::Allocation` 的文档：
    /// "there may be a slight frame delay until it is finally visible ... can cause
    /// undesirable flicker"）。而每次 `refresh_result` 都用 `from_rgba` 生成
    /// **新 id** 的句柄，使每次涂抹提交 / 撤销 / 重置都触发这条异步路径。
    ///
    /// 官方修法：对句柄调用 `renderer.load_image`（同步上传）并**持有**返回的
    /// `Allocation`，保证该句柄下一帧立即绘制。仅当句柄 id 变化时才重新上传一次
    /// —— 平移 / 缩放不改变句柄 id，绝不会重复上传。
    result_alloc: RefCell<Option<(Id, Allocation)>>,
    /// 原图句柄的预上传分配（同上，切换视图模式时原图/结果共用一套机制）
    original_alloc: RefCell<Option<(Id, Allocation)>>,
}

impl Default for StageViewState {
    fn default() -> Self {
        StageViewState {
            cache: Cache::new(),
            sig: RefCell::new(None),
            result_alloc: RefCell::new(None),
            original_alloc: RefCell::new(None),
        }
    }
}

/// 判定底层是否需要重画的签名：仅涵盖「会改变显示结果」的字段。
///
/// 笔刷光标 / 覆盖层相关字段（brush / picker / crop / split 的视觉）刻意不在这里 ——
/// 它们只影响覆盖层（`StageOverlay`），不该触发底层重栅格。
#[derive(Clone, PartialEq)]
struct StageViewSig {
    result: Option<Handle>,
    original: Option<Handle>,
    kind: StageKind,
    backdrop: Backdrop,
    zoom: i32,
    pan: (i32, i32),
    compare: i32,
    iw: i32,
    ih: i32,
    size: (i32, i32),
}

impl StageViewSig {
    fn from_program(p: &StageView<'_>, size: Size) -> Self {
        StageViewSig {
            result: p.result.cloned(),
            original: p.original.cloned(),
            kind: p.kind,
            backdrop: p.backdrop,
            zoom: (p.geom.zoom * 1000.0) as i32,
            pan: (p.geom.pan.0 as i32, p.geom.pan.1 as i32),
            compare: (p.compare_at * 1000.0) as i32,
            iw: p.geom.img_size.0 as i32,
            ih: p.geom.img_size.1 as i32,
            size: (size.width as i32, size.height as i32),
        }
    }
}

impl<'a> canvas::Program<Message> for StageView<'a> {
    /// 底层状态：缓存 + 失效签名。之前是 `()` —— 也就是「每帧都重栅格化」，
    /// 这正是画笔 / 橡皮擦卡顿 + 闪跳的根因之一（详见 `draw` 注释）。
    type State = StageViewState;

    /// 鼠标事件在这里换算成图像坐标再发布，App 不需要知道画布的屏幕尺寸
    fn update(
        &self,
        _state: &mut StageViewState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let pos = cursor.position_in(bounds)?;
        let img = self.geom.to_img(pos, bounds);
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Some(
                canvas::Action::publish(Message::Work(crate::message::WorkMessage::StagePress {
                    screen: pos,
                    img,
                    bounds: (bounds.width, bounds.height),
                })),
            ),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(
                canvas::Action::publish(Message::Work(crate::message::WorkMessage::StageRelease)),
            ),
            iced::Event::Mouse(mouse::Event::CursorLeft) => Some(canvas::Action::publish(
                Message::Work(crate::message::WorkMessage::StageLeave),
            )),
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => Some(
                canvas::Action::publish(Message::Work(crate::message::WorkMessage::StageMove {
                    screen: pos,
                    img,
                    bounds: (bounds.width, bounds.height),
                })),
            ),
            // ── 滚轮（参考项目 bind.js:470 的 wheel 分支）──
            // 智能抠图 / 看图工具（含对比 / 原图模式）→ 缩放，步进 0.9/1.1；
            // 画笔工具 → 笔刷大小 ±2。
            // 参考实现里滚轮处理器**没有**按视图模式特判 —— 对比模式下滚轮照样
            // 缩放（分割线仍可拖，互不干扰）。之前在这里禁掉是自行加戏，已移除。
            // 参考项目里 Ctrl+滚轮任何工具都缩放，但 iced 的 WheelScrolled 事件
            // 不带修饰键状态，画笔工具下以笔刷优先（这正是画笔场景的高频用法）。
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let up = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y > 0.0,
                    mouse::ScrollDelta::Pixels { y, .. } => *y > 0.0,
                };
                let msg = if matches!(self.tool, Tool::Keep | Tool::Erase) {
                    Message::Work(crate::message::WorkMessage::SetBrushSizeStep(if up {
                        2
                    } else {
                        -2
                    }))
                } else if up {
                    Message::Work(crate::message::WorkMessage::ZoomStep(1.1))
                } else {
                    Message::Work(crate::message::WorkMessage::ZoomStep(0.9))
                };
                Some(canvas::Action::publish(msg))
            }
            _ => None,
        }
    }

    /// 对比模式：悬停在分割线热区（±11px，对应参考项目 22px 宽的透明热区）
    /// 时显示 `ew-resize` 光标，提示「这里可以拖」。
    ///
    /// 必须由底层（唯一接收事件的 canvas）提供：覆盖层返回 `None`，
    /// `Stack::mouse_interaction` 会继续向下询问。
    fn mouse_interaction(
        &self,
        _state: &StageViewState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.kind == StageKind::Compare {
            if let Some(pos) = cursor.position_in(bounds) {
                let split = self.geom.split_x(self.geom.dest(bounds.size()), self.compare_at);
                if (pos.x - split).abs() <= HANDLE_HIT {
                    return mouse::Interaction::ResizingHorizontally;
                }
            }
        }
        mouse::Interaction::default()
    }

    fn draw(
        &self,
        state: &StageViewState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        // 诊断开关：COCOLE_DEBUG_CANVAS=1 时，画布状态每次变化打印一行。
        // 注意要把「句柄有无」一起编码进哨兵值 —— 只按 kind 判变化的话，抠图完成前后
        // kind 都是 Result，就只会打印首帧（那时 result 还是 None），反而误导判断。
        if std::env::var_os("COCOLE_DEBUG_CANVAS").is_some() {
            static LAST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0xff);
            let kind_bit = match self.kind {
                StageKind::Result => 0u8,
                StageKind::Compare => 1,
                StageKind::Original => 2,
            };
            let now = kind_bit
                | if self.result.is_some() { 4 } else { 0 }
                | if self.original.is_some() { 8 } else { 0 };
            if LAST.swap(now, std::sync::atomic::Ordering::Relaxed) != now {
                eprintln!(
                    "[canvas] kind={:?} result={} original={} img={:?} zoom={} bounds={:?}",
                    self.kind,
                    self.result.is_some(),
                    self.original.is_some(),
                    self.geom.img_size,
                    self.geom.zoom,
                    bounds
                );
            }
        }

        // ── 底层缓存：棋盘格 + 全幅图只在「显示内容真的变了」时才重栅格化 ──
        //
        // 根因：iced 的 `Canvas::draw` 每帧都会调用 `program.draw`，而 `Canvas`
        // 控件本身不缓存几何（state 是 `()` 时更是零缓存）。因此原本每次鼠标移动
        // —— 哪怕只是把笔刷光标从 A 点挪到 B 点、画笔都没按 —— 都会把棋盘格
        // （逐格 fill_rectangle 的全画布循环）+ 整幅图重新栅格化一次；叠加画笔涂抹时
        // `refresh_result` 又把整图重新合成 + 新建 GPU 纹理（每次 `from_rgba` 都是新 id），
        // 这就是「卡顿 + 闪跳」的直接来源（参考 cocole-v0.3.2 的 `composite.js`
        // 用脏矩形增量合成规避了同样的坑）。
        //
        // 修法：`geometry::Cache` 缓存底层几何，仅当「影响显示内容」的字段
        // （结果 / 原图句柄、视图模式、缩放、平移、对比位置、图片尺寸、画布尺寸）
        // 变化时 `clear()` 重画。笔刷光标 / 覆盖层相关字段刻意不进签名 ——
        // 它们只影响覆盖层，绝不该触发底层重栅格。
        let sig = StageViewSig::from_program(self, bounds.size());
        let mut prev = state.sig.borrow_mut();
        if *prev != Some(sig.clone()) {
            state.cache.clear();
            *prev = Some(sig);
        }

        // ── 显式预上传结果 / 原图句柄，消除 ≥2MB 图像首帧空白的闪跳 ──
        // 必须在返回几何（随后 Canvas 会调 draw_geometry → upload_raster）之前完成：
        // load_image 同步上传后纹理即为 Device 状态，upload_raster 当帧就能取到条目绘制；
        // 否则 >2MB 的图像会被 iced_wgpu 丢给后台 Worker，本帧直接跳过不画 → 闪跳。
        // 持有 Allocation 保证纹理常驻不被回收；仅 id 变化才重新上传（平移/缩放零成本）。
        if let Some(h) = self.result {
            ensure_image_alloc(renderer, h, &state.result_alloc);
        } else {
            *state.result_alloc.borrow_mut() = None;
        }
        if let Some(h) = self.original {
            ensure_image_alloc(renderer, h, &state.original_alloc);
        } else {
            *state.original_alloc.borrow_mut() = None;
        }

        let canvas_rect = Rectangle::new(Point::ORIGIN, bounds.size());
        let geom = state
            .cache
            .draw(renderer, bounds.size(), |frame| self.draw_content(frame, canvas_rect, bounds));
        vec![geom]
    }
}

impl<'a> StageView<'a> {
    /// 底层实际绘制（被 `Cache` 包裹，仅在失效时执行）。
    ///
    /// ⚠️ 调用方已保证 `draw_content` 只在需要时出现；其内部一切绘制都沿用旧实现，
    /// 包括「clip 必须与画布可视区求交」的防溢出处理（放大后图像不得溢出到侧栏 / 检查器）。
    fn draw_content(&self, frame: &mut Frame, canvas_rect: Rectangle, bounds: Rectangle) {
        // 画布底（`--bg`）由外层容器 `stage_bg(BG())` 提供，这里不再铺满整个画布：
        // 参考的棋盘格挂在 `.viewport > canvas.checker` 上，只覆盖**图片矩形**。
        let (iw, ih) = self.geom.img_size;
        if iw <= 0.0 || ih <= 0.0 {
            return;
        }
        let dest = self.geom.dest(bounds.size());

        // ── 图片矩形衬底：透明底=棋盘 / 白底 / 黑底（只铺图片矩形，图片外保持 --bg） ──
        frame.with_clip(canvas_rect, |frame| match self.backdrop {
            Backdrop::Checker => draw_checker_in(frame, dest),
            Backdrop::White => frame.fill_rectangle(dest.position(), dest.size(), Color::WHITE),
            Backdrop::Black => frame.fill_rectangle(dest.position(), dest.size(), Color::BLACK),
        });

        let draw_one = |f: &mut Frame, h: Option<&Handle>, clip: Option<Rectangle>| {
            let Some(h) = h else { return };
            let img = canvas::Image::new(h.clone());
            // ⚠️ iced 的 `with_clip` **不会**自动与控件 bounds 求交：放大后 dest 越出
            // 画布时，裸的 clip 矩形会把图像一直铺到侧栏 / 检查器上（2026-10-14 实测）。
            // 所有 clip 都必须先与画布可视区求交 —— 对应参考 `.canvas-wrap{overflow:hidden}`。
            // 无交集 → 零矩形（完全不绘制），绝不能回退成整块画布。
            let clip = clip.map(|c| {
                c.intersection(&canvas_rect)
                    .unwrap_or(Rectangle::new(Point::ORIGIN, Size::ZERO))
            });
            match clip {
                Some(c) => {
                    f.with_clip(c, |f| f.draw_image(dest, img));
                }
                None => {
                    f.with_clip(canvas_rect, |f| f.draw_image(dest, img));
                }
            }
        };

        match self.kind {
            StageKind::Result => draw_one(&mut *frame, self.result, None),
            StageKind::Original => draw_one(&mut *frame, self.original, None),
            StageKind::Compare => {
                // 分割线两侧：左 = 抠图结果，右 = 原图。
                //
                // ⚠️ 语义以参考项目的**实际行为**为准，不是它 bind.js 的注释：
                // `#viewOriginal` 的 `clip-path:inset(0 0 0 p%)` 让原图只显示在
                // 分割线**右侧**，左半是抠图结果（棋盘格衬底透出）。
                // 2026-10-14 用户拿参考版截图逐像素比对确认：左结果 / 右原图
                // 才是正确语义 —— 之前照注释实现成「左原图 / 右结果」，方向反了。
                let split = self.geom.split_x(dest, self.compare_at);
                draw_one(
                    &mut *frame,
                    self.result,
                    Some(Rectangle::new(
                        dest.position(),
                        Size::new((split - dest.x).clamp(0.0, dest.width), dest.height),
                    )),
                );
                draw_one(
                    &mut *frame,
                    self.original,
                    Some(Rectangle::new(
                        Point::new(split, dest.y),
                        Size::new(
                            (dest.x + dest.width - split).clamp(0.0, dest.width),
                            dest.height,
                        ),
                    )),
                );
            }
        }
    }
}

/// 覆盖层：图片外轮廓 / 对比分割线与角标 / 裁剪遮罩 / 笔刷光标 / 吸管十字
///
/// 单独一个 `Canvas`、由 `stack!` 放进独立 layer，才能绘制在图片之上。
/// 它不做任何事件处理（`update` / `mouse_interaction` 用 trait 默认实现）。
pub struct StageOverlay {
    pub geom: StageGeom,
    pub kind: StageKind,
    pub compare_at: f32,
    pub crop: Option<Rect>,
    /// 笔刷光标（位置 + 半径）
    pub brush: Option<(Point, f32)>,
    /// 吸管十字（位置）
    pub picker: Option<Point>,
    /// 是否正处于「按下涂抹」状态（用于画半透明笔刷足迹预览）
    pub painting: bool,
    /// 当前涂抹模式是否为橡皮擦（决定预览足迹的明暗）
    pub erase: bool,
}

/// 覆盖层缓存状态：与底层同款 `geometry::Cache` + 签名门控。
///
/// 之前 `State = ()` 零缓存 —— 覆盖层每帧都重栅格化（裁剪遮罩四条带、三分线、
/// 分割线 + 圆钮 + 角标、笔刷环……）。光标静止 / 无操作时这是纯浪费的重复渲染。
/// 加缓存后仅当「影响覆盖物显示」的字段变化时才重画（见 `OverlaySig`）。
pub struct OverlayState {
    cache: Cache<Renderer>,
    sig: RefCell<Option<OverlaySig>>,
}

impl Default for OverlayState {
    fn default() -> Self {
        OverlayState {
            cache: Cache::new(),
            sig: RefCell::new(None),
        }
    }
}

/// 覆盖层失效签名：涵盖所有会影响覆盖物绘制的字段。
///
/// 注意必须包含 `brush` 的**半径**（= brush_size × zoom）：用户改笔刷大小时
/// 光标可能不动，若签名不含半径会出现「环大小不变」的陈旧覆盖层。
/// 坐标按 0.25px 量化，避免亚像素抖动造成无意义的重画。
#[derive(Clone, PartialEq)]
struct OverlaySig {
    kind: StageKind,
    compare_at: i32,
    crop: Option<Rect>,
    brush: Option<(i32, i32, i32)>,
    picker: Option<(i32, i32)>,
    painting: bool,
    erase: bool,
    iw: i32,
    ih: i32,
    zoom: i32,
    pan: (i32, i32),
    size: (i32, i32),
}

impl OverlaySig {
    fn from_program(p: &StageOverlay, size: Size) -> Self {
        OverlaySig {
            kind: p.kind,
            compare_at: (p.compare_at * 1000.0) as i32,
            crop: p.crop,
            brush: p.brush.map(|(pt, r)| {
                (
                    (pt.x * 4.0) as i32,
                    (pt.y * 4.0) as i32,
                    (r * 4.0) as i32,
                )
            }),
            picker: p
                .picker
                .map(|pt| ((pt.x * 4.0) as i32, (pt.y * 4.0) as i32)),
            painting: p.painting,
            erase: p.erase,
            iw: p.geom.img_size.0 as i32,
            ih: p.geom.img_size.1 as i32,
            zoom: (p.geom.zoom * 1000.0) as i32,
            pan: (p.geom.pan.0 as i32, p.geom.pan.1 as i32),
            size: (size.width as i32, size.height as i32),
        }
    }
}

impl canvas::Program<Message> for StageOverlay {
    type State = OverlayState;

    fn draw(
        &self,
        state: &OverlayState,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry<Renderer>> {
        // 签名门控：覆盖物字段没变就复用缓存的几何（避免每帧重栅格）；
        // 变了才 clear 重画。geometry::Cache 自身还会在控件尺寸变化时失效。
        let sig = OverlaySig::from_program(self, bounds.size());
        let mut prev = state.sig.borrow_mut();
        if *prev != Some(sig.clone()) {
            state.cache.clear();
            *prev = Some(sig);
        }

        let geom = state.cache.draw(renderer, bounds.size(), |frame| {
            let (iw, ih) = self.geom.img_size;
            if iw <= 0.0 || ih <= 0.0 {
                return;
            }
            let dest = self.geom.dest(bounds.size());
            // 同底层：整体夹进控件矩形（对应 `.canvas-wrap{overflow:hidden}`），
            // 放大后分割线竖线、外轮廓等覆盖物不得溢出画布。
            let canvas_rect = Rectangle::new(Point::ORIGIN, bounds.size());
            frame.with_clip(canvas_rect, |frame| self.draw_overlays(frame, dest, bounds));
        });

        vec![geom]
    }
}

impl StageOverlay {
    /// 覆盖物绘制主体（调用方已套画布裁剪）
    fn draw_overlays(&self, frame: &mut Frame, dest: Rectangle, bounds: Rectangle) {
        // ── 图片矩形外轮廓（参考 `.viewport{box-shadow:var(--shadow-3)}`
        // 的近似`）──
        frame.stroke(
            &Path::rounded_rectangle(dest.position(), dest.size(), 8.0.into()),
            Stroke {
                style: canvas::Style::Solid(Color::from_rgba(0.0, 0.0, 0.0, 0.12)),
                width: 1.0,
                ..Stroke::default()
            },
        );

        // ── 对比分割线 + 圆钮 + 左右角标 ──
        if self.kind == StageKind::Compare {
            let split = self.geom.split_x(dest, self.compare_at);
            draw_compare_handle(frame, split, dest);
            // 角标与分割线两侧的图像语义一致：左 = 抠图结果，右 = 原图。
            //
            // 锚定基准是图片矩形与画布的**可见交集**：放大后图片矩形往往超出
            // 画布（dest.y 为负、右侧越界），若直接锚定图片矩形，角标会跟着
            // 跑出屏幕（放大对比时「两个角标都消失」正是这个原因）。
            const TAG_H: f32 = 22.0;
            let vis_left = dest.x.max(0.0);
            let vis_right = (dest.x + dest.width).min(bounds.width);
            let vis_top = dest.y.max(0.0);
            let vis_bottom = (dest.y + dest.height).min(bounds.height);
            // 先锚定可见区顶部 +12，再夹进可见区底部（可见区过窄时贴底不越界）
            let tag_y = (vis_top + 12.0).min((vis_bottom - TAG_H).max(vis_top));
            let left_w = compare_label_width(tr("抠图结果"));
            let right_w = compare_label_width(tr("原图"));
            let tag_lx = (vis_left + 12.0)
                .min(vis_right - 12.0 - left_w)
                .max(vis_left);
            let tag_rx = (vis_right - 12.0 - right_w).max(vis_left);
            draw_compare_label(frame, tr("抠图结果"), tag_lx, tag_y);
            draw_compare_label(frame, tr("原图"), tag_rx, tag_y);
        }

        // ─ 裁剪框：三分线 + 边框（已移除外部压暗遮罩）──
        if let Some(c) = self.crop {
            let r = c.normalize();
            // 三分构图线
            let thin = Stroke {
                style: canvas::Style::Solid(Color::from_rgba(1.0, 1.0, 1.0, 0.55)),
                width: 1.0,
                ..Stroke::default()
            };
            for i in 1..3 {
                let y = r.y + r.h * i as f32 / 3.0;
                frame.stroke(
                    &Path::line(Point::new(r.x, y), Point::new(r.x + r.w, y)),
                    thin,
                );
                let x = r.x + r.w * i as f32 / 3.0;
                frame.stroke(
                    &Path::line(Point::new(x, r.y), Point::new(x, r.y + r.h)),
                    thin,
                );
            }
            frame.stroke_rectangle(
                Point::new(r.x, r.y),
                Size::new(r.w, r.h),
                Stroke {
                    style: canvas::Style::Solid(ACC()),
                    width: 1.5,
                    ..Stroke::default()
                },
            );
            // 绘制八个控制点（固定屏幕像素大小，不随图片缩放）
            let handle_size = 10.0; // 屏幕像素
            let handle_half = handle_size / 2.0;
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
            for (hx, hy) in handles.iter() {
                // 控制点外框（白色）- 固定屏幕像素大小
                frame.fill_rectangle(
                    Point::new(hx - handle_half, hy - handle_half),
                    Size::new(handle_size, handle_size),
                    Color::WHITE,
                );
                // 控制点内部（强调色）- 固定屏幕像素大小
                frame.fill_rectangle(
                    Point::new(hx - handle_half + 1.5, hy - handle_half + 1.5),
                    Size::new(handle_size - 3.0, handle_size - 3.0),
                    ACC(),
                );
            }
            // 显示裁剪框像素尺寸（在裁剪框下方居中）
            let img_zoom = self.geom.zoom.max(0.0001);
            let crop_w_px = (r.w / img_zoom).round() as u32;
            let crop_h_px = (r.h / img_zoom).round() as u32;
            let size_text = format!("{} × {}", crop_w_px, crop_h_px);
            // 文字背景
            let text_w = size_text.len() as f32 * 7.0 + 10.0;
            let text_h = 20.0;
            let text_x = r.x + (r.w - text_w) / 2.0;
            let text_y = r.y + r.h + 6.0;
            frame.fill_rectangle(
                Point::new(text_x, text_y),
                Size::new(text_w, text_h),
                Color::from_rgba(0.0, 0.0, 0.0, 0.7),
            );
            frame.fill_text(canvas::Text {
                content: size_text,
                position: Point::new(text_x + 5.0, text_y + 4.0),
                size: 12.0.into(),
                color: Color::WHITE,
                ..canvas::Text::default()
            });
        }

        // ── 笔刷光标 ──
        if let Some((p, radius)) = self.brush {
            // 涂抹预览：按下并拖动时，在笔刷足迹内铺一层半透明提示，
            // 告诉用户「这一笔会保留 / 擦除哪片区域」。真正的蒙版像素只在松手时
            // 一次性提交（见 page_work.rs 的 `StageRelease` + `refresh_result_async`），
            // 这样既不每帧重算整图、也不每帧新建 GPU 纹理，画面稳定不闪跳。
            if self.painting {
                let tint = if self.erase {
                    Color::from_rgba(0.04, 0.06, 0.08, 0.30)
                } else {
                    Color::from_rgba(1.0, 1.0, 1.0, 0.32)
                };
                frame.fill(&Path::circle(p, radius.max(1.0)), tint);
            }
            frame.stroke(
                &Path::circle(p, radius.max(1.0)),
                Stroke {
                    style: canvas::Style::Solid(Color::BLACK),
                    width: 3.2,
                    ..Stroke::default()
                },
            );
            frame.stroke(
                &Path::circle(p, radius.max(1.0)),
                Stroke {
                    style: canvas::Style::Solid(Color::WHITE),
                    width: 1.3,
                    ..Stroke::default()
                },
            );
        }

        // ── 吸管十字 ──
        if let Some(p) = self.picker {
            let s = Stroke {
                style: canvas::Style::Solid(TXT3()),
                width: 1.0,
                ..Stroke::default()
            };
            frame.stroke(
                &Path::line(Point::new(p.x - 9.0, p.y), Point::new(p.x + 9.0, p.y)),
                s,
            );
            frame.stroke(
                &Path::line(Point::new(p.x, p.y - 9.0), Point::new(p.x, p.y + 9.0)),
                s,
            );
        }
    }
}

/// 对比分割线热区半宽（参考项目：22px 宽透明热区，视觉线只有 2px —— 否则 2px 的线几乎抓不住）
pub const HANDLE_HIT: f32 = 11.0;

/// 分割线 + 圆钮（对应参考项目 `.compare-handle` 及其 ::before / ::after）。
/// 视觉：2px 近黑竖线带白描边；竖直中点一枚 30px 近黑圆钮，内嵌 12px 白色竖条握把。
fn draw_compare_handle(frame: &mut Frame, split: f32, dest: Rectangle) {
    // 参考 `.compare-handle{top:0;bottom:0}`：竖线铺**图片矩形**的高度，
    // 圆钮落在图片矩形的竖直中点（不是整个画布的）。
    let top = dest.y;
    let bottom = dest.y + dest.height;
    let cy = top + dest.height / 2.0;
    // 圆钮投影（--shadow-2 近似）
    frame.fill(
        &Path::circle(Point::new(split, cy), 17.0),
        Color::from_rgba(0.05, 0.06, 0.08, 0.18),
    );
    // 线：白描边在下，近黑 2px 在上（box-shadow 0 0 0 1px rgba(255,255,255,.7)）
    let vline = Path::line(Point::new(split, top), Point::new(split, bottom));
    frame.stroke(
        &vline,
        Stroke {
            style: canvas::Style::Solid(Color::from_rgba(1.0, 1.0, 1.0, 0.7)),
            width: 4.0,
            ..Stroke::default()
        },
    );
    frame.stroke(
        &vline,
        Stroke {
            style: canvas::Style::Solid(INK()),
            width: 2.0,
            ..Stroke::default()
        },
    );
    // 圆钮（30px）
    frame.fill(&Path::circle(Point::new(split, cy), 15.0), INK());
    // 白色竖条握把（对应 ::after 里 45%~55% 的 linear-gradient 白条）
    frame.fill_rectangle(
        Point::new(split - 1.0, cy - 6.0),
        Size::new(2.0, 12.0),
        Color::WHITE,
    );
}

/// 标签药丸宽度估算：11px 字号的 CJK 字宽 ≈ 11px + 左右各 10px padding
fn compare_label_width(text: &str) -> f32 {
    text.chars().count() as f32 * 11.0 + 20.0
}

/// 对比角标（对应参考项目 `.compare-label`）：白底 92% + hairline + 8px 圆角，
/// 11px `--txt2` 文字。canvas 里没有 DOM，药丸底自绘。
fn draw_compare_label(frame: &mut Frame, text: &str, x: f32, y: f32) {
    let w = compare_label_width(text).max(30.0);
    let h = 22.0;
    let rect = Rectangle::new(Point::new(x, y), Size::new(w, h));
    let pill = Path::rounded_rectangle(rect.position(), rect.size(), 8.0.into());
    frame.fill(&pill, Color::from_rgba(1.0, 1.0, 1.0, 0.92));
    frame.stroke(
        &pill,
        Stroke {
            style: canvas::Style::Solid(LINE()),
            width: 1.0,
            ..Stroke::default()
        },
    );
    frame.fill_text(canvas::Text {
        content: text.to_string(),
        position: Point::new(x + 10.0, y + 5.0),
        size: 11.0.into(),
        color: TXT2(),
        ..canvas::Text::default()
    });
}

/// 在指定矩形内铺棋盘格（透明底指示），图案锚点取矩形左上角
/// —— 等价于参考里挂在 `<canvas class="checker">` 上的背景。
fn draw_checker_in(frame: &mut Frame, rect: Rectangle) {
    let cell = CHECKER_CELL;
    let cols = (rect.width / cell).ceil() as i32 + 1;
    let rows = (rect.height / cell).ceil() as i32 + 1;
    for r in 0..rows {
        for c in 0..cols {
            let x = rect.x + c as f32 * cell;
            let y = rect.y + r as f32 * cell;
            if x >= rect.x + rect.width || y >= rect.y + rect.height {
                continue;
            }
            // 末列 / 末行裁到矩形边界内，避免溢出图片矩形
            let w = (rect.x + rect.width - x).min(cell);
            let h = (rect.y + rect.height - y).min(cell);
            let color = if (r + c) % 2 == 0 { CHECKER_A() } else { CHECKER_B() };
            frame.fill_rectangle(Point::new(x, y), Size::new(w, h), color);
        }
    }
    let _ = Vector::ZERO;
}

/// 便捷构造：底层 + 覆盖层叠成一个元素
///
/// `stack!` 会把第二个子元素包进 `renderer.with_layer()`（独立 render layer），
/// 这是覆盖层能绘制在图片之上的**唯一**手段 —— 详见文件头注释。
pub fn stage_canvas<'a>(base: StageView<'a>, overlay: StageOverlay) -> Element<'a, Message> {
    stack![
        Canvas::new(base).width(Length::Fill).height(Length::Fill),
        Canvas::new(overlay).width(Length::Fill).height(Length::Fill),
    ]
    .into()
}