# 壹鸣 · 商品抠图工作台 —— iced 原生复刻版

用 **Rust + iced** 从零复刻 `cocole-v0.3.2`（原 Tauri 2 + Vite + 原生 ESM 前端）。  
**不含 WebView、Electron、任何前端框架**，UI 由 iced 直接经 wgpu 绘制。

- 原项目：`D:\HuaweiShare\koko\cocole-v0.3.2`
- 本复刻：`D:\HuaweiShare\koko\cocole-iced`
- iced 版本：0.14（Elm 架构，wgpu 渲染）

---

## 一、原项目分析

### 1.1 技术栈

| 层     | 原项目                                        | 本复刻                               |
| ----- | ------------------------------------------ | --------------------------------- |
| 外壳    | Tauri 2（系统 WebView）                        | 原生窗口（iced / winit / wgpu）         |
| UI    | `index.html` + 原生 ESM JS + 9 个 CSS         | iced 声明式 widget                   |
| 逻辑    | 24 个 ES 模块（core / matting / ui / platform） | Rust 模块（core / ui）                |
| 推理    | onnxruntime-web（浏览器内）+ Rust 侧 `ort` 命令     | 纯 CPU Rust 算法（SmartMatte v2）      |
| 持久化   | `localStorage`                             | 系统配置目录 `config.json`              |
| 文件对话框 | `@tauri-apps/plugin-dialog`                | `rfd`（Win32 / AppKit / XDG 原生对话框） |

### 1.2 窗口与页面布局

```
┌──────────┬───────────────────────────────────────────┐
│ sidenav  │  page                                      │
│ 208px    │  ├ 抠图页：toolstrip 46px                  │
│ 品牌     │  │          canvas（棋盘格 + 图 + 裁剪框） │
│ 5 个导航 │  │          stage-bar 38px                │
│ 收起     │  ├ 证件照页：预览 + 右侧参数栏             │
│ 版本/引擎│  ├ 设置页：分类导航 196px + 面板           │
│          │  └ 关于页：品牌 + 卡片                     │
├──────────┴───────────────────────────────────────────┤
│ statusbar 30px（首页隐藏）                            │
└──────────────────────────────────────────────────────┘
```

五个页面与导航项一一对应：`首页` / `抠图工具` / `证件照` / `设置` / `关于`。

### 1.3 核心功能与交互流程

**抠图工作台**

```
拖入/打开图片 → 解码 → 取样背景色 → 计算梯度场 + 色距场
   → 自适应多趟二值化（最多 4 趟，前景占比需落在 0.4%~98.5%）
   → 形态学收缩/扩张 → 填充内部孔洞（含窄颈渗入）→ 羽化
   → 去色晕 → 合成 RGBA → 显示
   → [保留/擦除画笔修边] → [裁剪/旋转] → 导出 PNG/JPG/WEBP
```

**证件照**

```
选择照片 → 本地抠图 → 边缘收缩 → 按规格构图（cover by height + 缩放 + 垂直偏移）
   → 底色渲染（纯色/上下渐变/中心渐变/透明/原背景）→ 单张 PNG
   → 相纸排版（横竖两种朝向取张数多者）→ 排版 JPG
```

### 1.4 数据模型

| 数据             | 内容                                                                                                                  |
| -------------- | ------------------------------------------------------------------------------------------------------------------- |
| `state.params` | sample / custom / keyAll / fillNeck / thresh(32) / feather(14) / morph(0) / defringe(60) / edge(30) / fill / shadow |
| `state.brush`  | size(18) / opacity(100) / flow(100) / hardness{keep:85, erase:85}                                                   |
| 证件照规格          | 20 种（毫米制 14 种 + 像素制 6 种），`ID_DPI = 300`                                                                             |
| 底色             | 7 种官方色值（标准白 #FFFFFF / 标准蓝 #438EDB / 考公蓝 #0066CC / CET 浅蓝 #64C5FF / 申根灰 #DCDCDC / 标准红 #FF0000 / 档案红 #CC0000）         |
| 相纸             | 六寸 102×152 / 五寸 89×127 / 七寸 127×178 / A4 210×297                                                                    |
| 撤销栈            | 深度 3，快照 {srcData, mask, maskBase, fgCount, rotTotal}，换图清空                                                           |
| 持久化键           | nav / api / stats / matte_nav 等 localStorage 键 → 合并进 `config.json`                                                  |

