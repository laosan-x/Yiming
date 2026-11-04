//! 抠图内核 —— SmartMatte v2 算法的 Rust 移植
//!
//! 与前端 `src/matting/{fields,mask,morpho,engine}.js` 逐行对齐：
//!   取样 → 色距场 / 梯度场 → 自适应多趟二值化 → 形态学 → 补孔洞 → 羽化。
//! 所有可调参数与 JS 版取值一致（阈值 1..100、羽化 0..100 等），
//! 保证同一张图在两端得到逐像素一致的结果。

use serde::{Deserialize, Serialize};

/* ═════════════════ 参数 ═════════════════ */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SampleMode {
    /// 自动（四角取样）
    #[default]
    Auto,
    /// 仅左上角
    Corner,
    /// 自定义颜色
    Custom,
}

impl SampleMode {
    pub const ALL: &'static [SampleMode] = &[SampleMode::Auto, SampleMode::Corner, SampleMode::Custom];
    pub fn label(&self) -> &'static str {
        match self {
            SampleMode::Auto => "自动（四角取样）",
            SampleMode::Corner => "仅左上角",
            SampleMode::Custom => "自定义颜色",
        }
    }
}

impl std::fmt::Display for SampleMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 蒙版参数，与前端 `state.params` 一一对应
///
/// `#[serde(default)]`：旧版 config.json 缺少后加的字段时也能正常反序列化（向后兼容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MatteParams {
    pub sample: SampleMode,
    pub custom: [u8; 3],
    /// 全图色键（仅自定义取样下生效）：true=不看连通性，纯按颜色判定
    pub key_all: bool,
    /// 孔洞判定的通道宽度阈值（px）
    pub fill_neck: i32,
    pub thresh: i32,
    pub feather: i32,
    pub morph: i32,
    pub defringe: i32,
    pub edge: i32,
    pub fill: bool,
    pub shadow: bool,
}

impl Default for MatteParams {
    fn default() -> Self {
        Self {
            sample: SampleMode::Auto,
            custom: [255, 255, 255],
            key_all: false,
            fill_neck: 4,
            thresh: 32,
            feather: 14,
            morph: 0,
            defringe: 60,
            edge: 30,
            fill: false,
            shadow: false,
        }
    }
}

/// 一次推理的产出
#[derive(Debug, Clone)]
pub struct MaskResult {
    /// 算法原始输出（画笔前的备份）
    pub base: Vec<u8>,
    /// 当前蒙版（画笔会在此基础上修改）
    pub mask: Vec<u8>,
    pub bg: [u8; 3],
    pub fg_count: usize,
    pub holes_filled: usize,
    pub elapsed_ms: f32,
}

/* ═════════════════ 场计算 ═════════════════ */

/// 取样背景色：取四角小块的中位数（抗噪）。
/// `custom` 模式下直接返回用户指定色。
    #[allow(clippy::needless_range_loop)] // 热循环按索引写入（多缓冲同索引），迭代器改写不减可读性
pub fn sample_bg(data: &[u8], w: usize, h: usize, mode: SampleMode, custom: [u8; 3]) -> [u8; 3] {
    if mode == SampleMode::Custom {
        return custom;
    }
    let patches: [(usize, usize); 4] = if mode == SampleMode::Corner {
        [(0, 0), (0, 0), (0, 0), (0, 0)]
    } else {
        [(0, 0), (1, 0), (0, 1), (1, 1)] // 用 x=1 表示「靠右」
    };
    let pc = if mode == SampleMode::Corner { 1 } else { 4 };

    let s = std::cmp::max(6, (std::cmp::min(w, h) as f32 * 0.04).floor() as usize);
    let s = std::cmp::min(s, std::cmp::min(w, h));
    let total = pc * s * s;
    let mut r = vec![0u8; total];
    let mut g = vec![0u8; total];
    let mut b = vec![0u8; total];
    let mut k = 0usize;

    for pi in 0..pc {
        let (px, py) = patches[pi];
        let x0 = if px == 0 { 0 } else { w - s };
        let y0 = if py == 0 { 0 } else { h - s };
        for y in y0..y0 + s {
            let mut i = (y * w + x0) * 4;
            for _ in 0..s {
                r[k] = data[i];
                g[k] = data[i + 1];
                b[k] = data[i + 2];
                i += 4;
                k += 1;
            }
        }
    }
    let med = |mut a: Vec<u8>| -> u8 {
        a.sort_unstable();
        a[total >> 1]
    };
    [med(r), med(g), med(b)]
}

