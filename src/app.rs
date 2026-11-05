//! 顶层 State / update / view —— Elm 架构的中枢
//!
//! 所有状态集中在这里，所有变更都必须经过 `Message`。
//! 每个主要界面各有一个模块（ui/page_*.rs），负责自己的 update 分支与 view，
//! 这里只做「外壳 + 分发」。

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::image::Handle;
use iced::{window, Task};
use image::RgbaImage;

use crate::core::config::Config;
use crate::core::history::History;
use crate::core::i18n::{tr, trf};
use crate::core::idphoto::{RenderMode, ID_DPI};
use crate::core::matting::MatteParams;
use crate::core::updater::{self, ReleaseInfo};
use crate::message::*;
use crate::theme;
use crate::ui;

/// 轻提示存活时长（对应原项目 `ui/toast.js` 里的 `setTimeout(..., 2400)`）
const TOAST_MS: u64 = 2400;

/// 获取系统 DPI（Windows）。
/// 使用 `GetDpiForSystem` API（Windows 10 1607+），失败时回退到 96。
#[cfg(windows)]
fn get_system_dpi() -> f32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CACHED_DPI: AtomicU32 = AtomicU32::new(0);

    let cached = CACHED_DPI.load(Ordering::Relaxed);
    if cached != 0 {
        return cached as f32;
    }

    // 使用 windows crate 的安全包装调用 GetDpiForSystem
    let dpi = unsafe {
        use windows::core::{PCSTR, PCWSTR};
        use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

        // user32.dll 在进程启动时已加载
        let user32_w: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        let hmodule = GetModuleHandleW(PCWSTR::from_raw(user32_w.as_ptr()));

        if let Ok(h) = hmodule {
            let proc = GetProcAddress(h, PCSTR::from_raw(c"GetDpiForSystem".as_ptr().cast()));
            if let Some(func) = proc {
                let func: unsafe extern "system" fn() -> u32 = std::mem::transmute(func);
                func()
            } else {
                96
            }
        } else {
            96
        }
    };

    CACHED_DPI.store(dpi, Ordering::Relaxed);
    dpi as f32
}

#[cfg(not(windows))]
fn get_system_dpi() -> f32 {
    96.0
}

/* ═════════════════ 顶层状态 ═════════════════ */

/// 一条轻提示（对应原项目的 `.toast` / `.toast.err`）
#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub text: String,
    /// 失败类提示用红底（原项目 `toast(msg, true)`）
    pub err: bool,
}

/// 消息中心条目（toast 的持久化镜像）
#[derive(Debug, Clone)]
pub struct MsgItem {
    /// 保留供「单条删除/定位」扩展
    #[allow(dead_code)]
    pub id: u64,
    pub title: String,
    pub err: bool,
    pub ts_ms: u64,
    pub read: bool,
}

pub struct App {
    pub page: Page,
    pub nav_collapsed: bool,
    /// 当前窗口宽度（逻辑像素；由 resize 订阅维护，首页响应式断点用）
    pub win_w: f32,
    /// 当前窗口高度（逻辑像素；消息中心等模态的比例尺寸用）
    pub win_h: f32,
    /// 持久化配置（写系统配置目录的 JSON）
    pub config: Config,
    /// 提示条队列（旧的在最上，最多 3 条）
    pub toasts: Vec<Toast>,
    toast_seq: u64,

    /* ── 抠图工作台 ── */
    pub work: WorkState,

    /* ── 证件照 ── */
    pub id: IdState,

    /* ─ 设置页 ── */
    pub settings_panel: usize,
    /// 系统 DPI（用于设置页显示）
    pub system_dpi: f32,
    /// DPI 检测状态：None=未检测, Some(true)=成功, Some(false)=失败
    pub dpi_detected: Option<bool>,

    /// 状态栏右下角的机器信息，启动时探测一次即可，避免每帧重复计算
    pub cpu_text: String,
    pub mem_text: String,

    /* ─ 自更新 ── */
    pub updater: UpdaterState,

    /* ─ 文件拖拽 ── */
    /// 有文件拖拽悬停在窗口上（首页拖放区高亮用；落下/离开后复位）
    pub drop_hover: bool,

    /* ─ 外观 / 语言 / 通知 ── */
    /// 主题模式（浅色 / 深色 / 跟随系统），config 持久化
    pub theme_mode: crate::theme::ThemeMode,
    /// 系统是否偏好深色（boot 时读注册表缓存一次；「跟随系统」模式用）
    pub system_dark: bool,
    /// 消息中心（toast 同源记录；最多保留 100 条）
    pub msgs: Vec<MsgItem>,
    pub unread: usize,
    /// 消息中心面板展开中
    pub msg_center_open: bool,
    /// 首页分步新手引导：Some(步骤序号) = 引导进行中；None = 未显示（默认）。
    /// 只能由首页「新手引导」按钮（StartGuide）置为 Some，不自动弹出。
    pub guide_step: Option<usize>,

    /* ─ 自绘标题栏 / 打赏 ── */
    /// 主窗口 Id（boot 时经 window::oldest 解析；窗口控制任务需要）
    pub win_id: Option<iced::window::Id>,
    /// 窗口最大化中（标题栏最大/还原图标切换）
    pub win_maximized: bool,
    /// Win11 圆角 / 阴影已恢复（decorations: false 后经 DWM 补回；失败随 resize 重试）
    pub win_frame_fixed: bool,
    /// 打赏弹窗展开中
    pub sponsor_open: bool,
    /// 选中的建议金额（元；打赏弹窗提示文案用）
    pub sponsor_amount: f32,
    /// 自定义金额输入（字符串，解析失败按 0 处理）
    pub sponsor_custom: String,

    /* ─ 关于页 Markdown 内容 ── */
    /// 解析后的关于页文档（content/about/*.md，进入页面时重新加载）
    pub about_md: Vec<iced::widget::markdown::Item>,
    /// 大字标语（slogan.md 非「作者：」行）
    pub about_slogan: String,
    /// 署名（slogan.md「作者：xxx」行；None = 不显示）
    pub about_author: Option<String>,
}

#[derive(Default)]
pub struct Metrics {
    pub elapsed_ms: f32,
    pub fg_ratio: f32,
    pub w: u32,
    pub h: u32,
    pub holes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Pan,
    Paint,
    /// 对比模式：拖动分割线（0..1，屏幕坐标 / 画布宽度）
    Compare,
    CropNew,
    CropMove,
    CropResize(usize), // 0..7 八个把手，简化为右下角 0
}

pub struct WorkState {
    /// 原图（Arc 共享：撤销快照零拷贝引用）
    pub src: Option<std::sync::Arc<RgbaImage>>,
    /// 色键场缓存（grad/dist 只依赖图像，跨参数调用复用；换图/旋转/裁剪时作废）
    pub fields: std::sync::Arc<std::sync::Mutex<crate::core::matting::FieldCache>>,
    /// 蒙版重算世代：回传时与发起瞬间的 seq 比对，丢弃过期帧；
    /// 换图 / 新一次重算都会推进（见 [`WorkState::invalidate_inflight_matte`]）
    pub recompute_seq: u64,
    /// 防抖序列号：滑块每次变化自增，只有最后一次的定时器允许触发重算
    pub debounce_seq: u64,
    /// 导出进行中（fit_export + 编码在后台线程，期间禁用导出按钮防重入）
    pub exporting: bool,
    /// 导出序列号：换图 / 重复导出后丢弃过期的后台编码结果
    pub export_seq: u64,
    /// 图片载入中（解码阶段；busy = 抠图阶段）——驱动载入遮罩
    pub loading: bool,
    /// 旋转 spinner 帧计数（时间订阅驱动，仅载入/处理期间递增）
    pub frame: f32,
    /// 重算取消令牌：高频拖动滑块时新请求使旧后台线程提前退出（防线程堆积）
    pub matte_cancel_tx: tokio::sync::watch::Sender<u64>,
    pub matte_cancel_rx: tokio::sync::watch::Receiver<u64>,
    /// 抠图结果（Arc：导出把像素交给后台线程时零拷贝共享，UI 线程不再深拷贝整图）
    pub result: Option<std::sync::Arc<RgbaImage>>,
    pub mask: Option<Vec<u8>>,
    pub base: Option<Vec<u8>>,
    /// iced 图像句柄（供 canvas 绘制）：full 层用于 zoom>=1，preview 层用于缩小视图
    pub result_handle: Option<Handle>,
    pub orig_handle: Option<Handle>,
    /// 视口降采样预览层（G2）：zoom<1 时改传它，GPU 每帧上传量按面积锐减
    pub result_preview: Option<Handle>,
    pub orig_preview: Option<Handle>,
    pub stem: String,
    pub src_path: Option<PathBuf>,