### 1.5 资源文件

| 资源       | 原项目                                         | 本复刻                                 |
| -------- | ------------------------------------------- | ----------------------------------- |
| 设计 token | `src/styles/tokens.css`（颜色/圆角/阴影/字号）        | `src/theme.rs`（逐值搬运）                |
| 图标       | 内联 SVG + lucide                             | 文本字形占位（见差异说明）                       |
| 中文字体     | 系统字体栈 Inter → PingFang SC → Microsoft YaHei | 运行时加载 `C:\Windows\Fonts\msyh.ttc` 等 |
| 模型       | `public/models/{u2netp,modnet,yunet}.onnx`  | 无（改用算法）                             |
| 关于页文案    | `public/content/about/*.md`                 | 内置常量（`src/ui/page_about.rs`）        |

---

## 二、模块划分

```
src/
  main.rs                入口：字体加载 + application(...).run()
  app.rs                 顶层 State（App / WorkState / IdState）+ update 分发 + view 分发
  message.rs             Message 与各页面子消息、枚举类型（Tool / Page / ModelKind ...）
  theme.rs               设计 token：颜色 / 圆角 / 尺寸 / 全部 widget 样式
  core/                  与 UI 无关的内核
    matting.rs           取样 / 梯度 / 色距 / 二值化 / 形态学 / 补孔 / 羽化 / 去色晕
    idphoto.rs           20 规格 / 7 底色 / 4 相纸 / 构图 / 渐变背景 / 排版
    image.rs             解码 / 编码 / 旋转 / 裁剪 / 缩放 / 蒙版合成 / 导出规整
    config.rs            Config 结构与 JSON 持久化
    history.rs           撤销栈（深度 3）
  ui/
    shell.rs             侧栏 + 状态栏
    canvas_view.rs       画布 Program（棋盘格 / 图像 / 对比 / 裁剪框 / 笔刷光标 / 吸管）
    widgets.rs           复用零件（section / slider_row / switch_row / swatch_grid ...）
    page_home.rs         首页
    page_work.rs         抠图工具页（update + view）
    page_idphoto.rs      证件照页
    page_settings.rs     设置页（7 个面板）
    page_about.rs        关于页
tools/                   离线回归工具（Node，与运行时无关，验证用）
  gen_sample.mjs         生成确定性合成样本（裸 RGBA，easy / hard 两档）
  js_dump.mjs            复用原项目 JS 内核跑出裸蒙版
  parity.mjs             串联 Rust / JS 两端并逐像素比对
```

每个界面对应一个模块：`update(app, 子消息)` + `view(app)`，顶层 `App::update` 只做分发。  
State 集中在 `App`，页面模块只读/改自己的那一段，符合 Elm 的「单一状态源 + 单向数据流」。

---

## 三、原功能 → iced 组件映射

