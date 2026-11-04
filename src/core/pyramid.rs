//! 视口分级显示纹理（G2/G3）
//!
//! 问题：`tex_display` 只在长边超过 `TEX_MAX_SIDE`(8192) 时降采样，
//! 一张 4000×3000 的照片每次抠图 / 涂抹提交都会把 48MB RGBA 重新打包成
//! 新的 `Handle`（`from_rgba` 每次都是新 id），触发一次完整的 GPU 上传。
//! 缩放、平移、滚动时 GPU 还要对这张大纹理做 minification，掉帧明显。
//!
//! 策略：把显示层拆成两级，字节在**后台线程**一次性生成，UI 线程只做
//! `Handle::from_rgba`（零拷贝包装）：
//!
//! - `preview`：按视口长边双线性（Triangle）预降采样，`zoom < 1` 时使用；
//!   原图比 cap 还小时不生成，直接复用 `full`，避免多余内存。
//! - `full`：全分辨率（长边仍受 `TEX_MAX_SIDE` 保护），`zoom >= 1`
//!   「放大看细节」时切回，保证 100% 以上的画质。
//!
//! 注意：`Handle` 内容变化时必须换新 id，这是 iced 的正确用法；现有
//! `ensure_image_alloc` 已通过同步预上传消除了首帧闪跳。本模块解决的不是
//! 「换不换 id」，而是「每次上传多少字节」——缩小视图时从 48MB 降到数 MB。

use image::{imageops::FilterType, RgbaImage};

use crate::core::image::TEX_MAX_SIDE;

/// 预览层长边上限的钳制区间（逻辑像素）
pub const PREVIEW_MIN: u32 = 768;
pub const PREVIEW_MAX: u32 = 2048;

/// 预览层目标长边：视口长边 ×1.5（留 DPI 缩放与轻微放大的余量），
/// 钳到 `[768, 2048]`，避免小窗口上传大纹理、大窗口画质过糊。
pub fn preview_cap(viewport_long: f32) -> u32 {
    let v = viewport_long * 1.5;
    if !v.is_finite() {
        return PREVIEW_MIN;
    }
    v.round().clamp(PREVIEW_MIN as f32, PREVIEW_MAX as f32) as u32
}

/// 一级显示字节（可直接喂给 `Handle::from_rgba`）
#[derive(Debug, Clone)]
pub struct LevelBytes {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
}

/// 两级显示字节。`preview` 为 `None` 表示原图已足够小，无需额外层。
#[derive(Debug, Clone)]
pub struct PyramidBytes {
    pub preview: Option<LevelBytes>,
    pub full: LevelBytes,
}

fn level(w: u32, h: u32, bytes: Vec<u8>) -> LevelBytes {
    LevelBytes { w, h, bytes }
}

/// 在后台线程生成两级字节。`viewport_long` 取画布可视区的长边（逻辑像素）。
pub fn build(img: &RgbaImage, viewport_long: f32) -> PyramidBytes {
    let (w, h) = (img.width(), img.height());
    let long = w.max(h);

    // 全分辨率层：仅超 wgpu 纹理上限时才降采样（Lanczos3 保画质）
    let full = if long <= TEX_MAX_SIDE {
        level(w, h, img.as_raw().clone())
    } else {
        let (nw, nh) = fit_side(w, h, TEX_MAX_SIDE);
        level(
            nw,
            nh,
            image::imageops::resize(img, nw, nh, FilterType::Lanczos3).into_raw(),
        )
    };

    // 预览层：Triangle ≈ 双线性，预降采样后 GPU 每帧上传的数据量按面积锐减
    let cap = preview_cap(viewport_long);
    let preview = if long > cap {
        let (nw, nh) = fit_side(w, h, cap);
        Some(level(
            nw,
            nh,
            image::imageops::resize(img, nw, nh, FilterType::Triangle).into_raw(),
        ))
    } else {
        None
    };

    PyramidBytes { preview, full }
}

/// 等比缩放到长边 `cap`，不放大；返回尺寸至少 1×1。
fn fit_side(w: u32, h: u32, cap: u32) -> (u32, u32) {
    let long = w.max(h).max(1);
    if long <= cap {
        return (w.max(1), h.max(1));
    }
    let s = cap as f64 / long as f64;
    (
        ((w as f64 * s).round() as u32).max(1),
        ((h as f64 * s).round() as u32).max(1),
    )
}