    /// 画布可视区尺寸（由 canvas 事件带出，用于「适应窗口」）
    pub stage: (f32, f32),
    pub params: MatteParams,
    pub brush_size: i32,
    pub brush_opacity: i32,
    pub brush_flow: i32,
    pub hardness_keep: i32,
    pub hardness_erase: i32,
    /// 「笔刷属性」下拉面板是否展开（保留 / 擦除模式专用，纯 UI 状态不持久化）
    pub brush_props_open: bool,

    pub tool: Tool,
    pub view_mode: ViewMode,
    pub bg_preview: BgPreview,
    pub out_color: String,
    pub zoom: f32,
    pub pan: (f32, f32),
    pub fit_zoom: f32,
    pub compare_at: f32,

    pub busy: bool,
    pub metrics: Metrics,
    pub history: History,
    pub rot_total: f32,
    pub rot_deg: f32,
    pub crop: Option<ui::canvas_view::Rect>,
    pub crop_ratio: CropRatio,
    /// 裁剪框初始状态（按下控制点时记录，用于拖动时基于初始状态计算）
    pub crop_initial: Option<ui::canvas_view::Rect>,
    /// 裁剪双击检测：记录上次点击时间和位置
    pub last_crop_click: Option<(std::time::Instant, iced::Point)>,
    pub drag: Option<Drag>,
    pub drag_from: Option<iced::Point>,
    pub brush_cursor: Option<iced::Point>,
    /// 上一次落笔的图像坐标（用于拖动时插值补点）
    pub last_paint: Option<(f32, f32)>,
    pub picker_at: Option<iced::Point>,
    /// 吸管悬停处取样的颜色（原图坐标越界 / 非吸管工具时为 None）
    pub picker_hover: Option<[u8; 3]>,
    /// 蒙版合成提交的序列号：每次异步 `refresh_result` 自增，回传 `ResultCommitted`
    /// 时比对，丢弃因快速重复提交而落后的过期帧（参考 IdMessage::RenderDone）。
    pub refresh_seq: u64,
    pub custom_hex: String,
    pub batch: Vec<WorkBatchItem>,
    /// 批量处理进行中（防重入；进度经 BatchEvent 回传）
    pub batch_running: bool,
    /// 上次批量输出目录（重试失败项时复用）
    pub batch_out_dir: Option<PathBuf>,
    pub last_bg: [u8; 3],
    /// 扫描到的神经网络模型（models/ 目录，插件式发现）
    pub nn_models: Vec<crate::core::nn::NnModelInfo>,
    /// 实际使用的模型目录（None = 目录不存在）
    pub models_dir: Option<std::path::PathBuf>,
}

impl WorkState {
    /// 作废所有在途抠图：推进世代 + 通知取消 + 复位 busy。
    ///
    /// 换图时必须调用：否则上一张图那条不可中断的后台推理仍会回传，把
    /// A 图的 result / mask / 显示句柄整套覆盖到 B 图上，并误复位 busy。
    /// 世代推进后 `MaskDone(seq, ..)` 会整包丢弃旧帧；取消令牌让色键路径
    /// 与推理闸门轮询尽早退出（ONNX 的 `run()` 本身无法中途打断）。
    pub fn invalidate_inflight_matte(&mut self) {
        self.recompute_seq += 1;
        let _ = self.matte_cancel_tx.send(self.recompute_seq);
        self.busy = false;
    }
}

impl Default for WorkState {
    fn default() -> Self {
        let (matte_cancel_tx, matte_cancel_rx) = tokio::sync::watch::channel(0u64);
        Self {
            src: None,
            fields: std::sync::Arc::new(std::sync::Mutex::new(
                crate::core::matting::FieldCache::default(),
            )),
            recompute_seq: 0,
            debounce_seq: 0,
            exporting: false,
            export_seq: 0,
            loading: false,
            frame: 0.0,
            matte_cancel_tx,
            matte_cancel_rx,
            result: None,
            mask: None,
            base: None,
            result_handle: None,
            orig_handle: None,
            result_preview: None,
            orig_preview: None,
            stem: "image".into(),
            src_path: None,
            stage: (920.0, 620.0),
            params: MatteParams::default(),
            brush_size: 18,
            brush_opacity: 100,
            brush_flow: 100,
            hardness_keep: 85,
            hardness_erase: 85,
            brush_props_open: false,
            tool: Tool::Auto,
            view_mode: ViewMode::Original,
            bg_preview: BgPreview::Checker,
            out_color: "transparent".into(),
            zoom: 1.0,
            pan: (0.0, 0.0),
            fit_zoom: 1.0,
            compare_at: 0.5,
            busy: false,
            metrics: Metrics::default(),
            history: History::default(),
            rot_total: 0.0,
            rot_deg: 0.0,
            crop: None,
            crop_ratio: CropRatio::Free,
            crop_initial: None,
            last_crop_click: None,
            drag: None,
            drag_from: None,
            brush_cursor: None,
            last_paint: None,
            picker_at: None,
            picker_hover: None,
            refresh_seq: 0,
            custom_hex: "#FFFFFF".into(),
            batch: Vec::new(),
            batch_running: false,
            batch_out_dir: None,
            last_bg: [255, 255, 255],
            nn_models: Vec::new(),
            models_dir: None,
        }
    }
}

/// 批量队列项状态（参考项目 batch 面板的四种状态）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl QueueStatus {
    pub fn label(&self) -> &'static str {
        match self {
            QueueStatus::Pending => tr("排队中"),
            QueueStatus::Running => tr("处理中"),
            QueueStatus::Done => tr("完成"),
            QueueStatus::Failed => tr("失败"),
        }
    }
}

/// 证件照批量导入队列项
#[derive(Debug, Clone)]
/// 工作台批量队列项（与证件照页 IdQueueItem 同构：状态徽标 + 失败明细）
pub struct WorkBatchItem {
    pub path: PathBuf,
    pub status: QueueStatus,
    /// 失败原因（仅 Failed 状态）
    pub err: Option<String>,
}

pub struct IdQueueItem {
    pub path: std::path::PathBuf,
    pub stem: String,
    pub status: QueueStatus,
}