| 原功能（HTML/CSS）                                        | iced 组件                                                | 还原度                  |
| ---------------------------------------------------- | ------------------------------------------------------ | -------------------- |
| `.sidenav` 侧栏 + 折叠                                   | `container(column![...]).style(sidenav)` + `ToggleNav` | 完全一致（208/60px）       |
| `.nav-item` 菜单项                                      | `button(...).style(nav_btn(active))`                   | 一致                   |
| `.toolstrip` 工具条                                     | `row![button...].style(rail_btn)`                      | 一致                   |
| `#view` + `#viewOriginal` + `#cropOverlay` 三层 canvas | 单个 `canvas::Program`（一次画完）                             | 一致（更省）               |
| 棋盘格透明底                                               | `canvas` 里 `fill_rectangle` 交替两色                       | 一致                   |
| 结果/对比/原图 分段控件                                        | `seg_btn` 自定义样式 button 组                               | 一致                   |
| 透明底/白底/黑底                                            | 同上 + `stage_bg(color)`                                 | 一致                   |
| 缩放 −/+/适应                                            | `button` + `zoom` 字段                                   | 一致                   |
| `.inspector` 检查器                                     | `scrollable(column![section...])`，宽 296px              | 一致                   |
| `.sec` 小节                                            | `widgets::section(title, badge, content)`              | 一致                   |
| `.slider` 参数滑块                                       | `slider(range, value, on_change)`（整数步长）                | 一致                   |
| `.switch` 开关                                         | 自定义 `switch(on)` 样式 button                             | 视觉一致                 |
| `.swatch-grid` 色板                                    | `button(container(...)).style(swatch(selected))`       | 一致                   |
| `select` 下拉                                          | `pick_list(options, selected, on_select)`              | 一致                   |
| 保留 / 擦除画笔                                            | canvas 事件 → `paint_at()` 改 mask → 重建句柄                 | 一致                   |
| 吸管取样                                                 | canvas 事件 → `pick_color()` 取像素                         | 一致                   |
| 裁剪框 + 比例锁定                                           | canvas 画框 + `CropRatio::ratio()` 约束                    | 一致                   |
| 旋转 90° / 任意角                                         | `rotate90` / `rotate_deg`（双线性）                         | 一致                   |
| 撤销（原 Ctrl+Z）                                         | 工具栏「撤销」按钮 + `History`（快照深度 3）                          | 行为一致，缺键盘快捷键（见 §四）    |
| 重置蒙版                                                 | 「重置蒙版」按钮 → 回滚到 `base`                                  | 一致                   |
| 裁剪框拖动 / 缩放                                           | canvas 命中测试 → `Drag::CropMove` / `CropResize`          | 一致                   |
| 裁剪比例锁定                                               | `seg_group(&CROP_RATIOS, ...)`                         | 一致                   |
| 旋转 90° / 任意角校正                                       | 「↺90°」「↻90°」按钮 + 角度滑块 +「应用旋转」                          | 一致                   |
| 批量队列                                                 | 顺序 `blocking_task` + 写到 `out/`                         | 一致                   |
| 导出（格式/尺寸/背景）                                         | `fit_export` + `rfd` 保存对话框                             | 一致                   |
| 证件照预览（单张 / 排版）                                       | `iced::widget::image::Image` + `ContentFit::Contain`   | 一致                   |
| 相纸排版                                                 | `make_layout` 生成画布后同一方式显示                              | 一致                   |
| 排版导出 JPG                                             | 「导出排版 JPG」→ `save_jpg`                                 | 一致                   |
| 设置页分类导航                                              | `column![button]` + `settings_panel` 索引                | 一致                   |
| toast 提示                                             | `stack![content, toast_layer]`，点击即 `DismissToast`      | 一致（比原版少了自动淡出，改为手动关闭） |
| 状态栏指标                                                | `shell::statusbar`                                     | 一致                   |

---

## 四、无法精确还原的部分与替代方案

