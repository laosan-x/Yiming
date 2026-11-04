//! 聚焦式新手引导（Spotlight）：遮罩挖洞高亮目标区 + 说明气泡锚定目标旁。
//!
//! 三件套：
//! 1. [`Target`] + [`Tracked`]：给目标控件包一层探针 widget，draw 时把自己的
//!    窗口绝对坐标写进注册表——每帧更新，天然跟随侧栏折叠 / 窗口缩放 / DPI；
//! 2. [`HoleMask`]：全屏 canvas 遮罩，在目标区域「挖洞」——四块半透明矩形
//!    围合 + 圆角强调色描边。洞内点击**穿透**到下层真实控件（与 `dashed_box`
//!    的 canvas 事件穿透同机制），洞外点击推进引导；
//! 3. [`spotlight`]：把说明气泡锚定到洞旁——优先放洞下方，放不下放上方，
//!    两者都放不下时兜底顶部居中（洞缺失 / 未注册时同样走兜底，不崩溃）。
//!
//! 坐标系说明：`Tracked` 记录的是窗口绝对坐标；view 构建发生在 draw 之前，
//! 读到的是**上一帧**的 bounds——布局未变时即为正确值，步骤切换 / 窗口
//! resize 会有最多一帧的锚点滞后，肉眼不可感。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{overlay, renderer, Clipboard, Shell, Widget};
use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::widget::{container, stack};
use iced::{
    Alignment, Color, Element, Event, Length, Padding, Point, Rectangle, Size, Theme, Vector,
};

use crate::message::{HomeMessage, Message};
use crate::theme::{ACC, R_CARD};

/* ═════════════════ 目标注册表 ═════════════════ */

/// 可聚焦的引导目标（`Root` 是坐标换算基准，不作为步骤目标）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// 首页内容区根容器（气泡定位的参考坐标系）
    Root,
    /// 「开始」卡（上传照片）
    StartCard,
    /// 「快速预设」卡
    Presets,
    /// 「当前状态」卡
    Status,
    /// 「去制作证件照」按钮
    MakeBtn,
}

fn tracks() -> &'static Mutex<HashMap<Target, Rectangle>> {
    static TRACKS: OnceLock<Mutex<HashMap<Target, Rectangle>>> = OnceLock::new();
    TRACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 读取上一帧记录的目标 bounds（窗口绝对坐标）
pub fn bounds(id: Target) -> Option<Rectangle> {
    let m = tracks().lock().ok()?;
    m.get(&id).copied()
}

/// 清空目标坐标注册表（切离首页 / 关闭引导时调用）。
///
/// 窗口 resize、DPI 变化或侧栏折叠后，首页重新 draw 之前存在一段空窗；
/// 此时若沿用上一帧的绝对坐标开引导，洞会画到错位甚至屏幕外。清空后由
/// `app.rs` 的就绪轮询等待首帧重新注册，避免读到过期坐标。
pub fn clear() {
    if let Ok(mut m) = tracks().lock() {
        m.clear();
    }
}

fn track(id: Target, rect: Rectangle) {
    if let Ok(mut m) = tracks().lock() {
        m.insert(id, rect);
    }
}

/* ═════════════════ 探针 widget ═════════════════ */

/// 包裹目标控件：draw 时把自身 bounds 写进注册表，其余全部委托给内部元素。
/// 布局零副作用（尺寸 / 间距完全由内部元素决定）。
pub struct Tracked<'a> {
    id: Target,
    inner: Element<'a, Message>,
}

impl<'a> Tracked<'a> {
    pub fn new(id: Target, inner: impl Into<Element<'a, Message>>) -> Self {
        Self {
            id,
            inner: inner.into(),
        }
    }

    fn w(&self) -> &dyn Widget<Message, Theme, iced::Renderer> {
        self.inner.as_widget()
    }

    fn w_mut(&mut self) -> &mut dyn Widget<Message, Theme, iced::Renderer> {
        self.inner.as_widget_mut()
    }
}