/// Sobel 梯度幅值（归一化到约 0..1），用于阻断背景扩散、保护淡轮廓。
    #[allow(clippy::needless_range_loop)] // 热循环按索引写入（多缓冲同索引），迭代器改写不减可读性
pub fn compute_grad(data: &[u8], w: usize, h: usize) -> Vec<f32> {
    let n = w * h;
    let mut gray = vec![0f32; n];
    for i in 0..n {
        let j = i * 4;
        gray[i] = (0.299 * data[j] as f32 + 0.587 * data[j + 1] as f32 + 0.114 * data[j + 2] as f32)
            / 255.0;
    }
    let mut grad = vec![0f32; n];
    for y in 1..h - 1 {
        let row = y * w;
        for x in 1..w - 1 {
            let i = row + x;
            let gx = gray[i - w + 1] + 2.0 * gray[i + 1] + gray[i + w + 1]
                - gray[i - w - 1]
                - 2.0 * gray[i - 1]
                - gray[i + w - 1];
            let gy = gray[i + w - 1] + 2.0 * gray[i + w] + gray[i + w + 1]
                - gray[i - w - 1]
                - 2.0 * gray[i - w]
                - gray[i - w + 1];
            grad[i] = (gx * gx + gy * gy).sqrt() * 0.25;
        }
    }
    grad
}

/// 每像素到背景色的距离场（整条流水线里最重的一段，故与梯度一起缓存复用）。
    #[allow(clippy::needless_range_loop)] // 热循环按索引写入（多缓冲同索引），迭代器改写不减可读性
pub fn compute_dist(data: &[u8], w: usize, h: usize, bg: [u8; 3]) -> Vec<f32> {
    let n = w * h;
    let (br, bgc, bb) = (bg[0] as f32, bg[1] as f32, bg[2] as f32);
    let mut dist = vec![0f32; n];
    for i in 0..n {
        let j = i * 4;
        let dr = (data[j] as f32 - br) / 255.0;
        let dg = (data[j + 1] as f32 - bgc) / 255.0;
        let db = (data[j + 2] as f32 - bb) / 255.0;
        let dy = 0.299 * dr + 0.587 * dg + 0.114 * db;
        let dc = (0.5 * (dr - dg) * (dr - dg) + 0.5 * (db - dg) * (db - dg)).sqrt();
        dist[i] = (0.62 * dy * dy
            + 0.38 * (dr * dr + dg * dg + db * db) / 3.0
            + 0.55 * dc * dc)
            .sqrt();
    }
    dist
}

/* ═════════════════ 单趟二值化 ═════════════════ */

/// 单趟 mask 计算。`dist` 由外部传入并复用，自适应多趟不再各自重算。
#[allow(clippy::too_many_arguments)] // 与 JS 版 passMask 参数一一对应（逐像素对齐口径）
/// 单趟蒙版计算（无取消检查版）。生产链路走 `pass_mask_cancellable`，
/// 这里保留公开算法 API 供基准/诊断脚本与单测对照使用。
#[allow(dead_code)]
pub fn pass_mask(
    w: usize,
    h: usize,
    t: f32,
    f: f32,
    edge_t: f32,
    grad: &[f32],
    key_all: bool,
    dist: &[f32],
) -> Vec<u8> {
    pass_mask_impl(w, h, t, f, edge_t, grad, key_all, dist, None)
        .expect("uncancellable pass cannot be cancelled")
}