/// 选层纯函数：`zoom >= 1` 或没有预览层时用全分辨率，否则用预览层。
/// 抽成纯函数便于单测，也保证 view 层不含状态写入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Preview,
    Full,
}

pub fn select(zoom: f32, has_preview: bool) -> Level {
    if has_preview && zoom < 1.0 {
        Level::Preview
    } else {
        Level::Full
    }
}

impl PyramidBytes {
    /// 取某一级字节；预览层缺失时回退全分辨率层（保证任何缩放下都不白屏）
    pub fn level(&self, which: Level) -> &LevelBytes {
        match which {
            Level::Preview => self.preview.as_ref().unwrap_or(&self.full),
            Level::Full => &self.full,
        }
    }

    /// 一次性消费字节，生成两级 iced 句柄（move 而非 clone，UI 线程零计算）。
    /// 返回 `(full, preview)`；`preview` 为 `None` 时视图层回退到 `full`。
    pub fn into_handles(
        self,
    ) -> (
        iced::widget::image::Handle,
        Option<iced::widget::image::Handle>,
    ) {
        let full =
            iced::widget::image::Handle::from_rgba(self.full.w, self.full.h, self.full.bytes);
        let preview = self
            .preview
            .map(|l| iced::widget::image::Handle::from_rgba(l.w, l.h, l.bytes));
        (full, preview)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_is_clamped() {
        assert_eq!(preview_cap(100.0), PREVIEW_MIN);
        assert_eq!(preview_cap(900.0), 1350);
        assert_eq!(preview_cap(4000.0), PREVIEW_MAX);
        assert_eq!(preview_cap(f32::NAN), PREVIEW_MIN); // 非有限值不 panic
        assert_eq!(preview_cap(f32::INFINITY), PREVIEW_MIN);
    }

    #[test]
    fn small_image_has_no_preview() {
        let img = RgbaImage::new(320, 240);
        let p = build(&img, 900.0);
        assert!(p.preview.is_none());
        assert_eq!((p.full.w, p.full.h), (320, 240));
        assert_eq!(p.full.bytes.len(), 320 * 240 * 4);
    }

    #[test]
    fn large_image_preview_is_downscaled_bilinearly() {
        let img = RgbaImage::new(4000, 3000);
        let p = build(&img, 900.0); // cap = 1350
        let pv = p.preview.as_ref().expect("preview should exist");
        assert_eq!((pv.w, pv.h), (1350, 1013));
        assert_eq!(pv.bytes.len(), 1350 * 1013 * 4);
        // full 保持原分辨率
        assert_eq!((p.full.w, p.full.h), (4000, 3000));
        // 预览层数据量约为全分辨率的 1/9
        assert!(pv.bytes.len() * 8 < p.full.bytes.len());
    }

    #[test]
    fn oversized_full_is_capped_at_texture_limit() {
        let img = RgbaImage::new(9000, 2);
        let p = build(&img, 900.0);
        assert_eq!(p.full.w, TEX_MAX_SIDE);
        assert!(p.full.h >= 1);
        assert!(p.preview.is_some());
    }

    #[test]
    fn fit_side_keeps_aspect_and_never_zero() {
        assert_eq!(fit_side(4000, 3000, 2000), (2000, 1500));
        assert_eq!(fit_side(100, 50, 2000), (100, 50));
        assert_eq!(fit_side(1, 9000, 100), (1, 100));
    }

    #[test]
    fn select_uses_preview_only_below_100_percent() {
        assert_eq!(select(0.25, true), Level::Preview);
        assert_eq!(select(0.99, true), Level::Preview);
        assert_eq!(select(1.0, true), Level::Full);
        assert_eq!(select(4.0, true), Level::Full);
        assert_eq!(select(0.25, false), Level::Full);
    }

    #[test]
    fn level_falls_back_to_full_and_into_handles_is_zero_copy() {
        let img = RgbaImage::new(16, 8);
        let p = build(&img, 900.0);
        assert_eq!(p.level(Level::Preview).w, 16);
        assert_eq!(p.level(Level::Preview).h, 8);
        let (full, preview) = p.into_handles();
        let _ = full; // Handle 构造成功（不依赖 GPU，可在单测里建）
        assert!(preview.is_none());
    }
}
