//! 图像 IO 与像素变换：解码 / 旋转 / 裁剪 / 缩放 / 蒙版合成 / 编码落盘

use image::{DynamicImage, ImageDecoder, ImageFormat, RgbaImage};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImgError {
    #[error("读取文件失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("解码失败（损坏或不支持的格式）：{0}")]
    Decode(#[from] image::ImageError),
    #[error("编码失败：{0}")]
    Encode(String),
    #[error("尺寸非法：{0}×{1}")]
    BadSize(u32, u32),
}

pub type Result<T> = std::result::Result<T, ImgError>;

#[derive(Debug, Clone)]
pub struct LoadedImage {
    pub rgba: RgbaImage,
    pub path: Option<PathBuf>,
    /// 去扩展名的文件名，导出时用作前缀
    pub stem: String,
}

/// 批量队列缩略图与文件选择对话框共用的扩展名白名单（小写、不含点）。
/// 与本项目 `image` crate 实际启用的解码格式保持一致；表外文件仍可进队列，
/// 但只显示「不支持预览」占位，不发起后台解码。
pub const SUPPORTED_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp"];

/// 缩略图长边上限（逻辑像素）：UI 显示约 40px，留 2x DPI 余量。
pub const THUMB_LONG: u32 = 96;

/// 该文件是否支持生成缩略图（按扩展名，大小写不敏感）
pub fn supports_thumb(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED_IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 等比生成缩略图 RGBA 字节；长边不超过 `long`，小图原样返回。
/// 后台线程调用，只回传纯字节；UI 线程仅做 `Handle::from_rgba` 包装（O(1)）。
pub fn thumb_bytes(img: &RgbaImage, long: u32) -> (u32, u32, Vec<u8>) {
    let (w, h) = (img.width(), img.height());
    let cap = long.max(1);
    let src_long = w.max(h).max(1);
    if src_long <= cap {
        return (w, h, img.as_raw().clone());
    }
    let sc = cap as f32 / src_long as f32;
    let tw = ((w as f32 * sc).round() as u32).max(1);
    let th = ((h as f32 * sc).round() as u32).max(1);
    let small = image::imageops::resize(img, tw, th, image::imageops::FilterType::Triangle);
    (tw, th, small.into_raw())
}

/// 从磁盘解码一张图
pub fn load_file(path: &Path) -> Result<LoadedImage> {
    // EXIF 方向：手机竖拍照片靠 EXIF 标记旋转，decode() 不会自动应用 ——
    // 必须显式读 orientation 再 apply，否则竖拍图全部横躺。
    // JPEG/TIFF/WebP 有真实 EXIF；PNG 等格式默认 NoTransforms。
    let reader = image::ImageReader::open(path)?
        .with_guessed_format()
        .map_err(|e| ImgError::Encode(e.to_string()))?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Err(ImgError::BadSize(rgba.width(), rgba.height()));
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());
    Ok(LoadedImage { rgba, path: Some(path.to_path_buf()), stem })
}

/// 从内存字节解码一张图（用于内嵌资源）
pub fn load_bytes(data: &[u8], name: &str) -> Result<LoadedImage> {
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| ImgError::Encode(e.to_string()))?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    let rgba = img.into_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 {
        return Err(ImgError::BadSize(rgba.width(), rgba.height()));
    }
    let stem = std::path::Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());
    Ok(LoadedImage { rgba, path: None, stem })
}

/* ═════════════════ 蒙版合成 ═════════════════ */

/// GPU 纹理安全边长（wgpu 默认 max texture 2D 维度 8192）。
/// 超过上限的图上传纹理会直接失败（白屏/崩溃）。
pub const TEX_MAX_SIDE: u32 = 8192;

/// 显示句柄的安全字节：长边超限时降采样，否则原样克隆。
/// 只用于显示纹理；处理与导出仍用全分辨率原图。
pub struct TexDisplay {
    pub w: u32,
    pub h: u32,
    pub bytes: Vec<u8>,
}