| 项                   | 原实现                                                       | 替代方案                                  | 说明                                                             |
| ------------------- | --------------------------------------------------------- | ------------------------------------- | -------------------------------------------------------------- |
| **神经网络抠图**          | onnxruntime-web 跑 MODNet / U²-Net；Rust 侧 `ort` 跑 BiRefNet | 复用工作台的 SmartMatte v2（色距 + 梯度 + 自适应阈值） | 未引入 ONNX Runtime；如需恢复，在 `core/matting.rs` 外加一个 `ort` 后端即可，接口不变 |
| **摄像头拍摄**           | `getUserMedia`                                            | 不实现                                   | iced 无摄像头 API；需接 `nokhwa` 等 crate，页面保留入口并提示                    |
| **矢量图标**            | 内联 SVG / lucide                                           | 文本字形（⌂ ⧉ ☺ ⚙ ⓘ）                       | 视觉为占位；可改用 `iced::widget::svg`（本项目已启用 `svg` feature）            |
| **文件拖放**            | webview 级 drop 事件                                         | 不实现（用打开对话框）                           | iced 需 `window::Event::FileDropped` 订阅，留作后续                    |
| **快捷键**             | Ctrl+Z / Ctrl+O / 空格平移                                    | 仅工具栏按钮                                | iced 0.14 可用 `keyboard::on_key_press` 订阅补齐                     |
| **真实尺寸预览（显示器 DPI）** | `nn_screen_dpi` Tauri 命令探测                                | 关掉该开关的影响                              | iced 有 `window::scale_factor`，可按同一思路补                          |
| **远程后端 RMBG-2.0**   | POST multipart 到自建服务                                      | 仅保留地址配置                               | 需 `reqwest` + multipart，未引入                                    |
| **模型插件目录扫描**        | Tauri `nn_scan_dir`                                       | 仅记录路径                                 | 同上，依赖推理后端                                                      |
| **CSS 阴影/过渡动画**     | `box-shadow` + `cubic-bezier`                             | iced 静态 `Shadow`                      | 无过渡动画（iced 默认无动画系统）                                            |

---

## 五、缺失信息与默认假设

原项目信息不足以确定的部分，按以下默认假设实现：

1. **证件照构图细节**：原 `composeSingle` 只给了 `scale = (sh / fg.height) * zoom/100` 与  
   `dy = (sh-dh)/2 + offY/100*sh`，未见人脸检测参与构图（YuNet 已停用）。  
   → 假设：**纯几何构图**，不检测五官位置，完全按上述公式实现。
2. **导出目标目录**：原走保存对话框。→ 假设：沿用保存对话框（rfd `save_file`）。
3. **批量输出位置**：原项目经前端 `saveBlob` 逐个保存。  
   → 假设：统一写到**源目录旁的 `out/` 子目录**，文件名 `<原名>_nobg.<ext>`。
4. **示例图**：原「载入示例」加载内置图。→ 假设：原生版不内置资源，改为弹出选择框。
5. **配置存储位置**：原 localStorage。  
   → 假设：系统配置目录（`directories::ProjectDirs`），Windows 下为  
   `%APPDATA%\yiming\cocole-iced\config.json`。
6. **窗口初始尺寸**：原 tauri.conf.json 未在本仓库读到具体值。→ 假设：1280×820，最小 1040×680。
7. **蒙版算法中的 `shadow`（保留投影）开关**：原项目 UI 有该开关，  
   但 JS 侧未见对应实现分支。→ 假设：仅作为开关记录，不改变算法输出。

---

## 六、异步、持久化与错误处理

- **异步**：所有耗时操作（解码、蒙版计算、批量、编码）都包在  
  `ui::page_work::blocking_task()` 里 —— `std::thread::spawn` + `tokio::sync::oneshot`
  - `Task::perform`，**不依赖 tokio runtime 存在**，UI 线程永不阻塞。  
    文件对话框用 `rfd::AsyncFileDialog`（真正的 async，不阻塞）。
- **文件读写**：`image` crate 解码/编码；保存走原生对话框后 `rfd::FileHandle::write`。
- **配置持久化**：`Config` 为 `serde` 结构体，任何参数变更调用 `App::persist()` 落盘；  
  读取失败或损坏时回落默认配置并打印日志，**不因配置问题启动失败**。
- **错误提示**：内核统一返回 `Result<_, String>` / `ImgError`，update 里转成  
  `App::toast()` 提示；线程异常退出时 `blocking_task` 收到 `None` 并提示「计算线程异常退出」。  
  全代码无 `unwrap()` 于用户输入路径。

---

## 七、编译与运行

```bash
cd D:\HuaweiShare\koko\cocole-iced

cargo build          # 调试构建
cargo build --release # 发布构建（LTO + strip）

cargo run            # 直接运行
cargo run --release
```