pub struct IdState {
    /// 抠图后的前景（带 alpha，全分辨率），Arc 共享避免深拷贝
    pub fg: Option<std::sync::Arc<RgbaImage>>,
    /// 当前照片的源文件路径（内存来源如示例图为 None）。
    /// 切换抠图模型时据此重跑 载入→抠图 链，无需在内存里另存一份原图
    pub src_path: Option<std::path::PathBuf>,
    /// 预览工作副本（长边 ≤ PREVIEW_LONG 的降采样版）：
    /// 滑块渲染链（腐蚀/旋转/合成）全部在预览尺寸上跑，
    /// 全分辨率 fg 只在导出/批量导出时使用 —— 每次滑块重算从秒级降到毫秒级
    pub fg_preview: Option<std::sync::Arc<RgbaImage>>,
    /// 用于导出文件名的前缀
    pub stem: String,
    pub src_handle: Option<Handle>,
    pub out_handle: Option<Handle>,
    /// 相纸排版预览
    pub sheet_handle: Option<Handle>,
    pub busy: bool,
    pub view: IdView,
    /// 批量导入队列（参考项目 batch 面板：逐张处理，可载入 / 移除）
    pub queue: Vec<IdQueueItem>,
    /// 正在处理的队列项索引（None = 非队列来源的载入）
    pub queue_active: Option<usize>,
    pub spec_idx: usize,
    pub bg_idx: usize,
    pub render: RenderMode,
    pub zoom: i32,
    pub off_y: i32,
    pub erode: i32,
    pub paper_idx: usize,
    pub cutline: bool,
    pub realsize: bool,
    pub dpi: u32,
    /// 屏幕物理 DPI（由系统缩放因子自动计算，用于「真实尺寸」预览）
    pub screen_dpi: f32,
    pub meta: String,
    /// 后台渲染序列号：每次编辑参数自增，回传时比对，丢弃过期帧
    pub render_seq: u64,
    /// 载入→抠图链的世代：每次开始载入新照片（选图 / 切模型 / 队列下一张）
    /// 自增；`PhotoLoaded` / `MatteDone` 携带发起时的 seq，过期帧整包丢弃，
    /// 避免上一张的抠图结果覆盖当前照片或把队列项误标 Done。
    pub load_seq: u64,
    /// 缓存的单张预览 meta（切到单张视图无需重算即可得到文案）
    pub single_meta: String,
    /// 缓存的排版预览 meta（切到排版视图无需重算即可得到文案）
    pub sheet_meta: String,
    /// 缓存腐蚀后的前景图（预览尺寸；erode/render 不变时复用，换照片作废）
    pub fg_eroded: Option<std::sync::Arc<image::RgbaImage>>,
    /// 腐蚀缓存键：(erode, render_mode, 预览宽, 预览高)——
    /// 尺寸进键，换照片后旧缓存绝不误用（旧实现只比对参数，存在跨照片串缓存隐患）
    pub erode_cache_key: Option<(i32, RenderMode, u32, u32)>,
    /// 取消令牌：每次 render 前取消旧任务，避免过期渲染占用 CPU
    pub cancel_tx: tokio::sync::watch::Sender<u64>,
    /// 取消令牌接收端副本（用于在 render 中创建新的 receiver 传入后台线程）
    pub cancel_rx: tokio::sync::watch::Receiver<u64>,
    /// 当前选中的抠图模型 id（对应 nn_models 中的 NnModelInfo.id）
    pub model_id: Option<String>,
    /// 图片载入中（解码阶段；busy = 抠图阶段）——驱动载入遮罩
    pub loading: bool,
    /// 旋转 spinner 帧计数（时间订阅驱动）
    pub frame: f32,
    /// 预取解码单槽：批量模式下「解码第 N+1 张 ∥ 抠图第 N 张」，
    /// 值 = (队列索引, 解码结果)。仅在索引与「下一张待处理项」匹配时被消费
    pub prefetch: Option<(usize, Result<Box<crate::message::LoadedPayload>, String>)>,
    /// 正在后台同步解码的队列项（防止预取与同步解码重复解码同一张）
    pub decode_inflight: Option<usize>,
    /// 单张导出格式（PNG / JPG / WEBP，随 config 持久化）
    pub export_fmt: crate::core::image::ExportFormat,
    /// 批量导出进行中（防重入；进度经 BatchEvent 回传）
    pub batch_exporting: bool,
    /// 旋转校正（度，-15..=15）
    pub rot: i32,
    /// 自定义底色开关 + HEX
    pub use_custom_bg: bool,
    pub custom_bg_hex: String,
}

impl IdState {
    /// 开启一条新的「载入→抠图」链，返回该链的世代号。
    ///
    /// 选新图 / 切模型 / 队列推进到下一张时调用：旧链的 `PhotoLoaded` 与
    /// `MatteDone` 回传时 seq 不再相等，会被整包丢弃（不覆盖 fg、不复位
    /// busy、不误标队列项完成）。
    pub fn begin_load_chain(&mut self) -> u64 {
        self.load_seq = self.load_seq.wrapping_add(1);
        // 旧链作废后它的回传不会再复位状态，必须在这里清干净；否则新链
        // 解码失败时会留下永久 busy（spinner 不停、按钮一直禁用）。
        // 各入口随后按需重新置 loading / busy，start_photo 会置 busy=true。
        self.busy = false;
        self.loading = false;
        self.load_seq
    }
}

impl Default for IdState {
    fn default() -> Self {
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(0u64);
        Self {
            fg: None,
            src_path: None,
            fg_preview: None,
            loading: false,
            frame: 0.0,
            stem: "photo".to_string(),
            src_handle: None,
            out_handle: None,
            sheet_handle: None,
            busy: false,
            view: IdView::Single,
            spec_idx: 0,
            bg_idx: 0,
            render: RenderMode::Solid,
            zoom: 100,
            off_y: 0,
            erode: 1,
            paper_idx: 0,
            cutline: true,
            realsize: true,
            dpi: ID_DPI,
            screen_dpi: get_system_dpi(),
            meta: String::new(),
            render_seq: 0,
            load_seq: 0,
            single_meta: String::new(),
            sheet_meta: String::new(),
            queue: Vec::new(),
            queue_active: None,
            fg_eroded: None,
            erode_cache_key: None,
            cancel_tx,
            cancel_rx,
            model_id: None,
            prefetch: None,
            decode_inflight: None,
            export_fmt: crate::core::image::ExportFormat::Png,
            batch_exporting: false,
            rot: 0,
            use_custom_bg: false,
            custom_bg_hex: "#438EDB".to_string(),
        }
    }
}

/* ═════════════════ 自更新状态 ═════════════════ */

/// 更新流程状态机：Idle → Checking → Available → Downloading → Ready，
/// 任何一步失败落到 Failed（保留原因文案，UI 提供重试 / 前往下载页）。
#[derive(Debug, Clone)]
pub enum UpdateStatus {
    /// 初始 / 关闭后回到的空闲态
    Idle,
    /// 正在检查
    Checking,
    /// 已是最新
    UpToDate,
    /// 发现新版本
    Available(Box<ReleaseInfo>),
    /// 下载中（保留 ReleaseInfo 以便完成后转入 Ready）
    Downloading {
        info: Box<ReleaseInfo>,
        received: u64,
        total: Option<u64>,
    },
    /// 安装包就绪，等待「重启并安装」
    Ready {
        info: Box<ReleaseInfo>,
        path: PathBuf,
    },
    /// 检查或下载失败（`info` 为 Some 表示下载失败——可重试下载；文案直接给 UI 展示）
    Failed {
        msg: String,
        info: Option<Box<ReleaseInfo>>,
    },
}

/// 自更新模块状态（挂在 App 上）
pub struct UpdaterState {
    pub status: UpdateStatus,
    /// 更新对话框是否展开
    pub dialog_open: bool,
    /// 下载取消令牌（Downloading 期间有效；置位后线程清理半成品退出）
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for UpdaterState {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
            dialog_open: false,
            cancel: None,
        }
    }
}

/* ═════════════════ 启动 ═════════════════ */