/// 带取消回调的 pass_mask：回调返回 true 时立即放弃本次计算（G4）。
// 算法内核的 8 个几何/参数项全都会被用到，拆成配置结构只会让逐像素热路径
// 多一层间接寻址；这里按项目约定显式豁免 too_many_arguments。
#[allow(clippy::too_many_arguments)]
pub(crate) fn pass_mask_cancellable(
    w: usize,
    h: usize,
    t: f32,
    f: f32,
    edge_t: f32,
    grad: &[f32],
    key_all: bool,
    dist: &[f32],
    cancel: &dyn Fn() -> bool,
) -> Option<Vec<u8>> {
    pass_mask_impl(w, h, t, f, edge_t, grad, key_all, dist, Some(cancel))
}

#[allow(clippy::too_many_arguments)]
fn pass_mask_impl(
    w: usize,
    h: usize,
    t: f32,
    f: f32,
    edge_t: f32,
    grad: &[f32],
    key_all: bool,
    dist: &[f32],
    cancel: Option<&dyn Fn() -> bool>,
) -> Option<Vec<u8>> {
    let n = w * h;
    let is_cancelled = || cancel.map(|f| f()).unwrap_or(false);
    let mut alpha = vec![0u8; n];
    let lo = t - f * 0.5;
    let hi = t + f * 0.5;
    let hard = t + f * 0.5;

    // 背景似然：颜色接近 + 非轮廓（全图色键模式下跳过轮廓判定）
    if is_cancelled() {
        return None;
    }
    let is_bg_like: Vec<u8> = (0..n)
        .map(|i| if dist[i] < hard && (key_all || grad[i] < edge_t) { 1 } else { 0 })
        .collect();

    let mut visited = vec![0u8; n];
    if key_all {
        for i in 0..n {
            if is_bg_like[i] == 1 {
                visited[i] = 1;
            }
        }
    } else {
        // 连通域：背景必须「颜色接近」且「非轮廓」才能从边缘扩散进来
        let mut queue = vec![0i32; n];
        let (mut qh, mut qt) = (0usize, 0usize);
        let push = |i: usize, visited: &mut Vec<u8>, queue: &mut Vec<i32>, qt: &mut usize| {
            if visited[i] == 0 && is_bg_like[i] == 1 {
                visited[i] = 1;
                queue[*qt] = i as i32;
                *qt += 1;
            }
        };
        for x in 0..w {
            push(x, &mut visited, &mut queue, &mut qt);
            push((h - 1) * w + x, &mut visited, &mut queue, &mut qt);
        }
        for y in 0..h {
            push(y * w, &mut visited, &mut queue, &mut qt);
            push(y * w + w - 1, &mut visited, &mut queue, &mut qt);
        }
        while qh < qt {
            // 连通域 BFS 是大图最耗时的一段：每 65536 个像素查一次取消令牌
            if (qh & 0xFFFF) == 0 && is_cancelled() {
                return None;
            }
            let i = queue[qh] as usize;
            qh += 1;
            let x = i % w;
            if x > 0 {
                push(i - 1, &mut visited, &mut queue, &mut qt);
            }
            if x < w - 1 {
                push(i + 1, &mut visited, &mut queue, &mut qt);
            }
            if i >= w {
                push(i - w, &mut visited, &mut queue, &mut qt);
            }
            if i + w < n {
                push(i + w, &mut visited, &mut queue, &mut qt);
            }
        }
    }

    let span = (hi - lo).max(1e-6);
    for i in 0..n {
        if (i & 0xFFFF) == 0 && is_cancelled() {
            return None;
        }
        if visited[i] == 0 {
            alpha[i] = 255; // 被轮廓围住的内部 → 强制保留
        } else {
            let d = dist[i];
            alpha[i] = if d <= lo {
                0
            } else if d >= hi {
                255
            } else {
                to_u8_clamped(255.0 * (d - lo) / span)
            };
        }
    }
    Some(alpha)
}

pub fn count_fg(mask: &[u8]) -> usize {
    mask.iter().filter(|&&v| v > 128).count()
}

/* ═════════════════ 形态学（可分离滑窗，单像素 O(1)） ═════════════════ */

