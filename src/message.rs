//! 全局消息：按页面分组成子消息，App::update 只做分发
//!
//! Elm 架构下所有状态变更都必须经过 Message —— UI、快捷键、异步回调走同一条路。

use std::path::PathBuf;

use crate::core::idphoto::RenderMode;
use crate::core::i18n::tr;
use crate::core::image::{ExportFormat, ExportSize};
use crate::core::matting::SampleMode;
use crate::core::pyramid::PyramidBytes;
use crate::core::updater::{CheckOutcome, DownloadEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// 首页（证件照工作台落地页）
    Home,
    /// 抠图工具
    #[default]
    Work,
    IdPhoto,
    Settings,
    About,
}

impl Page {
    pub const ALL: [Page; 5] = [Page::Home, Page::Work, Page::IdPhoto, Page::Settings, Page::About];
    pub fn label(&self) -> &'static str {
        match self {
            Page::Home => "nav.home",
            Page::Work => "nav.work2",
            Page::IdPhoto => "nav.idphoto",
            Page::Settings => "nav.settings",
            Page::About => "nav.about",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    /* ── 外壳 ── */
    Navigate(Page),
    ToggleNav,
    /// 窗口尺寸变化（逻辑像素宽 / 高；首页响应式断点 + 消息中心模态比例用）
    WinResized(f32, f32),
    /// 自绘标题栏（无系统 decorations 后的窗口控制）
    Titlebar(TitlebarMessage),
    /// 启动时解析主窗口 Id（window::oldest）
    WinIdResolved(Option<iced::window::Id>),
    /// 窗口最大化状态同步（toggle / resize 后回查）
    WinMaximized(bool),
    /// Win11 圆角 / 阴影恢复（DWM corner preference）是否已生效
    WinFrameFixed(bool),
    /// 打赏弹窗
    Sponsor(SponsorMessage),
    /// 旋转 spinner 帧推进（仅载入/处理期间由时间订阅派发）
    SpinnerTick,
    /// 关于页：重新读取 content/about/*.md
    AboutReload,
    /// 会话空闲清扫：世代号匹配（期间无新推理）时释放全部模型会话
    SessionSweep(u64),
    /// 切换主题模式（浅色 / 深色 / 跟随系统）
    SetThemeMode(crate::theme::ThemeMode),
    /// 设置自定义强调色（#RRGGBB；None = 恢复默认）
    SetAccent(Option<String>),
    /// 切换界面语言
    SetLang(crate::core::i18n::Lang),
    /// 免打扰开关
    ToggleDnd,
    /// 免打扰起止时间（HH:MM）
    SetDndTimes(String, String),
    /// 消息中心开关（打开时清未读）
    ToggleMsgCenter,
    /// 全部消息标记已读
    MarkAllMsgsRead,
    /// 用系统默认浏览器打开关于页里的外链
    OpenUrl(String),
    /// 文件拖入窗口并落下（全局订阅；按当前页路由载入）
    DroppedFile(PathBuf),
    /// 文件拖拽悬停进入 / 离开窗口（首页拖放区高亮用）
    DropHover(bool),
    /// 提示条到期自动消失（由 `toast_timer` 定时发出）
    ToastExpire(u64),

    /* ── 抠图工作台 ── */
    Work(WorkMessage),

    /* ── 证件照 ── */
    IdPhoto(IdMessage),

    /* ── 设置 ── */
    Settings(SettingsMessage),

    /* ── 首页 ── */
    Home(HomeMessage),

    /* ── 更新 ── */
    Updater(UpdaterMessage),
}

/// 自绘标题栏的窗口控制（应用关闭系统 decorations 后启用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitlebarMessage {
    /// 按住标题空白区拖动窗口
    Drag,
    /// 最小化
    Minimize,
    /// 最大化 / 还原（双击标题区同效）
    ToggleMaximize,
    /// 关闭窗口
    Close,
}

/// 打赏弹窗交互
#[derive(Debug, Clone, PartialEq)]
pub enum SponsorMessage {
    /// 打开 / 关闭弹窗
    Open(bool),
    /// 选择建议金额（元）
    Amount(f32),
    /// 自定义金额输入
    CustomInput(String),
}

/* ═════════════════ 工作台 ═════════════════ */

