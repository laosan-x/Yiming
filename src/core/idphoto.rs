//! 证件照：规格表 / 底色 / 相纸 / 构图 / 排版
//!
//! 数据与原项目 `src/core/idspecs.js`、`idlayout.js`、`idphoto.js` 对齐：
//!   - 毫米制与像素制规格分开存储（四六级 144×192px 这类按毫米出图会被系统拒收）
//!   - 底色只收有明文依据的官方色值

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use crate::core::i18n::tr;
use crate::core::matting::separable_extreme_from;

pub const ID_DPI: u32 = 300;

pub fn mm2px(mm: f32, dpi: u32) -> u32 {
    (mm * dpi as f32 / 25.4).round() as u32
}

/* ═════════════════ 规格 ═════════════════ */

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dim {
    /// 毫米制
    Mm { w: f32, h: f32 },
    /// 像素制（报名系统明文要求，与 DPI 无关）
    Px { w: u32, h: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IdSize {
    pub k: &'static str,
    pub n: &'static str,
    pub dim: Dim,
    pub g: &'static str,
    pub d: &'static str,
}

impl IdSize {
    /// 该规格在给定 DPI 下的输出像素
    pub fn px(&self, dpi: u32) -> (u32, u32) {
        match self.dim {
            Dim::Mm { w, h } => (mm2px(w, dpi), mm2px(h, dpi)),
            Dim::Px { w, h } => (w, h),
        }
    }
    /// 物理毫米尺寸（排版用；像素制按 300DPI 反推）
    pub fn mm(&self) -> (f32, f32) {
        match self.dim {
            Dim::Mm { w, h } => (w, h),
            Dim::Px { w, h } => (w as f32 * 25.4 / 300.0, h as f32 * 25.4 / 300.0),
        }
    }
    pub fn dim_text(&self) -> String {
        match self.dim {
            Dim::Px { w, h } => format!("{w}×{h}px"),
            Dim::Mm { w, h } => format!("{}×{}mm", w as i32, h as i32),
        }
    }
}

impl std::fmt::Display for IdSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} · {}", self.n, self.dim_text())
    }
}