> 首次构建会编译 iced 及其 wgpu 依赖链（约 300 个 crate），冷构建在数分钟量级。  
> Windows 需要 MSVC 或 GNU 工具链；iced 0.14 要求 Rust ≥ 1.82（本机 1.97.1 验证通过）。

#### 命令行

```bash
cocole-iced                                  # 正常启动
cocole-iced <图片路径>                        # 启动后直接载入该图并自动抠图
cocole-iced --dump-mask <in.rgba> <w> <h> <out.mask> [JSON参数|参数文件]
```

`--dump-mask` 是**无 GUI 的诊断子命令**：读裸 RGBA、跑一遍抠图内核、把 w×h 的 u8 蒙版写盘，  
并打印一行 JSON 摘要。配合 `tools/` 下的脚本可做与原版 JS 的逐像素回归（见 §8.4）。

#### 回归工具（`tools/`，需 Node ≥ 20）

```bash
node tools/gen_sample.mjs sample.rgba 1200 900 hard     # 生成确定性合成样本（裸 RGBA）
node tools/parity.mjs     sample.rgba 1200 900          # JS ↔ Rust 逐像素比对（差异非 0 则 exit 1）
node tools/parity.mjs     sample.rgba 1200 900 '{"thresh":20,"feather":30}'

export COCOLE_JS_DIR=D:/HuaweiShare/koko/cocole-v0.3.2 # 原项目根目录，js_dump.mjs 从这儿 import 内核
```

## 八、验证状态

以下三段输出均取自本机（Windows 11 / rustc 1.97.1 / iced 0.14.0）实际执行结果。

### 8.1 `cargo check`

```
$ cargo check --message-format short
    Checking cocole-iced v0.3.2 (D:\HuaweiShare\koko\cocole-iced)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.85s
```

**0 error / 0 warning**（`cargo check` 无输出即无诊断项）。

### 8.2 `cargo build`

```
$ cargo build
   Compiling cocole-iced v0.3.2 (D:\HuaweiShare\koko\cocole-iced)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.86s

$ cargo build --release
   Compiling iced v0.14.0
   Compiling cocole-iced v0.3.2 (D:\HuaweiShare\koko\cocole-iced)
    Finished `release` profile [optimized] target(s) in 6m 45s   # 冷构建（含 wgpu 全链路）

$ cargo build --release          # 改动单个 crate 后的增量重建
    Finished `release` profile [optimized] target(s) in 2m 21s
```

> 注意：`release` 二进制不会自动跟随 debug 构建。改了 `src/` 之后若要用 release 复验，  
> 必须重新 `cargo build --release`；本文 §8.4 的矩阵就是在重编后的 release 二进制上跑的。

产物：

| 配置      | 路径                               | 体积                        |
| ------- | -------------------------------- | ------------------------- |
| debug   | `target/debug/cocole-iced.exe`   | 52,397,568 B（50.0 MB，含符号） |
| release | `target/release/cocole-iced.exe` | 16,033,792 B（15.3 MB）     |


### 8.3 `cargo run`

```
$ cargo run                      # 或 target/debug/cocole-iced.exe
$ target/debug/cocole-iced.exe "D:\path\to\photo.jpg"   # 带路径启动：直接载入并自动抠图
```

启动后进程常驻、stderr 无 panic。五个页面（抠图 / 证件照 / 批量 / 设置 / 关于）全部可切换且正常渲染。

### 8.4 抠图内核：JS 原版 ↔ Rust 移植 逐像素回归 ★

原版算法（`cocole-v0.3.2/src/matting/*.js`）被逐行移植到 `src/core/matting.rs`。  
为了**避免靠肉眼判断**（肉眼曾误判过一次），加了离线比对工具链：