/// 色键抠图预设（SmartMatte v2）。
///
/// 神经网络模型（MODNet / BiRefNet / 用户插件）不走这个枚举 —— 它们由
/// `core::nn::scan()` 从 models/ 目录动态发现，以 `tr("nn:<文件名>")` id 标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelKind {
    #[default]
    Smart,
}

impl ModelKind {
    pub const ALL: &'static [ModelKind] = &[ModelKind::Smart];
    pub fn id(&self) -> &'static str {
        match self {
            ModelKind::Smart => "local-smart",
        }
    }
    pub fn from_id(_s: &str) -> Self {
        ModelKind::Smart
    }
    pub fn label(&self) -> &'static str {
        match self {
            ModelKind::Smart => tr("【图形-文言】_自研内置版"),
        }
    }
}

impl std::fmt::Display for ModelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 自定义取样的作用范围
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMode {
    #[default]
    Connected,
    All,
}

impl KeyMode {
    pub const ALL: &'static [KeyMode] = &[KeyMode::Connected, KeyMode::All];
    pub fn label(&self) -> &'static str {
        match self {
            KeyMode::Connected => tr("边缘连通"),
            KeyMode::All => tr("全图色键"),
        }
    }
}

impl std::fmt::Display for KeyMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// 智能抠图（自动）
    #[default]
    Auto,
    /// 保留画笔 · 修回主体
    Keep,
    /// 擦除画笔 · 抹掉残留
    Erase,
    /// 裁剪
    Crop,
    /// 吸管 · 取样背景色
    Picker,
    /// 移动画布
    Pan,
}

impl Tool {
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Original,
    Compare,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BgPreview {
    #[default]
    Checker,
    White,
    Black,
}

impl BgPreview {
    pub fn label(&self) -> &'static str {
        match self {
            BgPreview::Checker => tr("透明底"),
            BgPreview::White => tr("白底"),
            BgPreview::Black => tr("黑底"),
        }
    }
}

/// 蒙版参数的可调项（滑块 / 开关统一走这里）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Thresh,
    Edge,
    Feather,
    Morph,
    Defringe,
    FillNeck,
}