impl App {
    pub fn boot() -> (Self, Task<Message>) {
        let t0 = std::time::Instant::now();
        let config = Config::load();
        eprintln!("[boot] 配置读取 {:?}", t0.elapsed());
        let first = !config.seen_onboarding;
        // 主题：模式解析 + 自定义强调色；语言
        let system_dark = crate::theme::system_prefers_dark();
        let boot_mode = crate::theme::ThemeMode::from_code(&config.appearance.theme_mode);
        crate::theme::set_resolved(boot_mode.resolved(system_dark));
        crate::theme::set_accent(
            config
                .appearance
                .accent_hex
                .as_deref()
                .and_then(parse_hex_color),
        );
        crate::core::i18n::set_lang(crate::core::i18n::Lang::from_str(&config.appearance.lang));
        let mut app = Self {
            // 原项目首屏就是工作台（index.html 里 `#pageWork` 带 `on`），首页是可选项
            page: Page::Work,
            nav_collapsed: config.nav_collapsed,
            win_w: 1440.0,
            win_h: 900.0,
            work: WorkState {
                params: config.params.clone(),
                brush_size: config.brush.size,
                brush_opacity: config.brush.opacity,
                brush_flow: config.brush.flow,
                hardness_keep: config.brush.hardness_keep,
                hardness_erase: config.brush.hardness_erase,
                out_color: config.out_color.clone(),
                ..WorkState::default()
            },
            about_md: Vec::new(),
            about_slogan: String::new(),
            about_author: None,
            id: IdState {
                spec_idx: config.id.spec_idx,
                bg_idx: config.id.bg_idx,
                render: config.id.render,
                zoom: config.id.zoom,
                off_y: config.id.off_y,
                erode: config.id.erode,
                paper_idx: config.id.paper_idx,
                cutline: config.id.cutline,
                realsize: config.id.realsize,
                dpi: config.id.dpi,
                export_fmt: match config.id.export_fmt.as_str() {
                    "jpg" => crate::core::image::ExportFormat::Jpg,
                    "webp" => crate::core::image::ExportFormat::Webp,
                    _ => crate::core::image::ExportFormat::Png,
                },
                rot: config.id.rot,
                model_id: config.id.model_id.clone(),
                use_custom_bg: config.id.use_custom_bg,
                custom_bg_hex: config.id.custom_bg_hex.clone(),
                ..IdState::default()
            },
            settings_panel: 0,
            system_dpi: get_system_dpi(),
            dpi_detected: Some(true),
            config,
            toasts: Vec::new(),
            toast_seq: 0,
            cpu_text: sys_text::cpu(),
            mem_text: sys_text::mem(),
            updater: UpdaterState::default(),
            drop_hover: false,
            theme_mode: boot_mode,
            system_dark,
            msgs: Vec::new(),
            unread: 0,
            msg_center_open: false,
            guide_step: None,
            win_id: None,
            win_maximized: false,
            win_frame_fixed: false,
            sponsor_open: false,
            sponsor_amount: 9.9,
            sponsor_custom: String::new(),
        };
        if first {
            // 首次运行的欢迎 toast：普通提示，不属于分步新手引导 UI
            // （引导内容只有首页「新手引导」按钮能开启），对应原项目 hasSeen()
            app.toast(tr("欢迎使用壹鸣 · 拖入图片即可开始"));
            app.config.seen_onboarding = true;
            app.config.save();
        }
        // 关于页 Markdown 预载（进入页面时也会刷新，读取外部编辑后的内容）
        ui::page_about::load_markdown(&mut app);
        eprintln!("[boot] 关于页内容 {:?}", t0.elapsed());
        // 启动即扫描 models/ 目录 + 外置插件目录（插件式模型发现）
        let plugin_dir = app.config.plugin_dir.as_deref();
        let (mdir, models) = crate::core::nn::scan(plugin_dir);
        eprintln!("[boot] 模型扫描 {:?}", t0.elapsed());
        app.work.models_dir = mdir;
        app.work.nn_models = models;
        // 诊断钩子（与 canvas_view 的 COCOLE_DEBUG_CANVAS 同族）：
        // COCOLE_DEBUG_VIEW=compare|original 让启动即落到指定视图模式。
        // iced 无法从外部脚本驱动控件，截图验证只能靠它直达目标状态。
        if let Ok(v) = std::env::var("COCOLE_DEBUG_VIEW") {
            match v.as_str() {
                "compare" => app.work.view_mode = ViewMode::Compare,
                "original" => app.work.view_mode = ViewMode::Original,
                _ => {}
            }
        }
        // COCOLE_DEBUG_TOOL=crop|keep|erase|picker|pan|auto 同理，用于覆盖层截图验证
        if let Ok(v) = std::env::var("COCOLE_DEBUG_TOOL") {
            app.work.tool = match v.as_str() {
                "crop" => Tool::Crop,
                "keep" => Tool::Keep,
                "erase" => Tool::Erase,
                "picker" => Tool::Picker,
                "pan" => Tool::Pan,
                _ => Tool::Auto,
            };
        }
        // COCOLE_DEBUG_BRUSH=1：启动即展开「笔刷属性」下拉面板（仅 keep/erase 工具下有效）
        if std::env::var_os("COCOLE_DEBUG_BRUSH").is_some()
            && matches!(app.work.tool, Tool::Keep | Tool::Erase)
        {
            app.work.brush_props_open = true;
        }
        // COCOLE_DEBUG_COMPARE=0.0..1.0 直接设定对比分割线位置（相对图片宽度），
        // 用于在无法合成鼠标事件时截图验证分割线 / 角标的定位。
        if let Ok(v) = std::env::var("COCOLE_DEBUG_COMPARE") {
            if let Ok(t) = v.parse::<f32>() {
                app.work.compare_at = t.clamp(0.0, 1.0);
            }
        }
        // 命令行带图片路径时直接打开（`cocole-iced photo.png`），方便批处理与自动化
        let startup = std::env::args()
            .nth(1)
            .filter(|a| !a.starts_with('-'))
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .map(|p| Task::done(Message::Work(WorkMessage::OpenPath(p))));
        let welcome = app.expire_tasks(0);
        // 启动自动检查更新：开关开启且距上次检查超过 24h 才发起；
        // 延迟 3s 错开启动高峰（模型扫描 / 首帧渲染），失败静默不打扰。
        let auto_check = if app.config.updates.auto_check
            && updater::should_auto_check(app.config.updates.last_check)
        {
            Task::perform(
                async {
                    tokio::time::sleep(Duration::from_millis(updater::AUTO_CHECK_DELAY_MS)).await;
                    updater::check().await.map_err(|e| e.to_string())
                },
                |result| {
                    Message::Updater(UpdaterMessage::Checked {
                        manual: false,
                        result,
                    })
                },
            )
        } else {
            Task::none()
        };
        // 解析主窗口 Id（自绘标题栏的拖动 / 最小化 / 最大化 / 关闭任务都要用），
        // 并回查一次最大化状态（iced 0.14 无 MAIN 常量，只能经 oldest() 取）
        let win_id = iced::window::oldest().map(Message::WinIdResolved);
        (
            app,
            match startup {
                Some(t) => Task::batch([welcome, t, auto_check, win_id]),
                None => Task::batch([welcome, auto_check, win_id]),
            },
        )
    }

    pub fn title(&self) -> String {
        match self.page {
            Page::Home | Page::Work | Page::IdPhoto | Page::Settings | Page::About => {
                tr("虽无鸣 . 鸣必惊人").to_string()
            }
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), false);
    }

    /// 失败类提示（原项目 `toast(msg, true)`：红底）
    pub fn toast_err(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), true);
    }

    fn push_toast(&mut self, text: String, err: bool) {
        self.toast_seq += 1;
        if std::env::var_os("COCOLE_DEBUG_CANVAS").is_some() {
            eprintln!("[toast] +{} err={} {}", self.toast_seq, err, text);
        }
        self.toasts.push(Toast {
            id: self.toast_seq,
            text: text.clone(),
            err,
        });
        // 消息中心镜像：所有 toast 入档（限 100 条），未读计数 +1
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.msgs.insert(
            0,
            MsgItem {
                id: self.toast_seq,
                title: text.clone(),
                err,
                ts_ms: ts,
                read: false,
            },
        );
        self.msgs.truncate(100);
        self.unread += 1;
        // 错误类事件发系统通知（免打扰时段静默）
        if err {
            crate::core::notify::system_notify("壹鸣 Wonderland", &text, &self.config.dnd);
        }
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    /// 把「刚入队」的提示各挂一个到期任务，到点自动消失（不需要常驻定时器）
    pub fn expire_tasks(&self, since: u64) -> Task<Message> {
        let ids: Vec<u64> = self
            .toasts
            .iter()
            .map(|t| t.id)
            .filter(|id| *id > since)
            .collect();
        if ids.is_empty() {
            return Task::none();
        }
        Task::batch(ids.into_iter().map(toast_timer))
    }

    /// 参数写回配置并落盘
    pub fn persist(&mut self) {
        self.config.params = self.work.params.clone();
        self.config.out_color = self.work.out_color.clone();
        self.config.brush.size = self.work.brush_size;
        self.config.brush.opacity = self.work.brush_opacity;
        self.config.brush.flow = self.work.brush_flow;
        self.config.brush.hardness_keep = self.work.hardness_keep;
        self.config.brush.hardness_erase = self.work.hardness_erase;
        self.config.nav_collapsed = self.nav_collapsed;
        self.config.id.spec_idx = self.id.spec_idx;
        self.config.id.bg_idx = self.id.bg_idx;
        self.config.id.render = self.id.render;
        self.config.id.zoom = self.id.zoom;
        self.config.id.off_y = self.id.off_y;
        self.config.id.erode = self.id.erode;
        self.config.id.paper_idx = self.id.paper_idx;
        self.config.id.cutline = self.id.cutline;
        self.config.id.realsize = self.id.realsize;
        self.config.id.export_fmt = match self.id.export_fmt {
            crate::core::image::ExportFormat::Png => "png",
            crate::core::image::ExportFormat::Jpg => "jpg",
            crate::core::image::ExportFormat::Webp => "webp",
        }
        .to_string();
        self.config.id.rot = self.id.rot;
        self.config.id.model_id = self.id.model_id.clone();
        self.config.id.use_custom_bg = self.id.use_custom_bg;
        self.config.id.custom_bg_hex = self.id.custom_bg_hex.clone();
        self.config.save();
    }
}

/// 机器信息文案（对应原项目 Tauri 的 `sysinfo` 命令）
mod sys_text {
    use crate::core::i18n::{tr, trf};

    pub fn cpu() -> String {
        let n = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(0);
        if n > 0 {
            trf("CPU {} 线程", &[&n.to_string()])
        } else {
            "CPU —".to_string()
        }
    }

    /// 原生版不做精确内存探测，沿用原项目的估算口径（线程数 × 0.5 GB）
    pub fn mem() -> String {
        let n = std::thread::available_parallelism()
            .map(|v| v.get())
            .unwrap_or(0) as f32;
        if n > 0.0 {
            trf("内存 ≈{} GB 可用", &[&((n * 0.5) as u32).to_string()])
        } else {
            tr("内存 —").to_string()
        }
    }
}