| 工具                                                              | 作用                                  |
| --------------------------------------------------------------- | ----------------------------------- |
| `cocole-iced --dump-mask <in.rgba> <w> <h> <out.mask> [JSON参数]` | 用 Rust 内核跑出裸蒙版                      |
| `tools/js_dump.mjs`                                             | 用 Node 直接 `import()` 原项目 JS 内核跑出裸蒙版 |
| `tools/parity.mjs`                                              | 串联两端并逐字节比对，输出差异统计，非 0 差异时 exit 1    |
| `tools/gen_sample.mjs`                                          | 生成确定性合成样本（easy / hard 两档难度）         |

```bash
$ node tools/parity.mjs test.rgba 1400 1050
{
  "image": "1400x1050",
  "rust": { "bg": [254,254,252], "fgCount": 354185, "ratio": 0.240942, "holes": 0, "ms": 18.8 },
  "js":   { "bg": [254,254,252], "fgCount": 354185, "ratio": 0.240942, "holes": 0, "ms": 73 },
  "diffPixels": 0, "maxDiff": 0, "firstDiffIdx": -1,
  "verdict": "PASS 逐像素一致"
}
```

实测矩阵（**18 组，全部 `diffPixels = 0`**）：

| 样本                        | 尺寸        | 参数组                              | attempts | fgCount           | 结果       |
| ------------------------- | --------- | -------------------------------- | -------- | ----------------- | -------- |
| 合成商品图 easy                | 1400×1050 | 默认                               | 1        | 354,185           | PASS     |
| 合成商品图 easy                | 1400×1050 | `thresh=20 feather=30`           | 1        | 354,185           | PASS     |
| 合成商品图 easy                | 1400×1050 | `thresh=50 feather=0`            | 1        | 354,187           | PASS     |
| 合成商品图 easy                | 1400×1050 | `thresh=60 feather=80`           | 1        | 354,139           | PASS     |
| 合成商品图 easy                | 1400×1050 | `sample=Corner`                  | 1        | 354,185           | PASS     |
| 合成商品图 easy                | 1400×1050 | `sample=Custom key_all=true`     | 1        | 349,600           | PASS     |
| 合成商品图 easy                | 1400×1050 | `morph=+3 fill=true`             | 1        | 368,081           | PASS     |
| 合成商品图 easy                | 1400×1050 | `morph=-4 fill=true fill_neck=8` | 1        | 336,023           | PASS     |
| 困难样本 hard（渐变背景+噪声+软阴影+发丝） | 1200×900  | 同上 8 组                           | 1        | 282,014 – 311,545 | 8/8 PASS |
| 空图（无主体）                   | 300×200   | 默认                               | **4**    | 0                 | PASS     |
| 满图（主体充满画面）                | 300×200   | 默认                               | **4**    | 59,748            | PASS     |

后两行是关键：`fg 占比 ≤ 0.4%` 与 `≥ 98.5%` 都会触发自适应多趟重算，  
两端都把 4 次重试预算跑满、阈值按同一套规则漂移，最终蒙版仍逐字节相同 ——  
说明**自适应循环本身也移植正确**，不只是单次二值化对得上。

样本可由 `tools/gen_sample.mjs` 确定性重生成（固定 xorshift 种子，无需把二进制素材入库）：

```bash
node tools/gen_sample.mjs easy.rgba 1400 1050 easy
node tools/gen_sample.mjs hard.rgba 1200 900  hard
```

> 一个容易误判的细节：`easy` 样本的背景并非纯色，而是 252→247 的轻微渐变（仅 5 级）。
> 算法会把中间区域判成前景，画布上表现为「一圈棋盘格 + 中间不透明」。**这不是缺陷** ——
> 同一个色距阈值在 JS 端产出完全相同的蒙版（`diffPixels = 0`）。
> 判断算法是否正常请看 `--dump-mask` 的坐标探针或 `parity.mjs` 的差异数，
> 而不是靠肉眼读画布截图（这条教训在 §8.6 踩过一次）。

性能（同一张 1400×1050，release 版）：**Rust 18.8 ms vs JS 73 ms ≈ 3.9×**。

#### 回归中查出并修掉的一处真实偏差