#[derive(Debug, Clone)]
pub enum WorkMessage {
    /// 防抖到期，真正触发蒙版重算（滑块拖动期间只更新状态值）。
    /// 携带排定时的防抖序列号：拖动过程中排出的过期定时器一律丢弃，
    /// 避免它们各自启动一次后台重算、白占线程池并推进取消令牌。
    RecomputeNow(u64),
    /// 打开图片（弹原生对话框）
    OpenImage,
    /// 打开批量图片
    OpenBatch,
    /// 文件选择结果
    FilePicked(Option<PathBuf>),
    FilesPicked(Vec<PathBuf>),
    /// 直接按路径打开（命令行参数 / 未来接系统「打开方式」）
    OpenPath(PathBuf),
    /// 解码完成
    ImageLoaded(Result<Box<LoadedPayload>, String>),
    /// 蒙版计算完成
    MaskDone(Result<Box<MaskPayload>, String>),
    /// 画笔 / 橡皮擦松手（或撤销 / 重置蒙版）后，后台线程把「蒙版 → 结果图」的合成回传。
    ///
    /// 与 `MaskDone` 走同一条后台线程链路，但只改蒙版、不跑模型推理。
    /// `seq` 用于丢弃过期帧：连续涂抹会并发提交多次合成，只有序号与当前
    /// `WorkState.refresh_seq` 一致的那一次才被采纳，避免旧结果覆盖新参数
    /// （设计同 `IdMessage::RenderDone` 的防乱序载荷）。
    /// 载荷第三个元素是后台生成的两级显示字节（preview/full），
    /// UI 线程只转成 Handle，不做 resize / 整图拷贝。
    ResultCommitted(Result<(u64, Box<image::RgbaImage>, Box<PyramidBytes>), String>),
    /// 换工具
    SetTool(Tool),
    /// 改参数（滑块 / 开关）
    SetParam(ParamKind, i32),
    ToggleFill(bool),
    ToggleShadow(bool),
    ToggleAuto(bool),
    SetSampleMode(SampleMode),
    SetCustomHex(String),
    ToggleKeyAll(bool),
    /// 换模型：参数为模型 id（"local-*" 色键预设 / "nn:<文件名>" 神经网络模型）
    SetModel(String),
    /// 执行抠图（用户主动点击「智能抠图」按钮时触发）
    DoMatte,
    /// 重新扫描 models/ 目录（插件模型热发现）
    RescanModels,
    /// 笔刷属性
    SetBrushSize(i32),
    /// 滚轮 / [ ] 快捷键的笔刷大小增量（参考项目 wheel 与 [ ] 键）
    SetBrushSizeStep(i32),
    SetBrushHardness(i32),
    SetBrushOpacity(i32),
    SetBrushFlow(i32),
    /// 点击面板外部收起「笔刷属性」下拉面板
    CloseBrushProps,
    /// 视图
    SetViewMode(ViewMode),
    SetBgPreview(BgPreview),
    ZoomIn,
    ZoomOut,
    /// 滚轮缩放（参考 bind.js wheel 分支：×1.1 放大 / ×0.9 缩小；
    /// 与底部按钮的 1.25 步进区分开）
    ZoomStep(f32),
    ZoomFit,
    /// 旋转
    RotateCW,
    RotateCCW,
    /// 输出
    SetOutColor(String),
    SetFormat(ExportFormat),
    SetExportSize(ExportSize),
    Export,
    /// 后台线程完成 fit_export + 编码后回传（name, 编码字节）。
    /// `seq` 用于丢弃过期结果；`Box` 避免大 Vec 撑大整个 Message 枚举。
    ExportPrepared(u64, Result<Box<(String, Vec<u8>)>, String>),
    Exported(Result<String, String>),
    /// 破坏性编辑
    ResetMask,
    Undo,
    RotateNinety(bool),
    SetRotateDeg(f32),
    /// 把「旋转校正」滑块的角度真正落到像素上
    ApplyRotate,
    SetCropRatio(CropRatio),
    CropCancel,
    CropConfirm,
    /// 画布交互：屏幕坐标 + 对应的图像坐标（由 canvas Program 换算后带出）
    StagePress { screen: iced::Point, img: (f32, f32), bounds: (f32, f32) },
    StageMove { screen: iced::Point, img: (f32, f32), bounds: (f32, f32) },
    StageRelease,
    StageLeave,
    /// 批量
    BatchAdd,
    /// 选输出目录后开始批量处理
    BatchRun,
    BatchFolderPicked(Option<PathBuf>),
    BatchEvent(WorkBatchEvent),
    BatchRemove(usize),
    BatchClear,
    /// 把失败项重置为待处理并重新选择输出目录
    BatchRetryFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CropRatio {
    #[default]
    Free,
    Square,
    R4x3,
    R3x2,
    R16x9,
    R9x16,
    Orig,
}

impl CropRatio {
    pub fn label(&self) -> &'static str {
        match self {
            CropRatio::Free => tr("自由"),
            CropRatio::Square => "1:1",
            CropRatio::R4x3 => "4:3",
            CropRatio::R3x2 => "3:2",
            CropRatio::R16x9 => "16:9",
            CropRatio::R9x16 => "9:16",
            CropRatio::Orig => tr("原始"),
        }
    }
    /// 宽高比（w/h）；None = 自由
    pub fn ratio(&self, img: (f32, f32)) -> Option<f32> {
        match self {
            CropRatio::Free => None,
            CropRatio::Square => Some(1.0),
            CropRatio::R4x3 => Some(4.0 / 3.0),
            CropRatio::R3x2 => Some(3.0 / 2.0),
            CropRatio::R16x9 => Some(16.0 / 9.0),
            CropRatio::R9x16 => Some(9.0 / 16.0),
            CropRatio::Orig => Some(img.0 / img.1.max(1.0)),
        }
    }
}

/// 异步装载完成后的图片载荷
#[derive(Debug, Clone)]
pub struct LoadedPayload {
    pub rgba: image::RgbaImage,
    pub path: Option<PathBuf>,
    pub stem: String,
    /// 工作台载入时由后台线程生成的两级显示字节；证件照页链路为 `None`
    pub display: Option<Box<PyramidBytes>>,
}