/// 可分离矩形结构元的极值滤波（膨胀 ext>0 / 腐蚀 ext<0）。
/// 单调队列滑窗：每个下标进出队各一次，与半径无关。
fn separable_extreme(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, ext: i8) {
    let mut mid = vec![0u8; w * h];
    let mut q = vec![0i32; w.max(h).max(1)];
    let keep = ext > 0;

    // 水平
    for y in 0..h {
        let row = y * w;
        let (mut head, mut tail) = (0usize, 0usize);
        for i in 0..w {
            let v = src[row + i];
            while tail > head {
                let lv = src[row + q[tail - 1] as usize];
                if if keep { lv <= v } else { lv >= v } {
                    tail -= 1;
                } else {
                    break;
                }
            }
            q[tail] = i as i32;
            tail += 1;
            if i >= r {
                let x = i - r;
                let lim = x as i32 - r as i32;
                while head < tail && q[head] < lim {
                    head += 1;
                }
                mid[row + x] = src[row + q[head] as usize];
            }
        }
        for x in w.saturating_sub(r)..w {
            let lim = x as i32 - r as i32;
            while head < tail && q[head] < lim {
                head += 1;
            }
            mid[row + x] = src[row + q[head] as usize];
        }
    }

    // 垂直
    for x in 0..w {
        let (mut head, mut tail) = (0usize, 0usize);
        for i in 0..h {
            let idx = i * w + x;
            let v = mid[idx];
            while tail > head {
                let lv = mid[q[tail - 1] as usize * w + x];
                if if keep { lv <= v } else { lv >= v } {
                    tail -= 1;
                } else {
                    break;
                }
            }
            q[tail] = i as i32;
            tail += 1;
            if i >= r {
                let y = i - r;
                let lim = y as i32 - r as i32;
                while head < tail && q[head] < lim {
                    head += 1;
                }
                dst[y * w + x] = mid[q[head] as usize * w + x];
            }
        }
        for y in h.saturating_sub(r)..h {
            let lim = y as i32 - r as i32;
            while head < tail && q[head] < lim {
                head += 1;
            }
            dst[y * w + x] = mid[q[head] as usize * w + x];
        }
    }
}

/// 二值形态学：is_max=true 膨胀 / false 腐蚀
fn morph_bin(src: &[u8], w: usize, h: usize, r: usize, is_max: bool) -> Vec<u8> {
    if r == 0 {
        return src.to_vec();
    }
    let mut out = vec![0u8; w * h];
    separable_extreme(src, &mut out, w, h, r, if is_max { 1 } else { -1 });
    out
}

const HOLE_A: u8 = 128;

/// 填充内部孔洞：①完全封闭 ②窄颈渗入（腐蚀 neck 切断细通道后再判连通）
pub fn fill_holes(a: &mut [u8], w: usize, h: usize, neck: usize) -> usize {
    let n = w * h;
    let t: Vec<u8> = (0..n).map(|i| if a[i] < HOLE_A { 1 } else { 0 }).collect();

    // 腐蚀：切断细窄通道
    let core: Vec<u8> = if neck > 0 {
        morph_bin(&t, w, h, neck, false)
    } else {
        t.clone()
    };

    // 从四条边出发，沿 core 做 BFS，标记「确实通向外部」
    let mut reach = vec![0u8; n];
    let mut queue = vec![0i32; n];
    let (mut qh, mut qt) = (0usize, 0usize);
    let seed = |i: usize, reach: &mut Vec<u8>, queue: &mut Vec<i32>, qt: &mut usize| {
        if core[i] == 1 && reach[i] == 0 {
            reach[i] = 1;
            queue[*qt] = i as i32;
            *qt += 1;
        }
    };
    for x in 0..w {
        seed(x, &mut reach, &mut queue, &mut qt);
        seed((h - 1) * w + x, &mut reach, &mut queue, &mut qt);
    }
    for y in 0..h {
        seed(y * w, &mut reach, &mut queue, &mut qt);
        seed(y * w + w - 1, &mut reach, &mut queue, &mut qt);
    }
    while qh < qt {
        let i = queue[qh] as usize;
        qh += 1;
        let x = i % w;
        let y = i / w;
        if x > 0 {
            seed(i - 1, &mut reach, &mut queue, &mut qt);
        }
        if x < w - 1 {
            seed(i + 1, &mut reach, &mut queue, &mut qt);
        }
        if y > 0 {
            seed(i - w, &mut reach, &mut queue, &mut qt);
        }
        if y < h - 1 {
            seed(i + w, &mut reach, &mut queue, &mut qt);
        }
    }

    // 膨胀回来，还原被腐蚀掉的正常背景边缘带
    let ok: Vec<u8> = if neck > 0 {
        morph_bin(&reach, w, h, neck, true)
    } else {
        reach
    };

    let mut filled = 0usize;
    for i in 0..n {
        if t[i] == 1 && ok[i] == 0 {
            a[i] = 255;
            filled += 1;
        }
    }
    filled
}