/* ═════════════════ update：只做分发 ═════════════════ */

/// 入口：处理提示条生命周期，其余消息转给 `dispatch`
///
/// 任何一条消息跑完后，若队列里多了新提示，就顺手挂一个 2.4s 的到期任务 ——
/// 这样不需要常驻定时器（空闲时零开销）。
pub fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::ToastExpire(id) => {
            app.toasts.retain(|t| t.id != id);
            if std::env::var_os("COCOLE_DEBUG_CANVAS").is_some() {
                eprintln!("[toast] -{} (队列剩 {})", id, app.toasts.len());
            }
            Task::none()
        }
        other => {
            let before = app.toast_seq;
            let task = dispatch(app, other);
            let expire = app.expire_tasks(before);
            Task::batch([task, expire])
        }
    }
}

/// 单条提示的到期清理任务
fn toast_timer(id: u64) -> Task<Message> {
    Task::perform(
        async move {
            tokio::time::sleep(Duration::from_millis(TOAST_MS)).await;
        },
        move |_| Message::ToastExpire(id),
    )
}

/// 推理完成后安排延迟会话清扫：15s 内无新推理（世代号未变）→ 释放全部会话
pub(crate) fn schedule_session_sweep() -> Task<Message> {
    let gen = crate::core::nn::session_generation();
    Task::perform(
        tokio::time::sleep(std::time::Duration::from_secs(15)),
        move |_instant| Message::SessionSweep(gen),
    )
}

/// `#RRGGBB` → Color（非法输入返回 None）
pub(crate) fn parse_hex_color(hex: &str) -> Option<iced::Color> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(iced::Color::from_rgb8(r, g, b))
}

fn dispatch(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => {
            app.page = page;
            // 进入关于页即重新读取 content/about/*.md（外部编辑免重启生效）
            if page == Page::About {
                ui::page_about::load_markdown(app);
            }
            // 新手引导**只能**由首页「新手引导」按钮主动开启（StartGuide）。
            // 这里刻意不做任何自动弹出：启动、刷新、首次进入首页、从其他页
            // 切回首页都不展示引导内容，guide_done 也不再参与显示判定。
            // 因此切页不会凭空制造 StartGuide 消息，其它操作也无法误触发。
            // 离开首页仍收起引导并清空过期的目标坐标（窗口 resize / DPI
            // 变化后未重新 draw 的首页若沿用旧绝对坐标会画错气泡锚点）。
            if page != Page::Home {
                app.guide_step = None;
                ui::guide::clear();
            }
            Task::none()
        }
        Message::SessionSweep(gen) => {
            // 世代号匹配 = 期间没有新推理 → 会话已空闲，释放归还显存
            if crate::core::nn::session_generation() == gen {
                crate::core::nn::sweep_all_sessions();
            }
            Task::none()
        }
        Message::SetThemeMode(mode) => {
            app.config.appearance.theme_mode = mode.code().to_string();
            theme::set_resolved(mode.resolved(app.system_dark));
            app.persist();
            Task::none()
        }
        Message::SetAccent(hex) => {
            app.config.appearance.accent_hex = hex.clone();
            let color = hex.as_deref().and_then(parse_hex_color);
            theme::set_accent(color);
            app.persist();
            Task::none()
        }
        Message::SetLang(l) => {
            crate::core::i18n::set_lang(l);
            app.config.appearance.lang = l.code().to_string();
            app.persist();
            Task::none()
        }
        Message::ToggleDnd => {
            app.config.dnd.enabled = !app.config.dnd.enabled;
            app.persist();
            Task::none()
        }
        Message::SetDndTimes(start, end) => {
            app.config.dnd.start = start;
            app.config.dnd.end = end;
            app.persist();
            Task::none()
        }
        Message::ToggleMsgCenter => {
            app.msg_center_open = !app.msg_center_open;
            if app.msg_center_open {
                for m in &mut app.msgs {
                    m.read = true;
                }
                app.unread = 0;
            }
            Task::none()
        }
        Message::MarkAllMsgsRead => {
            for m in &mut app.msgs {
                m.read = true;
            }
            app.unread = 0;
            Task::none()
        }
        Message::AboutReload => {
            ui::page_about::load_markdown(app);
            app.toast(tr("关于页内容已重新加载"));
            Task::none()
        }
        Message::OpenUrl(url) => {
            // 用系统默认浏览器打开外链（隐藏 cmd 窗口）
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                let _ = std::process::Command::new("cmd")
                    .args(["/c", "start", "", &url])
                    .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                    .spawn();
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            }
            Task::none()
        }
        Message::ToggleNav => {
            app.nav_collapsed = !app.nav_collapsed;
            app.persist();
            Task::none()
        }
        Message::WinResized(w, h) => {
            app.win_w = w;
            app.win_h = h;
            // 最大化 / 还原也会触发 resize —— 顺手回查状态，驱动标题栏图标切换
            let maximize = match app.win_id {
                Some(id) => iced::window::is_maximized(id).map(Message::WinMaximized),
                None => Task::none(),
            };
            // 圆角 / 阴影尚未恢复成功时随 resize 重试（窗口早期可能尚未就绪）
            let frame = fix_window_frame(app);
            Task::batch([maximize, frame])
        }
        Message::WinIdResolved(id) => {
            app.win_id = id;
            let maximize = match id {
                Some(id) => iced::window::is_maximized(id).map(Message::WinMaximized),
                None => Task::none(),
            };
            let frame = fix_window_frame(app);
            Task::batch([maximize, frame])
        }
        Message::WinFrameFixed(ok) => {
            app.win_frame_fixed = ok;
            Task::none()
        }
        Message::WinMaximized(b) => {
            app.win_maximized = b;
            Task::none()
        }
        // 自绘标题栏：拖动 / 最小化 / 最大化还原 / 关闭（iced 0.14 原生窗口任务）
        Message::Titlebar(m) => match m {
            TitlebarMessage::Drag => iced::window::oldest().and_then(iced::window::drag),
            TitlebarMessage::Minimize => {
                iced::window::oldest().and_then(|id| iced::window::minimize(id, true))
            }
            TitlebarMessage::ToggleMaximize => iced::window::oldest().and_then(|id| {
                Task::batch([
                    iced::window::toggle_maximize(id),
                    iced::window::is_maximized(id).map(Message::WinMaximized),
                ])
            }),
            TitlebarMessage::Close => iced::window::oldest().and_then(iced::window::close),
        },
        Message::Sponsor(m) => match m {
            SponsorMessage::Open(open) => {
                app.sponsor_open = open;
                if open {
                    app.sponsor_custom.clear();
                }
                Task::none()
            }
            SponsorMessage::Amount(v) => {
                app.sponsor_amount = v;
                Task::none()
            }
            SponsorMessage::CustomInput(s) => {
                app.sponsor_custom = s;
                Task::none()
            }
        },
        // 拖拽悬停：只驱动首页拖放区高亮（幂等，事件重复无副作用）
        Message::DropHover(hover) => {
            app.drop_hover = hover;
            Task::none()
        }
        // 拖拽落下：按当前页路由 —— 证件照页进证件照链路，
        // 其余页（首页 / 工作台）统一送抠图工作台载入
        Message::DroppedFile(path) => {
            app.drop_hover = false;
            match app.page {
                Page::IdPhoto => ui::page_idphoto::update(app, IdMessage::FilePicked(Some(path))),
                _ => {
                    app.page = Page::Work;
                    ui::page_work::update(app, WorkMessage::OpenPath(path))
                }
            }
        }
        // 旋转 spinner 帧推进（仅在载入/处理期间订阅激活）
        Message::SpinnerTick => {
            app.work.frame += 1.0;
            app.id.frame += 1.0;
            Task::none()
        }
        // `ToastExpire` 已在 update() 里处理，这里不会到达
        Message::ToastExpire(_) => Task::none(),

        Message::Work(msg) => ui::page_work::update(app, msg),
        Message::IdPhoto(msg) => ui::page_idphoto::update(app, msg),
        Message::Updater(msg) => update_updater(app, msg),
        Message::Settings(msg) => {
            if let SettingsMessage::RefreshDpi = &msg {
                app.system_dpi = get_system_dpi();
                app.dpi_detected = Some(true);
            }
            ui::page_settings::update(app, msg)
        }
        Message::Home(msg) => ui::page_home::update(app, msg),
    }
}