pub fn tex_display(img: &RgbaImage) -> TexDisplay {
    let long = img.width().max(img.height());
    if long <= TEX_MAX_SIDE {
        return TexDisplay { w: img.width(), h: img.height(), bytes: img.as_raw().clone() };
    }
    let scale = TEX_MAX_SIDE as f32 / long as f32;
    let w = ((img.width() as f32 * scale).round() as u32).max(1);
    let h = ((img.height() as f32 * scale).round() as u32).max(1);
    let resized = image::imageops::resize(img, w, h, image::imageops::FilterType::Lanczos3);
    TexDisplay { w, h, bytes: resized.into_raw() }
}

/// 把蒙版写进 alpha 通道，得到带透明的前景图
pub fn apply_mask(src: &RgbaImage, mask: &[u8]) -> RgbaImage {
    let (w, h) = (src.width(), src.height());
    let src_raw = src.as_raw();
    let mut out_raw = src_raw.clone();
    for i in 0..(w as usize * h as usize) {
        out_raw[i * 4 + 3] = mask[i];
    }
    RgbaImage::from_raw(w, h, out_raw).unwrap_or_else(|| RgbaImage::new(w, h))
}

/// 在指定底色上压平（导出 / 预览用）；None 表示保留透明
pub fn flatten(fg: &RgbaImage, bg: Option<[u8; 3]>) -> RgbaImage {
    let (w, h) = (fg.width(), fg.height());
    let src_raw = fg.as_raw();
    let mut out_raw = src_raw.clone();
    if let Some(c) = bg {
        for i in 0..(w as usize * h as usize) {
            let off = i * 4;
            let a = src_raw[off + 3] as u32;
            if a == 255 {
                out_raw[off + 3] = 255;
                continue;
            }
            if a == 0 {
                out_raw[off] = c[0];
                out_raw[off + 1] = c[1];
                out_raw[off + 2] = c[2];
                out_raw[off + 3] = 255;
                continue;
            }
            let inv = 255 - a;
            out_raw[off] = ((src_raw[off] as u32 * a + c[0] as u32 * inv + 128) / 255) as u8;
            out_raw[off + 1] = ((src_raw[off + 1] as u32 * a + c[1] as u32 * inv + 128) / 255) as u8;
            out_raw[off + 2] = ((src_raw[off + 2] as u32 * a + c[2] as u32 * inv + 128) / 255) as u8;
            out_raw[off + 3] = 255;
        }
    }
    RgbaImage::from_raw(w, h, out_raw).unwrap_or_else(|| RgbaImage::new(w, h))
}

/* ═════════════════ 几何变换 ═════════════════ */

pub fn rotate90(img: &RgbaImage, cw: bool) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let src_raw = img.as_raw();
    let mut out_raw = vec![0u8; src_raw.len()];
    let src_stride = w as usize * 4;
    let out_stride = h as usize * 4;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let src_off = y * src_stride + x * 4;
            let (dx, dy) = if cw {
                (h as usize - 1 - y, x)
            } else {
                (y, w as usize - 1 - x)
            };
            let dst_off = dy * out_stride + dx * 4;
            out_raw[dst_off..dst_off + 4].copy_from_slice(&src_raw[src_off..src_off + 4]);
        }
    }
    RgbaImage::from_raw(h, w, out_raw).unwrap_or_else(|| RgbaImage::new(h, w))
}