/// 蒙版收缩(-) / 扩张(+)，原地作用
pub fn morph_op(a: &mut [u8], w: usize, h: usize, r: i32) {
    if r == 0 {
        return;
    }
    let k = r.unsigned_abs() as usize;
    separable_extreme_from(a, w, h, k, r > 0);
}

/// 原地版（src 与 dst 同一份数据）
pub fn separable_extreme_from(src: &mut [u8], w: usize, h: usize, r: usize, keep: bool) {
    let snapshot = src.to_vec();
    separable_extreme(&snapshot, src, w, h, r, if keep { 1 } else { -1 });
}

/// alpha 快速模糊（盒式，2 pass，滑动求和，单像素 O(1)）
pub fn blur_alpha(a: &mut [u8], w: usize, h: usize, r: usize) {
    if r < 1 {
        return;
    }
    let inv = 1.0 / (2.0 * r as f32 + 1.0);
    // 原版中间缓冲是 Uint8ClampedArray：水平 pass 结束就量化成 u8。
    // 这里同样用 u8 暂存（而非 f32），否则两级结果会有 ±1 偏差。
    let mut tmp = vec![0u8; w * h];

    // 水平：窗口 [x-r, x+r]，越界按边缘钳位
    for y in 0..h {
        let row = y * w;
        let mut s = 0f32;
        for d in -(r as i32)..=(r as i32) {
            s += a[row + clampi(d, 0, w as i32 - 1) as usize] as f32;
        }
        tmp[row] = to_u8_clamped(s * inv);
        for x in 1..w {
            s += a[row + clampi(x as i32 + r as i32, 0, w as i32 - 1) as usize] as f32
                - a[row + clampi(x as i32 - r as i32 - 1, 0, w as i32 - 1) as usize] as f32;
            tmp[row + x] = to_u8_clamped(s * inv);
        }
    }
    // 垂直
    for x in 0..w {
        let mut s = 0f32;
        for d in -(r as i32)..=(r as i32) {
            s += tmp[clampi(d, 0, h as i32 - 1) as usize * w + x] as f32;
        }
        a[x] = to_u8_clamped(s * inv);
        for y in 1..h {
            s += tmp[clampi(y as i32 + r as i32, 0, h as i32 - 1) as usize * w + x] as f32
                - tmp[clampi(y as i32 - r as i32 - 1, 0, h as i32 - 1) as usize * w + x] as f32;
            a[y * w + x] = to_u8_clamped(s * inv);
        }
    }
}

fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    v.clamp(lo, hi)
}

/// 复刻 ECMAScript `ToUint8Clamped`（写入 `Uint8ClampedArray` 时的转换规则）。
///
/// 与 Rust 内置 `f32::round()` 的关键差别：**平局取偶**（0.5→0、1.5→2、2.5→2、3.5→4），
/// 而 `round()` 是「平局远离零」（0.5→1、2.5→3）。原版 JS 的蒙版数组与羽化中间缓冲
/// 都是 `Uint8ClampedArray`，每一级赋值都会走这个量化；若这里用 `round()`，
/// 羽化半径较大时会出现约 1e-5 比例的 ±1 逐像素偏差（肉眼不可见，但回归比对会失败）。
fn to_u8_clamped(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    if v <= 0.0 {
        return 0;
    }
    if v >= 255.0 {
        return 255;
    }
    let f = v.floor();
    if f + 0.5 < v {
        (f + 1.0) as u8
    } else if v < f + 0.5 {
        f as u8
    } else if f % 2.0 != 0.0 {
        // 恰好落在 .5：奇数向上取偶
        (f + 1.0) as u8
    } else {
        f as u8
    }
}