pub const ID_SIZES: &[IdSize] = &[
    IdSize { k: "one_inch",   n: "一寸",           dim: Dim::Mm { w: 25.0, h: 35.0 }, g: "国内通用",   d: "简历、入职、社保" },
    IdSize { k: "small_one",  n: "小一寸",         dim: Dim::Mm { w: 22.0, h: 32.0 }, g: "国内通用",   d: "驾驶证" },
    IdSize { k: "large_one",  n: "大一寸",         dim: Dim::Mm { w: 33.0, h: 48.0 }, g: "国内通用",   d: "护照同版" },
    IdSize { k: "two_inch",   n: "二寸",           dim: Dim::Mm { w: 35.0, h: 49.0 }, g: "国内通用",   d: "资格证、档案" },
    IdSize { k: "small_two",  n: "小二寸",         dim: Dim::Mm { w: 35.0, h: 45.0 }, g: "国内通用",   d: "公务员、签证" },
    IdSize { k: "large_two",  n: "大二寸",         dim: Dim::Mm { w: 35.0, h: 53.0 }, g: "国内通用",   d: "人事档案" },
    IdSize { k: "cn_passport",n: "中国护照",       dim: Dim::Mm { w: 33.0, h: 48.0 }, g: "出入境",     d: "因私护照" },
    IdSize { k: "cn_visa",    n: "中国签证",       dim: Dim::Mm { w: 33.0, h: 48.0 }, g: "出入境",     d: "来华签证" },
    IdSize { k: "hk_mo",      n: "港澳通行证",     dim: Dim::Mm { w: 33.0, h: 48.0 }, g: "出入境",     d: "往来港澳" },
    IdSize { k: "schengen",   n: "申根签证",       dim: Dim::Mm { w: 35.0, h: 45.0 }, g: "出入境",     d: "欧洲申根" },
    IdSize { k: "us_visa",    n: "美国签证",       dim: Dim::Px { w: 600, h: 600 },   g: "出入境",     d: "2×2 英寸（固定 600×600px）" },
    IdSize { k: "jp_visa",    n: "日本签证",       dim: Dim::Mm { w: 45.0, h: 45.0 }, g: "出入境",     d: "日签正方形" },
    IdSize { k: "civil",      n: "公务员考试",     dim: Dim::Mm { w: 35.0, h: 45.0 }, g: "考试报名",   d: "国考省考（机器校验底色）" },
    IdSize { k: "teacher",    n: "教师资格证",     dim: Dim::Mm { w: 30.0, h: 40.0 }, g: "考试报名",   d: "教资认定" },
    IdSize { k: "cet",        n: "四六级",         dim: Dim::Px { w: 144, h: 192 },   g: "考试报名",   d: "CET 固定 144×192px" },
    IdSize { k: "ncre",       n: "计算机等级",     dim: Dim::Px { w: 390, h: 567 },   g: "考试报名",   d: "NCRE 固定 390×567px" },
    IdSize { k: "postgrad",   n: "研究生考试",     dim: Dim::Px { w: 531, h: 709 },   g: "考试报名",   d: "研招网固定 531×709px" },
    IdSize { k: "id_card",    n: "身份证",         dim: Dim::Mm { w: 26.0, h: 32.0 }, g: "证件办理",   d: "二代身份证" },
    IdSize { k: "social",     n: "社保卡",         dim: Dim::Px { w: 358, h: 441 },   g: "证件办理",   d: "社保卡固定 358×441px" },
    IdSize { k: "e_dl",       n: "电子驾驶证",     dim: Dim::Px { w: 260, h: 378 },   g: "证件办理",   d: "交管 12123 固定 260×378px" },
];

/* ═════════════════ 相纸 ═════════════════ */

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IdPaper {
    pub k: &'static str,
    pub n: &'static str,
    pub w: f32,
    pub h: f32,
}

impl std::fmt::Display for IdPaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.n)
    }
}

pub const ID_PAPERS: &[IdPaper] = &[
    IdPaper { k: "6inch", n: "六寸 102×152mm", w: 102.0, h: 152.0 },
    IdPaper { k: "5inch", n: "五寸 89×127mm",  w: 89.0,  h: 127.0 },
    IdPaper { k: "7inch", n: "七寸 127×178mm", w: 127.0, h: 178.0 },
    IdPaper { k: "A4",    n: "A4 210×297mm",   w: 210.0, h: 297.0 },
];

/* ═════════════════ 底色 ═════════════════ */

#[derive(Debug, Clone, Copy)]
pub struct IdBg {
    pub n: &'static str,
    pub c: &'static str,
    pub d: &'static str,
}

pub const ID_BGS: &[IdBg] = &[
    IdBg { n: "标准白",  c: "#FFFFFF", d: "护照 / 身份证 / 签证 / 驾驶证 —— 纯白，不可有阴影" },
    IdBg { n: "标准蓝",  c: "#438EDB", d: "简历 / 工牌 / 学历档案（广泛通行值）" },
    IdBg { n: "考公蓝",  c: "#0066CC", d: "公务员 / 事业单位 —— 机器校验底色（有明文依据）" },
    IdBg { n: "CET浅蓝", c: "#64C5FF", d: "四六级报名浅蓝（官方二选一）" },
    IdBg { n: "申根灰",  c: "#DCDCDC", d: "申根签证浅灰（200–220 通行区间）" },
    IdBg { n: "标准红",  c: "#FF0000", d: "结婚登记 / 荣誉证书（民政规范）" },
    IdBg { n: "档案红",  c: "#CC0000", d: "党团组织关系 / 工会 / 职称证" },
];