/// 任意角度旋转（双线性，画布按外接矩形扩展，空白处透明）
pub fn rotate_deg(img: &RgbaImage, deg: f32) -> RgbaImage {
    if deg.abs() < 1e-3 {
        return img.clone();
    }
    let (w, h) = (img.width() as f32, img.height() as f32);
    let rad = deg.to_radians();
    let (cos, sin) = (rad.cos(), rad.sin());
    let nw = (w * cos.abs() + h * sin.abs()).ceil().max(1.0) as u32;
    let nh = (w * sin.abs() + h * cos.abs()).ceil().max(1.0) as u32;
    let mut out = RgbaImage::new(nw, nh);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (ncx, ncy) = (nw as f32 / 2.0, nh as f32 / 2.0);
    for y in 0..nh {
        for x in 0..nw {
            let dx = x as f32 - ncx;
            let dy = y as f32 - ncy;
            let sx = dx * cos + dy * sin + cx;
            let sy = -dx * sin + dy * cos + cy;
            if sx >= 0.0 && sx < w && sy >= 0.0 && sy < h {
                let p = sample_bilinear(img, sx, sy);
                out.put_pixel(x, y, image::Rgba(p));
            }
        }
    }
    out
}


fn sample_bilinear(img: &RgbaImage, x: f32, y: f32) -> [u8; 4] {
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let x1 = (x0 + 1).min(img.width() as i64 - 1).max(0);
    let y1 = (y0 + 1).min(img.height() as i64 - 1).max(0);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let g = |xx: i64, yy: i64| img.get_pixel(xx as u32, yy as u32).0;
    let a = g(x0, y0);
    let b = g(x1, y0);
    let c = g(x0, y1);
    let d = g(x1, y1);
    let mut o = [0u8; 4];
    for i in 0..4 {
        let t = a[i] as f32 + (b[i] as f32 - a[i] as f32) * fx;
        let u = c[i] as f32 + (d[i] as f32 - c[i] as f32) * fx;
        o[i] = (t + (u - t) * fy).round() as u8;
    }
    o
}

pub fn crop(img: &RgbaImage, x: u32, y: u32, w: u32, h: u32) -> RgbaImage {
    let w = w.min(img.width().saturating_sub(x));
    let h = h.min(img.height().saturating_sub(y));
    let w = w.max(1);
    let h = h.max(1);
    
    // 优化：直接操作原始字节数组，按行拷贝，避免逐像素 get_pixel/put_pixel
    let src_raw = img.as_raw();
    let src_stride = img.width() as usize * 4;
    let mut out_raw = vec![0u8; w as usize * h as usize * 4];
    let dst_stride = w as usize * 4;
    
    for row in 0..h as usize {
        let src_off = (y as usize + row) * src_stride + x as usize * 4;
        let dst_off = row * dst_stride;
        out_raw[dst_off..dst_off + dst_stride]
            .copy_from_slice(&src_raw[src_off..src_off + dst_stride]);
    }
    
    RgbaImage::from_raw(w, h, out_raw).unwrap_or_else(|| RgbaImage::new(w, h))
}

/// 精确尺寸缩放（Lanczos3）：输出**严格**为 w×h。
///
/// 不要用 `DynamicImage::resize` 代替：它是「等比适配到不超过 w×h」，当目标
/// 宽高比与源图不完全一致时输出会小于请求尺寸（例 301×200 → 请求 73×48，
/// 实际 72×48）。方图导出的合成循环按 dw/dh 全量索引，尺寸不足会 panic。
/// 直接走 `imageops::resize` 还省掉一次全图 clone。
pub fn resize_exact(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    image::imageops::resize(
        img,
        w.max(1),
        h.max(1),
        image::imageops::FilterType::Lanczos3,
    )
}

