//! 聚焦式新手引导：目标自身高亮描边 + 说明气泡锚定目标旁。
//!
//! **默认完全不展示**：启动、刷新、首次进入首页、切页往返都不会自动弹出；
//! 整个引导层（描边 + 气泡 + 事件层）只有 `guide_step == Some` 时才进入
//! 视图树，而把它置为 Some 的唯一入口是首页「新手引导」按钮发出的
//! `HomeMessage::StartGuide`（→ [`transition`] 的 [`Action::Start`]）。
//!
//! 三件套：
//! 1. [`Target`] + [`Tracked`]：给目标控件包一层探针 widget，draw 时把自己的
//!    窗口绝对坐标写进注册表——每帧更新，天然跟随侧栏折叠 / 窗口缩放 / DPI；
//! 2. [`GuideLayer`]：全屏透明 canvas，**不绘制任何内容**——唯一的一圈描边
//!    由目标控件自身的高亮样式（`page_home::card_hi` / CTA 边框）承担，避免
//!    「控件边框 + 外扩聚焦框」叠成双层描边。它只负责捕获点击推进引导，
//!    事件保持 Ignored 继续传播，点目标本身会真实触发下层控件；页面完全
//!    不变暗、不被覆盖；
//! 3. [`spotlight`]：把说明气泡锚定到目标旁——优先放下方，放不下放上方，
//!    两者都放不下时兜底顶部居中（目标坐标未注册时同样走兜底，不崩溃）。
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
use iced::widget::canvas::{self, Canvas, Geometry};
use iced::widget::{container, stack};
use iced::{Alignment, Element, Event, Length, Padding, Rectangle, Size, Theme, Vector};

use crate::message::{HomeMessage, Message};

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
/// 此时若沿用上一帧的绝对坐标开引导，气泡锚点会落到错位甚至屏幕外。
/// 清空后由首页下一帧 draw 重新注册；引导只能由按钮手动开启，而点击时
/// 首页早已渲染完成，坐标是新的，无需额外的就绪轮询。
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

/* ═════════════════ 引导事件层 ═════════════════ */

/// 全屏透明事件层：**不绘制任何内容**，页面完全不变暗、不被覆盖。
///
/// 唯一的描边由目标控件自身承担（卡片 `card_hi` / CTA 边框，均为主题
/// `ACC()` 2px + `R_CARD` 圆角）。此前这里还会在目标外扩 6px 再画一圈
/// 聚焦框，与控件自身边框叠成双线，故移除画布描边、只保留事件职责。
///
/// 事件语义保持不变：任意位置点击都推进引导，且 `Action` 保持
/// Ignored 让事件继续传播——点目标本身会真实触发下层控件（如上传卡弹文件
/// 选择器），「操作即学会」；气泡在更上层，其按钮不会走到这里。
/// 注意本层只在用户点按钮后（`guide_step == Some`）才被放入视图树，
/// 引导关闭时它根本不存在，不会有「点任意位置意外弹出引导」的可能。
pub struct GuideLayer;

impl canvas::Program<Message> for GuideLayer {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        _renderer: &iced::Renderer,
        _theme: &Theme,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // 无描边、无遮罩：draw 只返回空 geometry，视觉完全交给下层控件
        Vec::new()
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
                // 点击推进引导；Action 为 Ignored 状态，事件继续传播——目标区
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
/// 气泡估算高度（仅用于判断锚定框下方是否放得下；实际高度由内容决定）
const BUBBLE_H_EST: f32 = 196.0;

/// 组装引导层：透明事件层 + 锚定在目标旁的说明气泡。
/// `anchor_abs` 为目标锚定框的窗口绝对坐标（外扩 6px 呼吸间距），**只用于
/// 气泡定位**，不再绘制外圈描边；坐标未注册（None）时气泡顶部居中兜底，
/// 页面不发生任何变暗或覆盖。
pub fn spotlight<'a>(
    anchor_abs: Option<Rectangle>,
    bubble: Element<'a, Message>,
) -> Element<'a, Message> {
    let root = bounds(Target::Root);
    // 绝对坐标 → 相对画布坐标（画布与 Root 同区域、同坐标系）
    let ring_rel = anchor_abs.zip(root).map(|(h, r)| Rectangle {
        x: h.x - r.x,
        y: h.y - r.y,
        width: h.width,
        height: h.height,
    });

    // 事件层：只接管点击推进，不参与绘制
    let focus = Canvas::new(GuideLayer)
        .width(Length::Fill)
        .height(Length::Fill);

    // 气泡定位：锚定框下方优先（padding-top 顶推），放不下改上方（底对齐 + padding-bottom），
    // 水平方向跟随锚定框左缘并钳制在容器内
    let anchored = match (ring_rel, root) {
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

    stack![focus, anchored]
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
///
/// **开启引导的唯一动作是 [`Action::Start`]**（首页「新手引导」按钮）。
/// 引导未开启时其它任何动作都保持 None，避免误发的消息把引导层带出来。
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
        // 跳步只在引导进行中有效；旧实现从 None 也会返回 Some(i)，
        // 会让一次误发的 GuideGoto 在用户没点按钮时弹出引导。
        Action::Goto(i) => match step {
            Some(_) => (Some(i.min(last)), false),
            None => (None, false),
        },
        Action::Skip => (None, false), // 仅关闭；再开启只能靠「新手引导」按钮
        Action::Never => (None, true), // 关闭并持久化「不再显示」历史标记
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
    fn goto_clamps_out_of_range_only_while_open() {
        assert_eq!(
            transition(Some(0), Action::Goto(2), TOTAL),
            (Some(2), false)
        );
        assert_eq!(
            transition(Some(0), Action::Goto(99), TOTAL),
            (Some(3), false)
        );
        assert_eq!(transition(Some(0), Action::Goto(5), 0), (Some(0), false));
        // 引导未开启时，跳步消息不得把引导带出来
        assert_eq!(transition(None, Action::Goto(2), TOTAL), (None, false));
    }

    #[test]
    fn only_start_can_open_the_guide() {
        // 回归需求：默认不自动展示，且其它任何操作都不得意外触发
        for action in [
            Action::Prev,
            Action::Next,
            Action::Goto(0),
            Action::Goto(3),
            Action::Skip,
            Action::Never,
        ] {
            let (step, _) = transition(None, action, TOTAL);
            assert_eq!(step, None, "{action:?} 不应开启引导");
        }
        assert_eq!(transition(None, Action::Start, TOTAL).0, Some(0));
    }

    #[test]
    fn skip_only_closes_but_never_closes_and_persists() {
        assert_eq!(transition(Some(2), Action::Skip, TOTAL), (None, false));
        assert_eq!(transition(Some(2), Action::Never, TOTAL), (None, true));
    }

    #[test]
    fn zero_total_does_not_panic() {
        assert_eq!(transition(None, Action::Goto(5), 0), (None, false));
        assert_eq!(transition(Some(0), Action::Next, 0), (None, true));
    }
}