/* ═════════════════ 自更新：update 分支 ═════════════════ */

/// 自更新消息处理。原则：
/// - 手动检查失败要 toast；后台自动检查失败静默（断网不打扰）。
/// - 「忽略此版本」只压制自动弹窗，手动检查仍可看到该版本。
/// - 强制更新（manifest.mandatory）不提供忽略入口。
fn update_updater(app: &mut App, msg: UpdaterMessage) -> Task<Message> {
    use crate::core::updater::DownloadEvent;

    match msg {
        UpdaterMessage::Check => {
            app.updater.status = UpdateStatus::Checking;
            Task::perform(
                async { updater::check().await.map_err(|e| e.to_string()) },
                |result| {
                    Message::Updater(UpdaterMessage::Checked {
                        manual: true,
                        result,
                    })
                },
            )
        }
        UpdaterMessage::Checked { manual, result } => {
            app.config.updates.last_check = Some(updater::now_unix());
            app.config.save();
            match result {
                Ok(updater::CheckOutcome::UpToDate) => {
                    app.updater.status = UpdateStatus::UpToDate;
                    if manual {
                        app.toast(trf(
                            "已是最新版本（v{}）",
                            &[updater::current_version_text()],
                        ));
                    }
                }
                Ok(updater::CheckOutcome::Update(info)) => {
                    let skipped = app.config.updates.skipped_version.as_deref()
                        == Some(info.version.as_str());
                    app.updater.status = UpdateStatus::Available(info.clone());
                    // 通知推送：手动检查总是弹窗；自动检查被忽略版本不弹窗，仅 toast 一次
                    if manual {
                        app.updater.dialog_open = true;
                    } else if skipped {
                        app.toast(trf(
                            "发现新版本 v{}（你已选择忽略此版本）",
                            &[&info.version],
                        ));
                    } else {
                        app.updater.dialog_open = true;
                        app.toast(trf("发现新版本 v{}", &[&info.version]));
                    }
                }
                Err(e) => {
                    app.updater.status = UpdateStatus::Failed {
                        msg: e.clone(),
                        info: None,
                    };
                    if manual {
                        app.toast_err(trf("检查更新失败：{}", &[&e]));
                    } else {
                        eprintln!("[updater] 后台检查失败（静默）：{e}");
                    }
                }
            }
            Task::none()
        }
        UpdaterMessage::StartInstall => {
            let (info, dest) = match &app.updater.status {
                UpdateStatus::Available(info) => {
                    (info.clone(), updater::installer_dest(&info.version))
                }
                // 下载失败后的「重试下载」也走这里
                UpdateStatus::Failed {
                    info: Some(info), ..
                } => (info.clone(), updater::installer_dest(&info.version)),
                _ => return Task::none(),
            };
            let Some(dest) = dest else {
                app.toast_err(tr("无法确定临时目录，下载中止"));
                return Task::none();
            };
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.updater.cancel = Some(cancel.clone());
            app.updater.status = UpdateStatus::Downloading {
                received: 0,
                total: info.size,
                info: info.clone(),
            };
            updater::download_task(&info, dest, cancel)
        }
        UpdaterMessage::DownloadEvent(DownloadEvent::Progress { received, total }) => {
            if let UpdateStatus::Downloading { info, .. } = &app.updater.status {
                app.updater.status = UpdateStatus::Downloading {
                    info: info.clone(),
                    received,
                    total: total.or(info.size),
                };
            }
            Task::none()
        }
        UpdaterMessage::DownloadEvent(DownloadEvent::Done { path, .. }) => {
            if let UpdateStatus::Downloading { info, .. } = &app.updater.status {
                app.updater.status = UpdateStatus::Ready {
                    info: info.clone(),
                    path: path.clone(),
                };
                app.updater.cancel = None;
            }
            Task::none()
        }
        UpdaterMessage::DownloadEvent(DownloadEvent::Failed(e)) => {
            // 下载失败：保留 ReleaseInfo，对话框里可「重试下载」
            let info = match &app.updater.status {
                UpdateStatus::Downloading { info, .. } => Some(info.clone()),
                _ => None,
            };
            app.updater.status = UpdateStatus::Failed {
                msg: e.clone(),
                info,
            };
            app.updater.cancel = None;
            app.toast_err(trf("下载失败：{}", &[&e]));
            Task::none()
        }
        UpdaterMessage::CancelDownload => {
            if let Some(cancel) = &app.updater.cancel {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            app.updater.status = UpdateStatus::Idle;
            Task::none()
        }
        UpdaterMessage::SkipVersion => {
            let version = match &app.updater.status {
                UpdateStatus::Available(info) => info.version.clone(),
                _ => return Task::none(),
            };
            app.config.updates.skipped_version = Some(version.clone());
            app.config.save();
            app.updater.dialog_open = false;
            app.updater.status = UpdateStatus::Idle;
            app.toast(trf("已忽略 v{}，之后可在「关于」页恢复提醒", &[&version]));
            Task::none()
        }
        UpdaterMessage::ShowDialog => {
            app.updater.dialog_open = true;
            Task::none()
        }
        UpdaterMessage::Dismiss => {
            app.updater.dialog_open = false;
            Task::none()
        }
        UpdaterMessage::QuitAndInstall => {
            let path = match &app.updater.status {
                UpdateStatus::Ready { path, .. } => path.clone(),
                _ => return Task::none(),
            };
            if let Err(e) = updater::launch_installer(&path) {
                app.toast_err(e);
                return Task::none();
            }
            // 安装器已拉起：退出应用（iced::exit 结束事件循环）
            iced::exit()
        }
        UpdaterMessage::OpenDownloadPage => {
            app.updater.dialog_open = false;
            if let Err(e) = updater::open_in_browser(updater::DOWNLOAD_PAGE_URL) {
                app.toast_err(e);
            }
            Task::none()
        }
        UpdaterMessage::ToggleAutoCheck(v) => {
            app.config.updates.auto_check = v;
            app.config.save();
            Task::none()
        }
        UpdaterMessage::ClearSkipped => {
            app.config.updates.skipped_version = None;
            app.config.save();
            app.toast(tr("已恢复新版本提醒"));
            Task::none()
        }
    }
}

/* ═════════════════ 键盘快捷键 ═════════════════ */

/// 键盘快捷键（对应参考项目 bind.js 末尾的 keydown 分支）：
/// 工具 A/B/E/I/M/C、0 适应窗口、[ ] 笔刷大小、Ctrl+S 导出、Ctrl+Z 撤销、
/// Ctrl+B 折叠导航；裁剪模式下 Enter 确认 / Esc 取消。
///
/// `keyboard::listen()` 只派发未被控件消费的按键（Ignored）：文本输入框
/// （HEX 输入等）聚焦时按键被控件捕获，不会误触快捷键 —— 等价于参考项目
/// 「target 是 INPUT 就跳过」。
pub fn subscription(app: &App) -> iced::Subscription<Message> {
    use iced::keyboard::{key::Named, Event as KbdEvent, Key};

    // 窗口尺寸变化 —— 所有页面都要监听（首页响应式断点依赖它）
    let resize =
        window::resize_events().map(|(_id, size)| Message::WinResized(size.width, size.height));

    // 文件拖拽 —— 全局监听：悬停高亮 + 落下按当前页路由载入。
    // （iced 0.14 由 winit 原生转发 FileHovered / FileDropped / FilesHoveredLeft，
    //   原生窗口无浏览器默认行为需要阻止。）
    let files = window::events().filter_map(|(_id, ev)| match ev {
        window::Event::FileDropped(path) => Some(Message::DroppedFile(path)),
        window::Event::FileHovered(_) => Some(Message::DropHover(true)),
        window::Event::FilesHoveredLeft => Some(Message::DropHover(false)),
        _ => None,
    });

    // 旋转 spinner 驱动：仅载入/处理期间激活（空闲时订阅不存在，零开销）
    let spinner = if app.work.loading || app.work.busy || app.work.exporting || app.id.loading || app.id.busy {
        iced::time::every(std::time::Duration::from_millis(50)).map(|_| Message::SpinnerTick)
    } else {
        iced::Subscription::none()
    };

    if app.page != Page::Work {
        // 证件照页快捷键：E 导出单张 / Shift+E 导出排版 / 1·2 切视图。
        // （keyboard::listen 只派发未被控件消费的按键，HEX 输入框聚焦时不会误触）
        let kbd = if app.page == Page::IdPhoto {
            iced::keyboard::listen().filter_map(|ev| match ev {
                KbdEvent::KeyPressed { key, modifiers, .. } => {
                    let ctrl = modifiers.control() || modifiers.logo();
                    if ctrl {
                        return None;
                    }
                    let Key::Character(c) = &key else { return None };
                    match c.to_lowercase().as_str() {
                        "e" => Some(Message::IdPhoto(if modifiers.shift() {
                            IdMessage::ExportSheet
                        } else {
                            IdMessage::ExportSingle
                        })),
                        "1" => Some(Message::IdPhoto(IdMessage::SetView(IdView::Single))),
                        "2" => Some(Message::IdPhoto(IdMessage::SetView(IdView::Sheet))),
                        _ => None,
                    }
                }
                _ => None,
            })
        } else {
            iced::Subscription::none()
        };
        return iced::Subscription::batch([resize, files, kbd, spinner]);
    }
    let crop_mode = app.work.tool == Tool::Crop;

    // `.with(crop_mode)` 把裁剪模式开关挂进订阅 —— 状态变化时订阅重建；
    // 闭包保持非捕获（Subscription 的 filter_map 不允许捕获外部变量）。
    let kbd = iced::keyboard::listen()
        .with(crop_mode)
        .filter_map(|(crop_mode, ev)| match ev {
            KbdEvent::KeyPressed { key, modifiers, .. } => {
                let ctrl = modifiers.control() || modifiers.logo();
                let _shift = modifiers.shift();
                let _alt = modifiers.alt();

                // 裁剪模式：Enter / Esc 有专门语义，先于通用快捷键
                match (&key, crop_mode) {
                    (Key::Named(Named::Enter), true) => {
                        return Some(Message::Work(WorkMessage::CropConfirm))
                    }
                    (Key::Named(Named::Escape), true) => {
                        return Some(Message::Work(WorkMessage::CropCancel))
                    }
                    _ => {}
                }

                let ch = match &key {
                    Key::Character(c) => Some(c.to_lowercase()),
                    Key::Named(Named::Space) => Some(" ".into()),
                    _ => None,
                };
                let key_char = ch?;

                // 组合键优先
                if ctrl {
                    return match key_char.as_str() {
                        "z" => Some(Message::Work(WorkMessage::Undo)),
                        "s" => Some(Message::Work(WorkMessage::Export)),
                        "o" => Some(Message::Work(WorkMessage::OpenImage)),
                        "b" => Some(Message::ToggleNav),
                        "r" => Some(Message::Work(WorkMessage::ResetMask)),
                        "=" => Some(Message::Work(WorkMessage::ZoomIn)),
                        "-" => Some(Message::Work(WorkMessage::ZoomOut)),
                        "]" => Some(Message::Work(WorkMessage::RotateCW)),
                        "[" => Some(Message::Work(WorkMessage::RotateCCW)),
                        _ => None,
                    };
                }

                // 单键：工具切换 / 视图 / 画笔
                match key_char.as_str() {
                    "q" => Some(Message::Work(WorkMessage::DoMatte)),
                    "b" => Some(Message::Work(WorkMessage::SetTool(Tool::Keep))),
                    "e" => Some(Message::Work(WorkMessage::SetTool(Tool::Erase))),
                    "i" => Some(Message::Work(WorkMessage::SetTool(Tool::Picker))),
                    "m" => Some(Message::Work(WorkMessage::SetTool(Tool::Pan))),
                    "c" => Some(Message::Work(WorkMessage::SetTool(Tool::Crop))),
                    "0" => Some(Message::Work(WorkMessage::ZoomFit)),
                    "a" => Some(Message::Work(WorkMessage::SetViewMode(ViewMode::Original))),
                    "s" => Some(Message::Work(WorkMessage::SetViewMode(ViewMode::Compare))),
                    "d" => Some(Message::Work(WorkMessage::SetViewMode(ViewMode::Result))),
                    "]" => Some(Message::Work(WorkMessage::SetBrushSizeStep(3))),
                    "[" => Some(Message::Work(WorkMessage::SetBrushSizeStep(-3))),
                    _ => None,
                }
            }
            _ => None,
        });

    iced::Subscription::batch([resize, files, kbd, spinner])
}

/* ═════════════════ view：外壳 + 分发 ═════════════════ */

pub fn view(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{column, row, stack};

    let nav = ui::shell::sidenav(app);
    let content: iced::Element<Message> = match app.page {
        Page::Home => ui::page_home::view(app),
        Page::Work => ui::page_work::view(app),
        Page::IdPhoto => ui::page_idphoto::view(app),
        Page::Settings => ui::page_settings::view(app),
        Page::About => ui::page_about::view(app),
    };

    let statusbar = if app.page == Page::Home {
        // 首页是独立落地页，不显示状态栏（与原项目一致）
        None
    } else {
        Some(ui::shell::statusbar(app))
    };

    // 提示条：底部居中、栈式向上排列（对应原项目 `.toasts` 的
    // `position:fixed; bottom:48px; left:50%`）。不可点击，鼠标事件穿透到画布。
    let toasts: iced::Element<Message> = if app.toasts.is_empty() {
        iced::widget::Space::new().width(0).height(0).into()
    } else {
        iced::widget::container(app.toasts.iter().fold(column![].spacing(8), |c, t| {
            c.push(
                iced::widget::container(iced::widget::text(&t.text).size(theme::FS_SM))
                    .padding(theme::pad(10.0, 17.0, 10.0, 17.0))
                    .style(if t.err {
                        theme::toast_err
                    } else {
                        theme::toast_ink
                    }),
            )
        }))
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::End)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .padding(theme::pad(0.0, 0.0, 48.0, 0.0))
        .into()
    };

    // 内容区与提示条叠放（提示条覆盖在内容之上、鼠标事件穿透）；
    // 整体作为 body 的右半部分，与左侧导航栏之间留 `--body-gap`(10px) 空隙。
    let mut content_area = stack![content, toasts]
        .width(iced::Length::Fill)
        .height(iced::Length::Fill);
    // 消息中心浮层：右下角弹出（更新对话框之下）
    if app.msg_center_open {
        content_area = content_area.push(msg_center_panel(app));
    }
    // 更新对话框：最高层模态（盖在提示条之上）
    if app.updater.dialog_open {
        content_area = content_area.push(ui::dialog_update::modal(&app.updater));
    }
    // 打赏弹窗：最高层（窗口控制仍可点，标题栏在 stack 之外）
    if app.sponsor_open {
        content_area = content_area.push(ui::dialog_sponsor::modal(app));
    }
    let body = row![nav, content_area].spacing(theme::BODY_GAP);

    // 外壳 `.body`：四周 10px 留白（padding），面板之间 10px 间距（gap）；
    // 状态栏作为底部浮卡，左 / 右 / 下 10px 外边距由这里统一给出。
    let root = match statusbar {
        Some(sb) => column![body, sb].spacing(theme::BODY_GAP),
        None => column![body],
    };

    // 自绘标题栏贴窗口顶边（全宽），其余区域保持原 10px 留白外壳
    let titlebar = ui::shell::titlebar(app);
    let main_area = iced::widget::container(root)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .padding(theme::BODY_PAD)
        .style(theme::app_bg);

    iced::widget::column![titlebar, main_area]
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}