pub fn parse_hex(s: &str) -> [u8; 3] {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0xFFFFFF);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/* ═════════════════ 背景渲染 ═════════════════ */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RenderMode {
    /// 纯色平铺
    #[default]
    Solid,
    /// 上下渐变：顶端底色 → 底部纯白
    Vertical,
    /// 中心渐变：中心纯白 → 外圈底色
    Center,
    /// 透明底
    Transparent,
    /// 原背景（不换底）
    Original,
}

impl RenderMode {
    pub fn label(&self) -> &'static str {
        match self {
            RenderMode::Solid => tr("纯色"),
            RenderMode::Vertical => tr("上下渐变"),
            RenderMode::Center => tr("中心渐变"),
            RenderMode::Transparent => tr("透明底"),
            RenderMode::Original => tr("原背景"),
        }
    }
    pub const ALL: &'static [RenderMode] = &[
        RenderMode::Solid,
        RenderMode::Vertical,
        RenderMode::Center,
        RenderMode::Transparent,
        RenderMode::Original,
    ];
    /// 各渲染模式的说明文案（随模式与底色联动，参考 index.html `#idRenderDesc`）
    pub fn desc(&self) -> &'static str {
        match self {
            RenderMode::Solid => tr("纯色平铺 · 常规证件照底"),
            RenderMode::Vertical => tr("顶端底色向下过渡到白 · 常见「渐变蓝」"),
            RenderMode::Center => tr("中心白向外过渡到底色 · 影楼常用"),
            RenderMode::Transparent => tr("保留透明通道，不铺底色"),
            RenderMode::Original => tr("保留原图背景，不做换底"),
        }
    }
}

impl std::fmt::Display for RenderMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 按渲染模式铺背景（透明 / 原背景不铺）
fn paint_bg(buf: &mut RgbaImage, mode: RenderMode, color: [u8; 3]) {
    let (w, h) = (buf.width(), buf.height());
    if w == 0 || h == 0 {
        return;
    }
    match mode {
        RenderMode::Transparent | RenderMode::Original => {}
        RenderMode::Solid => {
            for px in buf.pixels_mut() {
                px.0 = [color[0], color[1], color[2], 255];
            }
        }
        RenderMode::Vertical => {
            let mut raw = std::mem::replace(buf, RgbaImage::new(1, 1)).into_raw();
            let stride = w as usize * 4;
            for y in 0..h as usize {
                let t = y as f32 / (h - 1).max(1) as f32;
                let c = mix(color, [255, 255, 255], t);
                let row_off = y * stride;
                for x in 0..w as usize {
                    let off = row_off + x * 4;
                    raw[off] = c[0];
                    raw[off + 1] = c[1];
                    raw[off + 2] = c[2];
                    raw[off + 3] = 255;
                }
            }
            if let Some(back) = RgbaImage::from_raw(w, h, raw) {
                *buf = back;
            }
        }
        RenderMode::Center => {
            let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
            let max_r = w.max(h) as f32;
            let mut raw = std::mem::replace(buf, RgbaImage::new(1, 1)).into_raw();
            let stride = w as usize * 4;
            for y in 0..h as usize {
                let row_off = y * stride;
                for x in 0..w as usize {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    let t = (d / max_r).clamp(0.0, 1.0);
                    let c = mix([255, 255, 255], color, t);
                    let off = row_off + x * 4;
                    raw[off] = c[0];
                    raw[off + 1] = c[1];
                    raw[off + 2] = c[2];
                    raw[off + 3] = 255;
                }
            }
            if let Some(back) = RgbaImage::from_raw(w, h, raw) {
                *buf = back;
            }
        }
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t).round() as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t).round() as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t).round() as u8,
    ]
}

/* ═════════════════ 构图 ═════════════════ */