impl Widget<Message, Theme, iced::Renderer> for Tracked<'_> {
    fn size(&self) -> Size<Length> {
        self.w().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.w().size_hint()
    }

    fn tag(&self) -> tree::Tag {
        self.w().tag()
    }

    fn state(&self) -> tree::State {
        self.w().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.w().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.w().diff(tree);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.w_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.w_mut().operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.w_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        track(self.id, layout.bounds());
        self.w()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.w()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.w_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a> From<Tracked<'a>> for Element<'a, Message> {
    fn from(t: Tracked<'a>) -> Self {
        Element::new(t)
    }
}

/* ═════════════════ 挖洞遮罩 ═════════════════ */

/// 压暗色（比旧版 0.22 深，让「洞」的对比更强）
const SCRIM: Color = Color {
    r: 0.055,
    g: 0.065,
    b: 0.08,
    a: 0.55,
};

/// 全屏挖洞遮罩：`hole` 为相对画布的洞区域（None = 整面压暗兜底）。
/// 洞内点击穿透（返回 None，事件继续传播到下层真实控件），
/// 洞外点击推进引导（capture 防止误触下层控件）。
pub struct HoleMask {
    pub hole: Option<Rectangle>,
    pub radius: f32,
}

impl canvas::Program<Message> for HoleMask {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        match self.hole {
            Some(h) => {
                let (cw, ch) = (bounds.width, bounds.height);
                // 四块半透明矩形围合出洞（左右块 y 范围与洞重叠，接缝处无缝）。
                // 注意 fill 默认黑色，压暗色必须显式传 SCRIM。
                let mut quad = |p: Point, s: Size| frame.fill(&Path::rectangle(p, s), SCRIM);
                quad(Point::new(0.0, 0.0), Size::new(cw, h.y.max(0.0)));
                quad(
                    Point::new(0.0, h.y + h.height),
                    Size::new(cw, (ch - h.y - h.height).max(0.0)),
                );
                quad(
                    Point::new(0.0, h.y.max(0.0)),
                    Size::new(h.x.max(0.0), h.height),
                );
                quad(
                    Point::new(h.x + h.width, h.y.max(0.0)),
                    Size::new((cw - h.x - h.width).max(0.0), h.height),
                );
                // 洞边框：强调色圆角描边，外扩 1.5px 让线宽完整可见
                let path = Path::rounded_rectangle(
                    Point::new(h.x - 1.5, h.y - 1.5),
                    Size::new(h.width + 3.0, h.height + 3.0),
                    (self.radius + 1.5).into(),
                );
                frame.stroke(&path, Stroke::default().with_color(ACC()).with_width(2.5));
            }
            None => {
                frame.fill(&Path::rectangle(Point::new(0.0, 0.0), bounds.size()), SCRIM);
            }
        }
        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if cursor.position_in(bounds).is_some() {
                // 点击推进引导；Action 为 Ignored 状态，事件继续传播——洞内
                // 点击会同时触发下层真实控件（如上传卡弹文件选择器），
                // 「操作即学会」；气泡区在更上层，按钮点击不会到这里。
                return Some(canvas::Action::publish(Message::Home(
                    HomeMessage::GuideNext,
                )));
            }
        }
        None
    }
}

/* ═════════════════ 组装 ═════════════════ */

const GAP: f32 = 14.0;
const BUBBLE_W: f32 = 328.0;
/// 气泡估算高度（仅用于判断洞下方是否放得下；实际高度由内容决定）
const BUBBLE_H_EST: f32 = 196.0;

/// 组装聚焦层：挖洞遮罩 + 锚定在洞旁的说明气泡。
/// `hole_abs` 为洞的窗口绝对坐标（外扩过呼吸空间）；未注册（None）时
/// 整面压暗、气泡顶部居中兜底。
pub fn spotlight<'a>(
    hole_abs: Option<Rectangle>,
    bubble: Element<'a, Message>,
) -> Element<'a, Message> {
    let root = bounds(Target::Root);
    // 绝对坐标 → 相对遮罩画布（画布与 Root 同区域、同坐标系）
    let hole_rel = hole_abs.zip(root).map(|(h, r)| Rectangle {
        x: h.x - r.x,
        y: h.y - r.y,
        width: h.width,
        height: h.height,
    });

    let mask = Canvas::new(HoleMask {
        hole: hole_rel,
        radius: R_CARD,
    })
    .width(Length::Fill)
    .height(Length::Fill);

    // 气泡定位：洞下方优先（padding-top 顶推），放不下改上方（底对齐 + padding-bottom），
    // 水平方向跟随洞左缘并钳制在容器内
    let anchored = match (hole_rel, root) {
        (Some(h), Some(r)) => {
            let (aw, ah) = (r.width, r.height);
            let left = (h.x - 4.0).clamp(10.0, (aw - BUBBLE_W - 10.0).max(10.0));
            let fits_below = (ah - (h.y + h.height)) >= GAP + BUBBLE_H_EST;
            if fits_below {
                container(bubble)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Start)
                    .padding(Padding {
                        top: h.y + h.height + GAP,
                        left,
                        right: 0.0,
                        bottom: 0.0,
                    })
            } else {
                container(bubble)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Alignment::Start)
                    .align_y(Alignment::End)
                    .padding(Padding {
                        top: 0.0,
                        left,
                        right: 0.0,
                        bottom: (ah - h.y) + GAP,
                    })
            }
        }
        _ => container(bubble)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .padding(Padding {
                top: 96.0,
                ..Padding::ZERO
            }),
    };

    stack![mask, anchored]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/* ═════════════════ 纯状态机（headless 单测） ═════════════════ */