/// 消息中心浮层（右下角弹出：标题 + 全部已读 + 消息列表 + 空态）
pub fn msg_center_panel(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{
        button, column, container, mouse_area, row, scrollable, stack, text, Space,
    };

    let i18n_key = |k: &'static str| crate::core::i18n::tr(k);
    let mut list = column![].spacing(8);
    if app.msgs.is_empty() {
        list = list.push(
            container(
                text(i18n_key("msg.empty"))
                    .size(theme::FS_SM)
                    .color(theme::TXT3()),
            )
            .padding(theme::pad(24.0, 12.0, 24.0, 12.0))
            .width(iced::Length::Fill)
            .center_x(iced::Length::Fill),
        );
    }
    for m in app.msgs.iter().take(30) {
        let time = crate::core::i18n::fmt_datetime(m.ts_ms);
        let dot_color = if m.read { theme::TXT3() } else { theme::ACC() };
        list = list.push(
            container(
                column![
                    row![
                        text(if m.read {
                            i18n_key("msg.unread")
                        } else {
                            "NEW"
                        })
                        .size(theme::FS_XS)
                        .color(dot_color),
                        text(time.clone()).size(theme::FS_XS).color(theme::TXT3()),
                        Space::new().width(iced::Length::Fill),
                        text(if m.err { "!" } else { "·" })
                            .size(theme::FS_XS)
                            .color(if m.err {
                                theme::DANGER()
                            } else {
                                theme::TXT3()
                            }),
                    ]
                    .spacing(8),
                    text(&m.title).size(theme::FS_SM).color(theme::TXT()),
                ]
                .spacing(4),
            )
            .padding(theme::pad(10.0, 12.0, 10.0, 12.0))
            .width(iced::Length::Fill)
            .style(theme::panel),
        );
    }

    let header = row![
        text(i18n_key("msg.center"))
            .size(theme::FS_H)
            .color(theme::TXT())
            .font(theme::bold_font()),
        Space::new().width(iced::Length::Fill),
        button(
            text(i18n_key("msg.mark_all_read"))
                .size(theme::FS_XS)
                .color(theme::TXT2())
        )
        .padding(theme::pad(4.0, 10.0, 4.0, 10.0))
        .style(theme::btn_ghost)
        .on_press(Message::MarkAllMsgsRead),
    ]
    .align_y(iced::Alignment::Center);

    // 浮层卡片：停靠尺寸（小面板而非整屏模态），随窗口高度钳制
    let panel_h = (app.win_h * 0.52).clamp(320.0, 520.0);

    // 透明关闭层：点击浮层外的任意区域关闭——不绘制背景，页面保持可见可读
    let catcher = mouse_area(
        container(
            Space::new()
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
        )
        .width(iced::Length::Fill)
        .height(iced::Length::Fill),
    )
    .on_press(Message::ToggleMsgCenter);

    // 卡片样式：与现有界面一致（PANEL 底 + hairline + --r-panel + 最高层阴影）
    let card = container(
        column![
            header,
            scrollable(container(list).width(iced::Length::Fill)).height(iced::Length::Fill)
        ]
        .spacing(12),
    )
    .width(iced::Length::Fixed(360.0))
    .height(iced::Length::Fixed(panel_h))
    .padding(theme::pad_all(theme::CARD_PAD))
    .style(|_t| iced::widget::container::Style {
        text_color: Some(theme::TXT()),
        background: Some(iced::Background::Color(theme::PANEL())),
        border: iced::Border {
            color: theme::LINE(),
            width: 1.0,
            radius: theme::R_PANEL.into(),
        },
        shadow: theme::shadow_4(),
        snap: false,
    });

    // 停靠定位：贴侧栏右侧（与铃铛按钮并排）、底部对齐铃铛所在区域——
    // 避开底部居中的提示条，两个弹窗同显时互不重叠
    let docked = container(card)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .align_x(iced::Alignment::Start)
        .align_y(iced::Alignment::End)
        .padding(theme::pad(0.0, 0.0, 12.0, 0.0));

    // iced 0.14 Stack：覆盖层需自带 Fill×Fill 包裹层（见 memory 布局规则）
    container(stack![catcher, docked])
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
}