/* ═════════════════ 导出 ═════════════════ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Jpg,
    Webp,
}

impl ExportFormat {
    pub fn label(&self) -> &'static str {
        match self {
            ExportFormat::Png => "PNG（保留透明通道）",
            ExportFormat::Jpg => "JPG（白底 · 体积最小）",
            ExportFormat::Webp => "WEBP",
        }
    }
    pub fn ext(&self) -> &'static str {
        match self {
            ExportFormat::Png => "png",
            ExportFormat::Jpg => "jpg",
            ExportFormat::Webp => "webp",
        }
    }
    pub const fn all() -> &'static [ExportFormat] {
        &[ExportFormat::Png, ExportFormat::Jpg, ExportFormat::Webp]
    }
    fn image_format(&self) -> ImageFormat {
        match self {
            ExportFormat::Png => ImageFormat::Png,
            ExportFormat::Jpg => ImageFormat::Jpeg,
            ExportFormat::Webp => ImageFormat::WebP,
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// 导出尺寸：1 = 原始尺寸，其余为方画布边长
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSize {
    Original,
    Square(u32),
}

impl ExportSize {
    pub fn label(&self) -> String {
        match self {
            ExportSize::Original => "原始尺寸".to_string(),
            ExportSize::Square(800) => "800 × 800（淘宝主图）".to_string(),
            ExportSize::Square(t) => format!("{t} × {t}"),
        }
    }
    pub fn to_u32(self) -> u32 {
        match self {
            ExportSize::Original => 1,
            ExportSize::Square(t) => t,
        }
    }
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => ExportSize::Original,
            t => ExportSize::Square(t),
        }
    }
    pub const fn all() -> &'static [ExportSize] {
        &[
            ExportSize::Original,
            ExportSize::Square(800),
            ExportSize::Square(1000),
            ExportSize::Square(1200),
            ExportSize::Square(2048),
        ]
    }
}

impl std::fmt::Display for ExportSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// 按导出尺寸规整：原始尺寸直接返回；否则等比缩放（0.92 留边）居中放进方画布
pub fn fit_export(fg: &RgbaImage, size: ExportSize, bg: Option<[u8; 3]>) -> RgbaImage {
    match size {
        ExportSize::Original => flatten(fg, bg),
        ExportSize::Square(t) => {
            let (w, h) = (fg.width() as f32, fg.height() as f32);
            let s = (t as f32 / w).min(t as f32 / h) * 0.92;
            let dw = (w * s).max(1.0) as u32;
            let dh = (h * s).max(1.0) as u32;
            // 必须严格尺寸：下方合成循环按 dw/dh 全量 get_pixel
            let small = resize_exact(fg, dw, dh);
            let mut canvas = RgbaImage::new(t, t);
            if let Some(c) = bg {
                for px in canvas.pixels_mut() {
                    px.0 = [c[0], c[1], c[2], 255];
                }
            }
            let dx = (t - dw) / 2;
            let dy = (t - dh) / 2;
            for y in 0..dh {
                for x in 0..dw {
                    let p = small.get_pixel(x, y).0;
                    let a = p[3] as f32 / 255.0;
                    let dst = canvas.get_pixel_mut(dx + x, dy + y);
                    let inv = 1.0 - a;
                    dst.0 = [
                        (p[0] as f32 * a + dst.0[0] as f32 * inv) as u8,
                        (p[1] as f32 * a + dst.0[1] as f32 * inv) as u8,
                        (p[2] as f32 * a + dst.0[2] as f32 * inv) as u8,
                        ((a + dst.0[3] as f32 / 255.0 * inv) * 255.0) as u8,
                    ];
                }
            }
            canvas
        }
    }
}

/// 生成导出默认文件名
pub fn export_name(stem: &str, fmt: ExportFormat) -> String {
    format!("{stem}_nobg.{}", fmt.ext())
}

/// JPG 导出质量（image 默认 write_to 是 75，证件照冲印要求 95+ 才无明显块效应）
const JPEG_EXPORT_QUALITY: u8 = 95;

/// 编码到内存（证件照导出 / 批量共用的另一条入口）
///
/// 画质策略：
/// - JPG：显式 `JpegEncoder::new_with_quality(95)`，替代 `write_to` 的默认 75
/// - PNG：无损，默认编码即可
/// - WebP：image 0.25 内置编码器只有无损模式（`new_lossless`），画质无损
pub fn save_to_memory(
    fg: &RgbaImage,
    fmt: ExportFormat,
    bg: Option<[u8; 3]>,
) -> Result<Vec<u8>> {
    let bg = if fmt == ExportFormat::Jpg { Some([255, 255, 255]) } else { bg };
    let flat = flatten(fg, bg);
    match fmt {
        ExportFormat::Jpg => {
            // JpegEncoder 只收 RGB8/灰度；flatten 后 alpha 全 255，安全转 RGB
            let rgb = DynamicImage::ImageRgba8(flat).to_rgb8();
            let mut buf = Vec::new();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut buf,
                JPEG_EXPORT_QUALITY,
            );
            enc.encode_image(&rgb)
                .map_err(|e| ImgError::Encode(e.to_string()))?;
            Ok(buf)
        }
        _ => {
            let mut buf = std::io::Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(flat)
                .write_to(&mut buf, fmt.image_format())
                .map_err(|e| ImgError::Encode(e.to_string()))?;
            Ok(buf.into_inner())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::from_fn(w, h, |_, _| image::Rgba(rgba))
    }

    #[test]
    fn thumb_bytes_keeps_aspect_and_caps_long_side() {
        let src = solid(400, 200, [1, 2, 3, 255]);
        let (w, h, bytes) = thumb_bytes(&src, THUMB_LONG);
        assert_eq!((w, h), (96, 48));
        assert_eq!(bytes.len(), (w * h * 4) as usize);
    }

    #[test]
    fn thumb_bytes_small_image_is_untouched() {
        let src = solid(30, 20, [9, 8, 7, 255]);
        let (w, h, bytes) = thumb_bytes(&src, THUMB_LONG);
        assert_eq!((w, h), (30, 20));
        assert_eq!(bytes, *src.as_raw());
    }

    #[test]
    fn thumb_bytes_never_produces_zero_side() {
        let src = solid(1000, 1, [1, 1, 1, 255]);
        let (w, h, _) = thumb_bytes(&src, THUMB_LONG);
        assert_eq!((w, h), (96, 1));
    }

    #[test]
    fn supports_thumb_matches_whitelist_case_insensitively() {
        assert!(supports_thumb(std::path::Path::new("a.PNG")));
        assert!(supports_thumb(std::path::Path::new("a.JpEg")));
        assert!(supports_thumb(std::path::Path::new("a.webp")));
        assert!(!supports_thumb(std::path::Path::new("a.txt")));
        assert!(!supports_thumb(std::path::Path::new("noext")));
    }

    /// 精确缩放的输出必须严格等于请求尺寸（DynamicImage::resize 会等比收缩）
    #[test]
    fn resize_exact_keeps_requested_dimensions() {
        let src = solid(301, 200, [255, 0, 0, 255]);
        let out = resize_exact(&src, 73, 48);
        assert_eq!((out.width(), out.height()), (73, 48));
    }

    /// 回归：非整数缩放比例下方图导出不得越界 panic，前景区域严格为 dw×dh
    #[test]
    fn fit_export_square_no_panic_on_fractional_scale() {
        let src = solid(301, 200, [255, 0, 0, 255]);
        let t = 80u32;
        let (w, h) = (src.width() as f32, src.height() as f32);
        let s = (t as f32 / w).min(t as f32 / h) * 0.92;
        let dw = (w * s).max(1.0) as u32;
        let dh = (h * s).max(1.0) as u32;

        let out = fit_export(&src, ExportSize::Square(t), Some([0, 0, 255]));
        assert_eq!((out.width(), out.height()), (t, t));
        let red = out.pixels().filter(|p| p.0[0] > 200 && p.0[2] < 50).count();
        assert_eq!(red, (dw * dh) as usize, "前景区域必须严格为 dw×dh");
    }

    #[test]
    fn fit_export_square_tiny_canvas() {
        let src = solid(7, 5, [10, 200, 30, 128]);
        let out = fit_export(&src, ExportSize::Square(1), None);
        assert_eq!((out.width(), out.height()), (1, 1));
    }
}