/* ═════════════════ 主流程 ═════════════════ */

/// 场缓存：梯度只依赖源图，色距场只依赖「源图 + 背景色」
#[derive(Default)]
pub struct FieldCache {
    grad: Option<Vec<f32>>,
    dist: Option<Vec<f32>>,
    bg_key: String,
}

impl FieldCache {
    fn grad(&mut self, data: &[u8], w: usize, h: usize) -> &Vec<f32> {
        if self.grad.is_none() {
            self.grad = Some(compute_grad(data, w, h));
        }
        self.grad.as_ref().expect("just set")
    }
}

/// 计算蒙版（双向自适应：全被吃掉 / 全部保留 时自动修正）
pub fn compute_mask(
    data: &[u8],
    w: usize,
    h: usize,
    p: &MatteParams,
    cache: &mut FieldCache,
) -> MaskResult {
    compute_mask_impl(data, w, h, p, cache, &|| false)
        .expect("uncancellable compute cannot be cancelled")
}

/// 带取消回调的 compute_mask（G4）。与 `compute_mask` 算法完全一致，
/// 但在各耗时阶段及 `pass_mask` 的 BFS 循环内检查取消令牌；返回 `None`
/// 表示任务已过期，调用方应静默丢弃结果。
pub fn compute_mask_cancellable(
    data: &[u8],
    w: usize,
    h: usize,
    p: &MatteParams,
    cache: &mut FieldCache,
    cancel: &dyn Fn() -> bool,
) -> Option<MaskResult> {
    compute_mask_impl(data, w, h, p, cache, cancel)
}

fn compute_mask_impl(
    data: &[u8],
    w: usize,
    h: usize,
    p: &MatteParams,
    cache: &mut FieldCache,
    cancel: &dyn Fn() -> bool,
) -> Option<MaskResult> {
    let t0 = std::time::Instant::now();
    let n = w * h;
    if cancel() {
        return None;
    }
    let bg = sample_bg(data, w, h, p.sample, p.custom);

    let f = (p.feather as f32 / 100.0).max(0.001);
    let base_edge = (p.edge as f32 / 100.0) * 0.28;

    let grad = cache.grad(data, w, h).clone();
    let key = format!("{},{},{}", bg[0], bg[1], bg[2]);
    if cache.dist.is_none() || cache.bg_key != key {
        cache.dist = Some(compute_dist(data, w, h, bg));
        cache.bg_key = key;
    }
    let dist = cache.dist.clone().expect("just set");
    if cancel() {
        return None;
    }

    let key_all = p.sample == SampleMode::Custom && p.key_all;
    let mut alpha = Vec::new();
    let mut t = p.thresh as f32 / 100.0;
    let mut edge_t = base_edge;

    for _attempt in 0..4 {
        if cancel() {
            return None;
        }
        alpha = pass_mask_cancellable(w, h, t, f, edge_t, &grad, key_all, &dist, cancel)?;
        let fg = alpha.iter().filter(|&&v| v > 128).count();
        let ratio = fg as f32 / n as f32;
        if ratio > 0.004 && ratio < 0.985 {
            break;
        }
        if key_all {
            if ratio <= 0.004 {
                if t < 0.9 {
                    t *= 1.6;
                } else {
                    break;
                }
            } else if t > 0.02 {
                t *= 0.6;
            } else {
                break;
            }
        } else if ratio <= 0.004 {
            if edge_t > 0.004 {
                edge_t *= 0.4;
            } else if t < 0.9 {
                t *= 1.6;
            } else {
                break;
            }
        } else {
            if edge_t < 1.2 {
                edge_t *= 2.2;
            } else if t > 0.02 {
                t *= 0.6;
            } else {
                break;
            }
        }
    }

    if cancel() {
        return None;
    }
    // 形态学：收缩(-) / 扩张(+)
    if p.morph != 0 {
        morph_op(&mut alpha, w, h, p.morph);
    }
    // 填充内部孔洞（放在形态学之后，形态学新开出的孔也能补上）
    let holes_filled = if p.fill {
        fill_holes(&mut alpha, w, h, p.fill_neck.max(0) as usize)
    } else {
        0
    };
    if cancel() {
        return None;
    }
    // 羽化
    if p.feather > 0 {
        blur_alpha(&mut alpha, w, h, std::cmp::max(1, (p.feather as f32 / 22.0).round() as usize));
    }

    let fg_count = count_fg(&alpha);
    Some(MaskResult {
        mask: alpha.clone(),
        base: alpha,
        bg,
        fg_count,
        holes_filled,
        elapsed_ms: t0.elapsed().as_secs_f32() * 1000.0,
    })
}