/// 尝试恢复 Win11 圆角 / 阴影（decorations: false 的 DWM 补偿）；
/// 成功一次后不再重复（EnumWindows + DwmSetWindowAttribute 开销极小）
fn fix_window_frame(app: &App) -> Task<Message> {
    if app.win_frame_fixed {
        return Task::none();
    }
    let title = app.title();
    Task::perform(
        async move { ui::shell::apply_window_rounding(&title) },
        Message::WinFrameFixed,
    )
}

pub fn window_settings() -> window::Settings {
    window::Settings {
        // 与原项目 tauri.conf.json 的窗口配置保持一致
        size: iced::Size::new(1440.0, 900.0),
        min_size: Some(iced::Size::new(1024.0, 640.0)),
        position: window::Position::Centered,
        icon: window_icon(),
        // 隐藏系统原生标题栏，改用 shell::titlebar 自绘（拖动/双击最大化/窗口控制
        // 经 TitlebarMessage 驱动；边缘缩放由 winit 对无装饰+resizable 窗口的原生
        // hit-test 支持）
        decorations: false,
        ..window::Settings::default()
    }
}

/// 窗口标题栏 / 任务栏图标。
///
/// 优先从可执行文件所在目录加载 `icons/icon.png`，
/// 加载失败时回退到内嵌的 `icon-badge.png`。
/// 图标会被缩放到 256x256（Windows 标准最大图标尺寸）。
fn window_icon() -> Option<window::Icon> {
    // 调试日志写入文件（release 版无控制台）
    fn log(msg: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("icon_debug.log")
        {
            use std::io::Write;
            let _ = writeln!(f, "{}", msg);
        }
    }

    log("[icon] window_icon() called");

    // 优先尝试从 exe 所在目录加载高清图标
    let icon_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("icons/icon.png")));

    let img = if let Some(ref path) = icon_path {
        log(&format!("[icon] 尝试加载: {:?}", path));
        match std::fs::read(path) {
            Ok(bytes) => {
                log(&format!("[icon] 读取成功, {} bytes", bytes.len()));
                match image::load_from_memory(&bytes) {
                    Ok(decoded) => {
                        let rgba = decoded.to_rgba8();
                        log(&format!(
                            "[icon] 解码成功: {}x{}",
                            rgba.width(),
                            rgba.height()
                        ));
                        Some(rgba)
                    }
                    Err(e) => {
                        log(&format!("[icon] 解码失败: {}", e));
                        None
                    }
                }
            }
            Err(e) => {
                log(&format!("[icon] 读取失败: {}", e));
                None
            }
        }
    } else {
        log("[icon] 无法获取 exe 路径");
        None
    };

    // 回退到内嵌的徽章图标（编译时打包进二进制）
    let img = img.or_else(|| {
        log("[icon] 使用内嵌 icon-badge.png");
        const BADGE: &[u8] = include_bytes!("../icons/icon-badge.png");
        match image::load_from_memory(BADGE) {
            Ok(decoded) => {
                let rgba = decoded.to_rgba8();
                log(&format!(
                    "[icon] 内嵌图标解码成功: {}x{}",
                    rgba.width(),
                    rgba.height()
                ));
                Some(rgba)
            }
            Err(e) => {
                log(&format!("[icon] 内嵌图标解码失败: {}", e));
                None
            }
        }
    });

    let img = match img {
        Some(img) => img,
        None => {
            log("[icon] 所有图标加载失败");
            return None;
        }
    };

    // 缩放到 256x256（Windows 标准最大图标尺寸，确保兼容性）
    let resized = image::imageops::resize(&img, 256, 256, image::imageops::Lanczos3);
    log("[icon] 缩放到 256x256");

    match window::icon::from_rgba(resized.into_raw(), 256, 256) {
        Ok(icon) => {
            log("[icon] 窗口图标创建成功");
            Some(icon)
        }
        Err(e) => {
            log(&format!("[icon] 窗口图标创建失败: {:?}", e));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：换图必须一次调用同时推进世代、通知取消、复位 busy。
    /// 旧帧 seq=7 与当前世代不再相等，MaskDone 会整包丢弃。
    #[test]
    fn invalidate_inflight_matte_advances_generation_and_cancels() {
        let mut w = WorkState {
            recompute_seq: 7,
            busy: true,
            ..Default::default()
        };
        let _ = w.matte_cancel_tx.send(7);

        w.invalidate_inflight_matte();

        assert_eq!(w.recompute_seq, 8);
        assert_eq!(*w.matte_cancel_rx.borrow(), 8);
        assert!(!w.busy);
    }

    /// 回归：每次开启新载入链都会推进世代，旧帧 seq 与当前不再相等
    #[test]
    fn begin_load_chain_invalidates_inflight_frames() {
        let mut id = IdState {
            busy: true,
            loading: true,
            ..Default::default()
        };
        let s1 = id.begin_load_chain();
        assert_eq!(s1, 1);
        assert_eq!(id.load_seq, s1);
        // 旧链已被作废，其回传不会再来复位状态 → 这里必须清干净
        assert!(!id.busy);
        assert!(!id.loading);
        let s2 = id.begin_load_chain();
        assert_eq!(s2, 2);
        assert_ne!(s1, id.load_seq);
    }

    #[test]
    fn invalidate_inflight_matte_is_repeatedly_safe() {
        let mut w = WorkState::default();
        w.invalidate_inflight_matte();
        w.invalidate_inflight_matte();
        w.invalidate_inflight_matte();
        assert_eq!(w.recompute_seq, 3);
        assert_eq!(*w.matte_cancel_rx.borrow(), 3);
        assert!(!w.busy);
    }
}