#[derive(Debug, Clone)]
pub struct MaskPayload {
    pub mask: Vec<u8>,
    pub base: Vec<u8>,
    pub result: image::RgbaImage,
    /// 两级显示字节，在后台线程连同抠图一起生成
    pub display: Box<PyramidBytes>,
    pub fg_count: usize,
    pub holes: usize,
    pub elapsed_ms: f32,
    pub bg: [u8; 3],
}

/* ═════════════════ 证件照 ═════════════════ */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdView {
    #[default]
    Single,
    Sheet,
}

#[derive(Debug, Clone)]
pub enum IdMessage {
    OpenPhoto,
    OpenBatch,
    FilePicked(Option<PathBuf>),
    /// 批量导入结果（多选；参考项目 btnIdToolbarBatch）
    FilesPicked(Vec<PathBuf>),
    PhotoLoaded(Result<Box<LoadedPayload>, String>),
    /// 内存图片直接进入抠图链路（保留供未来使用）
    #[allow(dead_code)]
    MemoryPhoto(Box<LoadedPayload>),
    MatteDone(Result<Box<LoadedPayload>, String>),
    SetSpec(usize),
    SetBg(usize),
    /// 切换渲染模式（纯色 / 渐变 / 透明底 / 原背景）
    SetRender(RenderMode),
    SetZoom(i32),
    SetOffY(i32),
    SetErode(i32),
    SetPaper(usize),
    /// 切换抠图模型（参数为模型 id，如 "nn:modnet.onnx"）
    SetModel(String),
    ToggleCutline(bool),
    ToggleRealsize(bool),
    SetView(IdView),
    ExportSingle,
    ExportSheet,
    Saved(Result<String, String>),
    ChangePhoto,
    /// 载入批量队列中的某一项（参考项目 batch 面板「载入」按钮）
    QueueLoad(usize),
    /// 从批量队列移除某一项（参考项目「移除」按钮）
    QueueRemove(usize),
    /// 批处理按钮（参考项目 btnIdBatchProc：无队列提示先导入，有队列报数量）
    BatchProc,
    /// 后台渲染完成（携带腐蚀缓存）
    RenderDoneWithCache(Box<RenderDonePayload>),
    /// 防抖延迟到期，真正触发后台渲染（滑块拖动期间只更新状态值，到期后才 render）
    RenderNow,
    /// 导出编码完成（文件名 + 字节流），等待保存对话框落盘。
    /// 合成 / 排版 / PNG·JPG 编码全部在后台线程完成，UI 线程零重活。
    SaveReady(Box<Result<ExportReady, String>>),
    /// 设置单张导出格式（PNG / JPG / WEBP）
    SetExportFormat(ExportFormat),
    /// 批量导出：选目标文件夹
    BatchExport,
    /// 文件夹选择结果，选中后启动批量导出流水线
    BatchExportFolder(Option<PathBuf>),
    /// 批量导出进度事件（Task::stream 保序回传）
    BatchEvent(BatchExportEvent),
    /// 旋转校正（度，-15..=15；正值顺时针）
    SetRotate(i32),
    /// 启用/停用自定义底色
    ToggleCustomBg(bool),
    /// 自定义底色 HEX 输入（#RRGGBB）
    SetCustomBg(String),
    /// 输出 DPI（只影响毫米制规格；像素制规格固定不变）
    SetDpi(u32),
    /// 批量预取解码完成：单槽缓存实现「解码 N+1 ∥ 抠图 N」
    PrefetchLoaded(usize, Result<Box<LoadedPayload>, String>),
}

/// 工作台批量事件：逐张 更新队列徽标，结束汇总
#[derive(Debug, Clone)]
pub enum WorkBatchEvent {
    Start { idx: usize },
    /// 输出文件路径：当前仅用于写盘结果核对，UI 未消费（保留供「打开所在文件夹」扩展）
    Done {
        idx: usize,
        #[allow(dead_code)]
        path: String,
    },
    Fail { idx: usize, err: String },
    Finished { ok: usize, fail: usize },
}

/// 批量导出进度事件：逐张 更新队列徽标，结束汇总
#[derive(Debug, Clone)]
pub enum BatchExportEvent {
    Start { idx: usize },
    Done { idx: usize },
    Fail { idx: usize, err: String },
    Finished { ok: usize, fail: usize, folder: String },
}