/// 引导动作，与 `HomeMessage` 一一对应但不持有任何 UI / App 状态。
///
/// iced 0.14 没有官方 kittest / headless GUI 测试框架，把状态转移抽成纯函数
/// 是唯一能自动化回归引导流程的方式；`page_home::update` 只负责写回 App
/// 与持久化副作用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Prev,
    Next,
    Goto(usize),
    Skip,
    Never,
}

/// 纯状态转移。
///
/// 返回 `(新步, 是否应写入 guide_done)`；`step == None` 表示引导未开启。
/// `total` 为步骤总数（由调用方传 `GUIDE_STEPS.len()`）。
pub fn transition(step: Option<usize>, action: Action, total: usize) -> (Option<usize>, bool) {
    let last = total.saturating_sub(1);
    match action {
        Action::Start => (Some(0), false),
        Action::Prev => (step.map(|s| s.saturating_sub(1)), false),
        Action::Next => match step {
            Some(s) if s + 1 < total => (Some(s + 1), false),
            Some(_) => (None, true), // 末步「完成」：关闭并持久化
            None => (None, false),   // 未开启时忽略，绝不误写 guide_done
        },
        Action::Goto(i) => (Some(i.min(last)), false),
        Action::Skip => (None, false), // 仅关闭，下次进首页仍会再弹
        Action::Never => (None, true), // 关闭并持久化「不再显示」
    }
}

#[cfg(test)]
mod tests {
    use super::{transition, Action};

    const TOTAL: usize = 4;

    #[test]
    fn start_opens_first_step() {
        assert_eq!(transition(None, Action::Start, TOTAL), (Some(0), false));
        // 手动重开：即使在最后一步也回到第 0 步
        assert_eq!(transition(Some(3), Action::Start, TOTAL), (Some(0), false));
    }

    #[test]
    fn prev_clamps_at_zero() {
        assert_eq!(transition(Some(0), Action::Prev, TOTAL), (Some(0), false));
        assert_eq!(transition(Some(3), Action::Prev, TOTAL), (Some(2), false));
        assert_eq!(transition(None, Action::Prev, TOTAL), (None, false));
    }

    #[test]
    fn next_walks_every_step_then_finishes() {
        let mut step = None;
        step = transition(step, Action::Start, TOTAL).0;
        assert_eq!(step, Some(0));
        for expected in 1..TOTAL {
            let (next, done) = transition(step, Action::Next, TOTAL);
            assert_eq!(next, Some(expected));
            assert!(!done, "非末步不应写 guide_done");
            step = next;
        }
        assert_eq!(transition(step, Action::Next, TOTAL), (None, true));
    }

    #[test]
    fn next_when_closed_is_ignored_without_persisting() {
        // 回归：旧实现在 step=None 时收到 GuideNext 也会把 guide_done 置 true，
        // 会让用户以后再也看不到引导。
        assert_eq!(transition(None, Action::Next, TOTAL), (None, false));
    }

    #[test]
    fn goto_clamps_out_of_range() {
        assert_eq!(transition(None, Action::Goto(2), TOTAL), (Some(2), false));
        assert_eq!(transition(None, Action::Goto(99), TOTAL), (Some(3), false));
        assert_eq!(transition(None, Action::Goto(0), 0), (Some(0), false));
    }

    #[test]
    fn skip_only_closes_but_never_closes_and_persists() {
        assert_eq!(transition(Some(2), Action::Skip, TOTAL), (None, false));
        assert_eq!(transition(Some(2), Action::Never, TOTAL), (None, true));
    }

    #[test]
    fn zero_total_does_not_panic() {
        assert_eq!(transition(None, Action::Goto(5), 0), (Some(0), false));
        assert_eq!(transition(Some(0), Action::Next, 0), (None, true));
    }
}