/// 方形腐蚀：使用单调队列优化的两趟滑动窗口最小值算法，复杂度 O(h*w)
pub fn erode_mask(m: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    if r == 0 {
        return m.to_vec();
    }
    let mut out = m.to_vec();
    separable_extreme_from(&mut out, w, h, r, false);
    out
}

/// 单张成品：规格像素的底色画布 + 前景按构图绘制。
///
/// 构图规则（与 idphoto.js::composeSingle 一致）：
/// 以「高度恰好铺满」为基准（cover by height），zoom 与垂直偏移在其上叠加。
///
/// 优化：先用 imageops::resize 缩放前景图到目标尺寸（内部 SIMD 友好），
/// 再做偏移 + alpha 合成，避免逐像素 get_pixel + 浮点插值。
pub fn compose_single(
    fg: &RgbaImage,
    spec: &IdSize,
    bg_color: [u8; 3],
    render: RenderMode,
    zoom_pct: i32,
    off_y_pct: i32,
    dpi: u32,
) -> RgbaImage {
    let (sw, sh) = spec.px(dpi);
    let (sw, sh) = (sw.max(1), sh.max(1));
    let mut out = RgbaImage::new(sw, sh);
    paint_bg(&mut out, render, bg_color);

    let src_w = fg.width() as f32;
    let src_h = fg.height().max(1) as f32;

    let scale = (sh as f32 / src_h) * (zoom_pct as f32 / 100.0);
    let dw = (src_w * scale).max(1.0).round() as u32;
    let dh = (src_h * scale).max(1.0).round() as u32;
    let dx = ((sw as f32 - dw as f32) / 2.0).round() as i64;
    let dy = ((sh as f32 - dh as f32) / 2.0 + (off_y_pct as f32 / 100.0) * sh as f32).round() as i64;

    if dw == 0 || dh == 0 {
        return out;
    }

    // Lanczos3：放大时边缘锐利（Triangle 双线性会上采样发糊，是导出清晰度
    // 下降的主因之一）。合成在后台线程执行，滤波耗时可比画质优先。
    let scaled = image::imageops::resize(fg, dw, dh, image::imageops::FilterType::Lanczos3);
    let scaled_raw = scaled.as_raw();
    let mut out_raw = std::mem::replace(&mut out, RgbaImage::new(1, 1)).into_raw();
    let out_stride = sw as usize * 4;
    let is_transparent = render == RenderMode::Transparent;

    for sy in 0..dh as usize {
        let dy_i = dy + sy as i64;
        if dy_i < 0 || dy_i >= sh as i64 {
            continue;
        }
        let dy_u = dy_i as usize;
        let src_row_off = sy * dw as usize * 4;
        let dst_row_off = dy_u * out_stride;
        for sx in 0..dw as usize {
            let dx_i = dx + sx as i64;
            if dx_i < 0 || dx_i >= sw as i64 {
                continue;
            }
            let dx_u = dx_i as usize;
            let src_off = src_row_off + sx * 4;
            let dst_off = dst_row_off + dx_u * 4;
            // 原背景模式：fg 的 RGB 通道本就是原图（apply_mask 只改 alpha），
            // 按不透明合成即完整还原原图背景；此前按 alpha 处理会把被抠掉的
            // 背景区域跳过，输出成透明镂空（与「透明底」无异，显示异常的根因）。
            let ca = if render == RenderMode::Original {
                255
            } else {
                scaled_raw[src_off + 3]
            };
            if ca == 0 {
                continue;
            }
            if ca == 255 {
                out_raw[dst_off] = scaled_raw[src_off];
                out_raw[dst_off + 1] = scaled_raw[src_off + 1];
                out_raw[dst_off + 2] = scaled_raw[src_off + 2];
                out_raw[dst_off + 3] = 255;
                continue;
            }
            let ca_i = ca as u32;
            let inv_i = 255 - ca_i;
            out_raw[dst_off] = ((scaled_raw[src_off] as u32 * ca_i + out_raw[dst_off] as u32 * inv_i + 128) / 255) as u8;
            out_raw[dst_off + 1] = ((scaled_raw[src_off + 1] as u32 * ca_i + out_raw[dst_off + 1] as u32 * inv_i + 128) / 255) as u8;
            out_raw[dst_off + 2] = ((scaled_raw[src_off + 2] as u32 * ca_i + out_raw[dst_off + 2] as u32 * inv_i + 128) / 255) as u8;
            out_raw[dst_off + 3] = if is_transparent {
                (ca_i + (out_raw[dst_off + 3] as u32 * inv_i + 128) / 255).min(255) as u8
            } else {
                255
            };
        }
    }
    if let Some(back) = RgbaImage::from_raw(sw, sh, out_raw) {
        out = back;
    }
    out
}