/// 导出编码产物（后台线程产出，交给保存对话框写盘）
#[derive(Debug, Clone)]
pub struct ExportReady {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// 后台渲染回传载荷：携带「序列号 + 视图 + 合成图 + meta 文案」。
///
/// `seq` 用来丢弃过期帧——连续拖动滑块会并发发起多帧合成，只有
/// 序列号与当前最新一致的那一帧才被采纳，避免旧结果覆盖新参数。
#[derive(Debug, Clone)]
pub struct RenderDonePayload {
    pub seq: u64,
    pub view: IdView,
    pub img: Box<image::RgbaImage>,
    pub meta: String,
    /// 缓存腐蚀后的前景图（预览尺寸；回传到主线程存入 IdState，下次渲染复用）
    pub fg_eroded: Option<std::sync::Arc<image::RgbaImage>>,
    /// 腐蚀缓存键，与 fg_eroded 配对
    pub erode_key: Option<(i32, RenderMode, u32, u32)>,
    /// 首次渲染时构建的预览工作副本（之后一直复用，None = 已有）
    pub preview: Option<std::sync::Arc<image::RgbaImage>>,
}

/* ═════════════════ 设置 ═════════════════ */

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    SetPanel(usize),
    SetModel(ModelKind),
    ToggleAuto(bool),
    SetFormat(ExportFormat),
    SetExportSize(ExportSize),
    SetApiUrl(String),
    SaveApiUrl,
    PickPluginDir,
    PluginDirPicked(Option<PathBuf>),
    ResetParams,
    ClearRecent,
    /// 释放内存缓存：模型会话 + 撤销历史 + 色键场 + 预览副本/预取槽
    ClearCache,
    /// 重新扫描 models/ 目录下的 ONNX 模型
    RescanModels,
    /// 刷新系统 DPI 检测
    RefreshDpi,
}

/* ═════════════════ 首页 ═════════════════ */

#[derive(Debug, Clone)]
pub enum HomeMessage {
    /// 应用某个快速预设（规格索引）
    ApplyPreset(usize),
    /// 「开始」卡：跳转抠图工作台并弹原生文件选择器（点击上传）
    OpenPhoto,
    LoadSample,
    ResetParams,
    /// 新手引导：**唯一开启入口**，仅由首页「新手引导」按钮发出
    StartGuide,
    /// 新手引导：上一步
    GuidePrev,
    /// 新手引导：下一步（末步即完成，写入 guide_done 历史标记）
    GuideNext,
    /// 新手引导：跳过（仅关闭，不写 guide_done；再次开启只能点按钮）
    GuideSkip,
    /// 新手引导：不再显示（关闭并持久化历史标记；已无自动弹出逻辑）
    GuideNever,
    /// 新手引导：跳到指定步（仅引导进行中生效，不会从关闭态开启）
    GuideGoto(usize),
}

/* ═════════════════ 更新 ═════════════════ */

/// 自更新全链路消息：检查 → 提醒 → 下载 → 安装，以及用户交互（忽略 / 稍后）
#[derive(Debug, Clone)]
pub enum UpdaterMessage {
    /// 手动检查更新（关于页按钮）
    Check,
    /// 检查结果回传。`manual=true` 表示用户主动触发（失败要 toast 报错；
    /// 后台自动检查失败静默，不打扰）
    Checked { manual: bool, result: Result<CheckOutcome, String> },
    /// 「一键更新」：开始下载安装包
    StartInstall,
    /// 「忽略此版本」：记录版本号，自动提醒不再弹窗
    SkipVersion,
    /// 打开更新对话框（关于页状态行 / 通知入口）
    ShowDialog,
    /// 关闭对话框（稍后再说）
    Dismiss,
    /// 下载进度 / 完成 / 失败（由下载任务流逐条回传）
    DownloadEvent(DownloadEvent),
    /// 取消下载
    CancelDownload,
    /// 下载完成后：启动安装器并退出主程序
    QuitAndInstall,
    /// 浏览器打开下载页（清单不可用 / 用户想手动下载时的兜底）
    OpenDownloadPage,
    /// 切换「启动时自动检查更新」
    ToggleAutoCheck(bool),
    /// 清除「忽略此版本」标记（关于页恢复提醒）
    ClearSkipped,
}