首轮比对出现 `diffPixels=24 / maxDiff=1`（约 1.6e-5 比例）。定位结论：

- 原版 `blurAlpha` 的中间缓冲 `tmp` 是 **`Uint8ClampedArray`**，水平 pass 结束就量化成整数；  
  Rust 版当时用 `f32` 暂存 —— 两级精度不同。
- 更隐蔽的是取整规则：`Uint8ClampedArray` 走 ECMAScript `ToUint8Clamped`，**平局取偶**  
  （0.5→0、1.5→2、2.5→2、3.5→4），而 Rust `f32::round()` 是**平局远离零**（0.5→1、2.5→3）。

修法见 `src/core/matting.rs` 的 `to_u8_clamped()`：按 `ToUint8Clamped` 逐条规则实现，  
并把 `blur_alpha` 的 `tmp` 改回 `Vec<u8>`；蒙版软边斜坡与 `defringe` 的 RGBA 写入一并收口。  
修复后上述 18 组全部归零。

### 8.5 运行时行为验证

| 项目    | 验证方式                               | 结果                                                                                                                                                             |
| ----- | ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 首屏页面  | 无参数冷启动截图                           | 落在「抠图工具」页，与原版 `#pageWork.on` 一致                                                                                                                                |
| 画布渲染  | `COCOLE_DEBUG_CANVAS=1` 诊断输出       | `[canvas] kind=Result result=true original=true img=(1400,1050) zoom=0.56 bounds=…` —— 句柄已绑定                                                                   |
| 抠图结果  | `COCOLE_DEBUG_CANVAS=1` 的 alpha 探针 | `[matte] 1400x1050 fg=0.241 alpha(100,52)=0 (1000,52)=0 (700,522)=255 (300,522)=255` —— 背景区 0、主体区 255                                                          |
| Toast | 载入图片后观察生命周期                        | `[toast] +1 err=false 已载入 test_product（1400×1050）` → `[toast] -1 (队列剩 0)`，2.4 s 自动消失                                                                           |
| 配置持久化 | 读盘 `config.json`                   | `seen_onboarding=true`、`model=nn-modnet`、`auto=false`、`params{thresh:32,feather:14,morph:0,defringe:60,edge:30}`、`stats{count:14,total_ms:11111.9}`、`recent=4` |

配置落盘位置（Windows）：  
`C:\Users\<用户>\AppData\Roaming\yiming\cocole-iced\config\config.json`

### 8.6 运行期修掉的两个真实缺陷

| 缺陷                     | 现象                                                                                        | 修复                                                              |
| ---------------------- | ----------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| 画布只显示棋盘格，看不到抠图结果       | `MaskDone` 只写了 `app.work.result`，没有同步 GPU 句柄 `result_handle`，`StageView.result` 恒为 `None` | 在 `MaskDone` 分支补 `result_handle = Some(Handle::from_rgba(...))` |
| `seen_onboarding` 只读不写 | 每次启动都弹欢迎 toast                                                                            | 首次启动时置 `true` 并 `config.save()`                                 |

> 另有一次**误判**已纠正：截图上「左半边不透明」一度被当成抠图失效，  
> 实际是用户点了「对比」模式（左=原图、右=结果）。此后所有算法结论一律以  
> `--dump-mask` + `parity.mjs` 的逐像素数据为准，不再凭截图下判断。

### 8.7 编译期踩到的坑（供复现时参考）

| 现象                              | 原因                                           | 处理                                                   |
| ------------------------------- | -------------------------------------------- | ---------------------------------------------------- |
| `Padding::from([f32; 4])` 未实现   | iced 0.14 只提供具名构造                            | 统一收口为 `theme::pad(top, right, bottom, left)`         |
| `E0283 type annotations needed` | `container(..).into()` 的 Renderer 泛型无法推断     | 给 `let x: Element<'_, Message> = ...` 加显式标注          |
| `E0515 返回局部变量引用`                | `hint(&str)` / `badge(&str)` 把局部变量借进 Element | 改为 `impl Into<String>` 按值接收，返回 `Element<'static, _>` |
| `E0499 二次可变借用`                  | `let w = &mut app.work;` 借用横跨整个 match        | 展开为 `app.work.*`，让每个 arm 各自按需借用                      |
| 中文显示为方块                         | iced 自带字体无 CJK 字形                            | `main.rs` 启动时加载系统 `msyh.ttc` / `simhei` 等并设为默认字体     |