/* ═════════════════ 去色晕 Defringe ═════════════════ */

/// 去色晕：把半透明边缘像素的颜色拉向前景内部的不透明色，
/// 强度 0..100。与前端 composite 阶段的 defringe 处理对齐。
pub fn defringe(rgba: &mut [u8], mask: &[u8], w: usize, h: usize, strength: i32) {
    if strength <= 0 {
        return;
    }
    let strength = strength.min(100) as f32 / 100.0;
    let src = rgba.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let a = mask[i];
            if a >= 250 || a <= 5 {
                continue;
            }
            // 在 3×3 邻域里找最不透明的前景色
            let mut best = i;
            let mut best_a = a;
            for dy in -1..=1i32 {
                for dx in -1..=1i32 {
                    let j = ((y as i32 + dy) as usize) * w + (x as i32 + dx) as usize;
                    if mask[j] > best_a {
                        best_a = mask[j];
                        best = j;
                    }
                }
            }
            if best == i {
                continue;
            }
            let k = i * 4;
            let b = best * 4;
            for c in 0..3 {
                let v = src[k + c] as f32;
                let t = src[b + c] as f32;
                rgba[k + c] = to_u8_clamped(v + (t - v) * strength);
            }
        }
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;

    /// 中心深色前景 + 浅色背景，让色键算法有实际像素要处理
    fn sample_image(w: usize, h: usize) -> Vec<u8> {
        let mut d = vec![240u8; w * h * 4];
        for y in (h / 4)..(3 * h / 4) {
            for x in (w / 4)..(3 * w / 4) {
                let i = (y * w + x) * 4;
                d[i] = 30;
                d[i + 1] = 60;
                d[i + 2] = 120;
            }
        }
        d
    }

    #[test]
    fn cancellation_short_circuits_compute() {
        let (w, h) = (128, 96);
        let d = sample_image(w, h);
        let p = MatteParams::default();
        let mut cache = FieldCache::default();
        assert!(compute_mask_cancellable(&d, w, h, &p, &mut cache, &|| true).is_none());
    }

    #[test]
    fn no_cancel_matches_original_api() {
        let (w, h) = (128, 96);
        let d = sample_image(w, h);
        let p = MatteParams::default();
        let a = compute_mask(&d, w, h, &p, &mut FieldCache::default()).mask;
        let b = compute_mask_cancellable(&d, w, h, &p, &mut FieldCache::default(), &|| false)
            .expect("not cancelled")
            .mask;
        assert_eq!(a, b, "取消版不得改变算法结果");
    }

    #[test]
    fn pass_mask_cancel_is_honoured() {
        let (w, h) = (64, 64);
        let n = w * h;
        let grad = vec![0.0f32; n];
        let dist = vec![0.5f32; n];
        assert!(
            pass_mask_cancellable(w, h, 0.5, 0.2, 0.2, &grad, false, &dist, &|| true)
                .is_none()
        );
        let a = pass_mask(w, h, 0.5, 0.2, 0.2, &grad, false, &dist);
        let b = pass_mask_cancellable(w, h, 0.5, 0.2, 0.2, &grad, false, &dist, &|| false)
            .expect("not cancelled");
        assert_eq!(a, b);
    }
}