/* ═════════════════ 排版 ═════════════════ */

const LAY_MAX_SIDE: f32 = 4200.0;
const LAY_MAX_PIXELS: f32 = 20_000_000.0;

#[derive(Debug, Clone, Copy)]
pub struct LayoutPlan {
    pub cols: u32,
    pub rows: u32,
    pub rot_tile: bool,
    pub count: u32,
    pub pw: u32,
    pub ph: u32,
    /// 自动排版选中的相纸朝向（landscape / portrait）。参考项目不把朝向写进
    /// meta —— 排版画布的 px 宽高已经体现朝向，这里仅作诊断信息保留。
    #[allow(dead_code)]
    pub dir: &'static str,
}

/// 在 (PW×PH) 相纸上能放下多少个 (iw×ih) 瓦片：横竖两种朝向各算一遍取多者
fn fit_counts(pw: u32, ph: u32, iw: u32, ih: u32, gap: u32, margin: u32) -> (u32, u32, bool, u32) {
    let fit = |w: u32, h: u32| -> (u32, u32) {
        let c = (pw as i64 - 2 * margin as i64 + gap as i64) / (w as i64 + gap as i64);
        let r = (ph as i64 - 2 * margin as i64 + gap as i64) / (h as i64 + gap as i64);
        (c.max(0) as u32, r.max(0) as u32)
    };
    let (c1, r1) = fit(iw, ih);
    let (c2, r2) = fit(ih, iw);
    if c1 * r1 >= c2 * r2 {
        (c1, r1, false, c1 * r1)
    } else {
        (c2, r2, true, c2 * r2)
    }
}

/// 生成排版画布
pub fn make_layout(
    photo: &RgbaImage,
    paper: &IdPaper,
    dpi: u32,
    pw_mm: f32,
    ph_mm: f32,
    cutline: bool,
) -> (RgbaImage, LayoutPlan) {
    make_layout_with_cap(photo, paper, dpi, pw_mm, ph_mm, cutline, LAY_MAX_SIDE, LAY_MAX_PIXELS)
}