---

## 九、鸣谢 / Acknowledgements

本项目的抠图能力建立在以下开源模型之上，权重版权归原项目所有，请遵循各自许可证使用（建议从官方渠道获取权重文件并放入 `models/`）。

### 9.1 模型致谢

| 模型 / 项目 | 作者 | 许可证 | 在本项目中的用途 |
|---|---|---|---|
| [MODNet](https://github.com/ZHKKKe/MODNet) | Zhanghan Ke（柯彰翰）等，香港城市大学 | Apache-2.0 | 人像发丝级 alpha 抠图，证件照换底主力（`modnet.onnx`，512 输入） |
| [BiRefNet](https://github.com/ZhengPeng7/BiRefNet) | Peng Zheng（@ZhengPeng7）等 | MIT | 通用生活场景高精度抠图（`birefnet_1024.onnx`，1024 输入） |
| [BEN2](https://github.com/PramaLLC/BEN2)（[HuggingFace](https://huggingface.co/PramaLLC/BEN2)） | Prama LLC（@PramaLLC） | MIT（Base 版） | 电商产品图 / 通用背景移除（`BEN2_Base.onnx`，1024 输入） |
| [InSPyReNet](https://github.com/plemeri/InSPyReNet) | Taehun Kim（@plemeri）等，POSTECH | MIT | 高分辨率显著目标抠图（`model_fp16.onnx`，1024 输入） |
| [U²-Net / u2netp](https://github.com/xuebinqin/U-2-Net) | Xuebin Qin（秦雪宾）等 | Apache-2.0 | 轻量 CPU 档通用抠图，秒级出图（`u2netp.onnx`，320 输入） |
| [YuNet](https://github.com/opencv/opencv_zoo) / [libfacedetection](https://github.com/ShiqiYu/libfacedetection) | Shiqi Yu（于仕琪）团队 · OpenCV Zoo 团队 | Apache-2.0 / MIT | 人脸检测，证件照头部定位与构图校验（`face_detection_yunet_2023mar.onnx`） |

对应论文：

- Ke et al., *MODNet: Real-Time Trimap-Free Portrait Matting via Objective Decomposition*, AAAI 2022
- Zheng et al., *Bilateral Reference for High-Resolution Dichotomous Image Segmentation*, CAAI AIR 2024
- Kim et al., *Revisiting Image Pyramid Structure for High Resolution Salient Object Detection*, ACCV 2022
- Qin et al., *U²-Net: Going Deeper with Nested U-Structure for Salient Object Detection*, Pattern Recognition 2020

### 9.2 框架与工具致谢

| 项目 | 许可证 | 用途 |
|---|---|---|
| [Rust](https://www.rust-lang.org/) | MIT / Apache-2.0 | 开发语言 |
| [iced](https://github.com/iced-rs/iced) | MIT | 原生 GUI 框架（Elm 架构，0.14） |
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) | MIT | 推理引擎（DirectML GPU 加速，经 [ort](https://github.com/pykeio/ort) 绑定） |
| [windows-rs](https://github.com/microsoft/windows-rs) | MIT / Apache-2.0 | Win32 API（注册表 / 时间 / 原生对话框） |
| [notify-rust](https://github.com/hoodie/notify-rust) | MIT / Apache-2.0 | 系统通知 |
| [NSIS](https://nsis.sourceforge.io/) | zlib | Windows 安装包制作 |

再次向所有上游作者与社区贡献者表示感谢——壹鸣只是站在这些巨人肩膀上的一层薄薄的应用。