/// 带画布上限的排版：`max_side`/`max_pixels` 触发全局等比缩放（纸/瓦片/
/// 间距/边距同乘 sc），瓦片数与布局完全一致，仅分辨率降低 ——
/// 预览用小 cap（省 8 倍绘制量），导出用原常量保画质。
#[allow(clippy::too_many_arguments)] // 纸/瓦片/间距/线全套几何参数，拆 struct 反而绕
pub fn make_layout_with_cap(
    photo: &RgbaImage,
    paper: &IdPaper,
    dpi: u32,
    pw_mm: f32,
    ph_mm: f32,
    cutline: bool,
    max_side: f32,
    max_pixels: f32,
) -> (RgbaImage, LayoutPlan) {
    let iw0 = mm2px(pw_mm, dpi).max(1);
    let ih0 = mm2px(ph_mm, dpi).max(1);
    let gap0 = mm2px(2.0, dpi).max(1);
    let margin0 = mm2px(5.0, dpi);

    let land = [paper.w.max(paper.h), paper.w.min(paper.h)];
    let port = [paper.w.min(paper.h), paper.w.max(paper.h)];

    let mut best: Option<(u32, u32, bool, u32, u32, u32, &'static str)> = None;
    for (mm, dir) in [(land, "landscape"), (port, "portrait")] {
        let pw = mm2px(mm[0], dpi);
        let ph = mm2px(mm[1], dpi);
        let (cols, rows, rot, count) = fit_counts(pw, ph, iw0, ih0, gap0, margin0);
        if best.map(|b| count > b.3).unwrap_or(true) {
            best = Some((cols, rows, rot, count, pw, ph, dir));
        }
    }
    let (cols, rows, rot_tile, _count, pw, ph, dir) =
        best.unwrap_or((0, 0, false, 0, 0, 0, "landscape"));

    let mut sc = (max_side / pw.max(ph) as f32)
        .min((max_pixels / (pw.max(1) as f32 * ph.max(1) as f32)).sqrt())
        .min(1.0);
    if !sc.is_finite() || sc <= 0.0 {
        sc = 1.0;
    }
    let (pw, ph) = ((pw as f32 * sc).round() as u32, (ph as f32 * sc).round() as u32);
    let (iw, ih) = ((iw0 as f32 * sc).round() as u32, (ih0 as f32 * sc).round() as u32);
    let gap = (gap0 as f32 * sc).round().max(1.0) as u32;
    let _margin = (margin0 as f32 * sc).round() as u32;

    let mut canvas = RgbaImage::new(pw.max(1), ph.max(1));
    for px in canvas.pixels_mut() {
        px.0 = [255, 255, 255, 255];
    }

    if iw == 0 || ih == 0 || cols == 0 || rows == 0 {
        return (
            canvas,
            LayoutPlan { cols, rows, rot_tile, count: cols * rows, pw, ph, dir },
        );
    }

    // 单张瓦片：Lanczos3 高质量缩放到瓦片尺寸（原先的逐像素最近邻采样在
    // 尺寸不一致时会产生明显锯齿与细节丢失，是排版画质差的主因），
    // 再按需旋转 90° 多排一张（冲印店要的是数量）。
    // rotate90 与原最近邻映射逐像素等价（已核对 affine.rs 的坐标变换）。
    let scaled = if (iw, ih) != (photo.width(), photo.height()) {
        image::imageops::resize(photo, iw, ih, image::imageops::FilterType::Lanczos3)
    } else {
        photo.clone()
    };
    let cell = if rot_tile { image::imageops::rotate90(&scaled) } else { scaled };
    let cell_w = cell.width();
    let cell_h = cell.height();

    let total_w = cols * cell_w + cols.saturating_sub(1) * gap;
    let total_h = rows * cell_h + rows.saturating_sub(1) * gap;
    let start_x = ((pw as i64 - total_w as i64) / 2).max(0) as u32;
    let start_y = ((ph as i64 - total_h as i64) / 2).max(0) as u32;

    let cell_raw = cell.as_raw();
    let mut canvas_raw = std::mem::replace(&mut canvas, RgbaImage::new(1, 1)).into_raw();
    let cw = cell_w as usize;
    let canvas_stride = pw as usize * 4;

    for r in 0..rows {
        for c in 0..cols {
            let px = start_x + c * (cell_w + gap);
            let py = start_y + r * (cell_h + gap);
            let px_us = px as usize;
            let py_us = py as usize;
            for yy in 0..cell_h as usize {
                let dy = py_us + yy;
                if dy >= ph as usize {
                    break;
                }
                let dx_end = px_us + cw;
                if dx_end > pw as usize {
                    continue;
                }
                let src_off = yy * cw * 4;
                let dst_off = dy * canvas_stride + px_us * 4;
                canvas_raw[dst_off..dst_off + cw * 4]
                    .copy_from_slice(&cell_raw[src_off..src_off + cw * 4]);
            }
            if cutline {
                let c8 = [200u8, 200, 200, 255];
                for xx in 0..cell_w {
                    let x = px + xx;
                    if x < pw {
                        if py < ph {
                            let off = py as usize * canvas_stride + x as usize * 4;
                            canvas_raw[off..off + 4].copy_from_slice(&c8);
                        }
                        if py + cell_h < ph {
                            let off = (py + cell_h) as usize * canvas_stride + x as usize * 4;
                            canvas_raw[off..off + 4].copy_from_slice(&c8);
                        }
                    }
                }
                for yy in 0..cell_h {
                    let y = py + yy;
                    if y < ph {
                        if px < pw {
                            let off = y as usize * canvas_stride + px as usize * 4;
                            canvas_raw[off..off + 4].copy_from_slice(&c8);
                        }
                        if px + cell_w < pw {
                            let off = y as usize * canvas_stride + (px + cell_w) as usize * 4;
                            canvas_raw[off..off + 4].copy_from_slice(&c8);
                        }
                    }
                }
            }
        }
    }

    if let Some(back) = RgbaImage::from_raw(pw, ph, canvas_raw) {
        canvas = back;
    }

    (
        canvas,
        LayoutPlan { cols, rows, rot_tile, count: cols * rows, pw, ph, dir },
    )
}

/// 排版元信息文案
pub fn sheet_text(paper: &IdPaper, plan: &LayoutPlan) -> String {
    if plan.count == 0 {
        return "该相纸放不下当前规格，请换更大相纸".to_string();
    }
    format!(
        "{} · {}列×{}行 = {}张{} · {}×{}px",
        paper.n,
        plan.cols,
        plan.rows,
        plan.count,
        if plan.rot_tile { "（照片旋转 90° 排布）" } else { "" },
        plan.pw,
        plan.ph
    )
}
/* ═════════════════ 单元测试 ═════════════════ */

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    /// 原背景模式：alpha=0 的像素也必须输出 fg 的 RGB（原图背景还原）；
    /// 此前按 alpha 跳过导致透明镂空（与「透明底」无异）——回归锁。
    /// fg 用 295×413（= 一寸 @300DPI 规格），zoom=100 时 1:1 无重采样，
    /// 输出逐像素确定。
    #[test]
    fn compose_original_keeps_rgb_of_transparent_pixels() {
        let (w, h) = (295u32, 413u32);
        let mut fg = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                if x < 148 {
                    // 原「背景」区域：alpha=0，但 RGB 保留原图背景色
                    fg.put_pixel(x, y, image::Rgba([10, 20, 30, 0]));
                } else {
                    fg.put_pixel(x, y, image::Rgba([200, 150, 100, 255]));
                }
            }
        }

        let sp = ID_SIZES[0];
        let out = compose_single(
            &fg,
            &sp,
            [67, 116, 217],
            RenderMode::Original,
            100,
            0,
            300,
        );
        assert_eq!(out.dimensions(), (w, h), "1:1 缩放下输出尺寸应等于规格");
        let left = out.get_pixel(10, 200);
        assert_eq!(
            (left[0], left[1], left[2], left[3]),
            (10, 20, 30, 255),
            "原背景模式丢失了 alpha=0 像素的原 RGB"
        );
        let right = out.get_pixel(200, 200);
        assert_eq!((right[0], right[1], right[2], right[3]), (200, 150, 100, 255));

        // 对照：透明底模式保留镂空语义（左半 alpha=0）
        let out_t = compose_single(
            &fg,
            &sp,
            [67, 116, 217],
            RenderMode::Transparent,
            100,
            0,
            300,
        );
        let tl = out_t.get_pixel(10, 200);
        assert_eq!(tl[3], 0, "透明底模式的镂空语义被破坏");
        let tr = out_t.get_pixel(200, 200);
        assert_eq!((tr[0], tr[1], tr[2], tr[3]), (200, 150, 100, 255));
    }
}