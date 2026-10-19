//! Export: crop, resize, convert and re-encode one file or many, with metadata handling.

use crate::{decode, formats, meta_full};
use image::codecs::{jpeg::JpegEncoder, png, webp::WebPEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageEncoder, ImageFormat};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ExportMode {
    Modify,
    Copy,
    Move,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ApplyTo {
    Current,
    All,
    Flagged,
    Selected,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum OutFormat {
    Match,
    Jpeg,
    Png,
    Webp,
    Gif,
    Bmp,
    Tiff,
}

impl OutFormat {
    pub const ALL: [OutFormat; 7] = [Self::Match, Self::Jpeg, Self::Png, Self::Webp, Self::Gif, Self::Bmp, Self::Tiff];
    pub fn label(self) -> &'static str {
        match self {
            Self::Match => "Как у исходного",
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Webp => "WebP (без потерь)",
            Self::Gif => "GIF",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png | Self::Match => "png",
            Self::Webp => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tif",
        }
    }
    pub fn resolve(self, src: &Path) -> Self {
        if self != Self::Match {
            return self;
        }
        match formats::ext_of(src).as_str() {
            "jpg" | "jpeg" | "jpe" | "jfif" => Self::Jpeg,
            "webp" => Self::Webp,
            "gif" => Self::Gif,
            "bmp" | "dib" => Self::Bmp,
            "tif" | "tiff" => Self::Tiff,
            _ => Self::Png,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum CropMode {
    Visible,
    Select,
    Ratio,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ResizeBy {
    Pixels,
    Scale,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ScaleMode {
    Fit,
    FillCrop,
    FillOverflow,
    Stretch,
    FitNoUpscale,
}

impl ScaleMode {
    pub const ALL: [ScaleMode; 5] = [Self::Fit, Self::FillCrop, Self::FillOverflow, Self::Stretch, Self::FitNoUpscale];
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Вписать",
            Self::FillCrop => "Заполнить, обрезать лишнее",
            Self::FillOverflow => "Заполнить, с выходом за край",
            Self::Stretch => "Растянуть",
            Self::FitNoUpscale => "Вписать, но не больше 1:1",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Interp {
    Nearest,
    Bilinear,
    Cubic,
    Lanczos,
}

impl Interp {
    pub const ALL: [Interp; 4] = [Self::Lanczos, Self::Cubic, Self::Bilinear, Self::Nearest];
    pub fn label(self) -> &'static str {
        match self {
            Self::Nearest => "Ближайший пиксель",
            Self::Bilinear => "Билинейная",
            Self::Cubic => "Бикубическая",
            Self::Lanczos => "Ланцош (высокое качество)",
        }
    }
    fn filter(self) -> FilterType {
        match self {
            Self::Nearest => FilterType::Nearest,
            Self::Bilinear => FilterType::Triangle,
            Self::Cubic => FilterType::CatmullRom,
            Self::Lanczos => FilterType::Lanczos3,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum KeepMeta {
    None,
    Common,
    All,
}

impl KeepMeta {
    pub const ALL: [KeepMeta; 3] = [Self::None, Self::Common, Self::All];
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Не сохранять",
            Self::Common => "Основные свойства",
            Self::All => "Все",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PngCompression {
    Auto,
    None,
    Max,
    Sub,
    Up,
    Average,
    Paeth,
}

impl PngCompression {
    pub const ALL: [PngCompression; 7] = [Self::Auto, Self::None, Self::Max, Self::Sub, Self::Up, Self::Average, Self::Paeth];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Автоматически",
            Self::None => "Без сжатия",
            Self::Max => "Максимальное (адаптивный фильтр)",
            Self::Sub => "Фильтр Sub",
            Self::Up => "Фильтр Up",
            Self::Average => "Фильтр Average",
            Self::Paeth => "Фильтр Paeth",
        }
    }
    fn params(self) -> (png::CompressionType, png::FilterType) {
        use png::{CompressionType as C, FilterType as F};
        match self {
            Self::Auto => (C::Default, F::Adaptive),
            Self::None => (C::Uncompressed, F::NoFilter),
            Self::Max => (C::Best, F::Adaptive),
            Self::Sub => (C::Default, F::Sub),
            Self::Up => (C::Default, F::Up),
            Self::Average => (C::Default, F::Avg),
            Self::Paeth => (C::Default, F::Paeth),
        }
    }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub mode: ExportMode,
    pub apply: ApplyTo,
    pub format: OutFormat,
    pub crop_on: bool,
    pub crop: CropMode,
    pub ratio: (u32, u32),
    pub resize_on: bool,
    pub resize_by: ResizeBy,
    /// Target sizes in pixels; 0 means "auto" (keep proportions).
    pub sizes: Vec<(u32, u32)>,
    pub scale_pct: f32,
    pub scale_mode: ScaleMode,
    pub interp: Interp,
    /// Cut out the main subject with the neural network (transparent background).
    pub remove_bg: bool,
    pub keep_meta: KeepMeta,
    pub bg_on: bool,
    pub bg: [u8; 3],
    pub prefer_alpha: bool,
    pub png: PngCompression,
    pub jpeg_quality: u8,
    pub dither: bool,
    pub name_template: String,
    pub open_after: bool,
    pub reuse_folder: bool,
    pub folder: Option<PathBuf>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            mode: ExportMode::Copy,
            apply: ApplyTo::Current,
            format: OutFormat::Match,
            crop_on: false,
            crop: CropMode::Visible,
            ratio: (3, 4),
            resize_on: false,
            resize_by: ResizeBy::Pixels,
            sizes: vec![(0, 0)],
            scale_pct: 50.0,
            scale_mode: ScaleMode::Fit,
            interp: Interp::Lanczos,
            remove_bg: false,
            keep_meta: KeepMeta::Common,
            bg_on: false,
            bg: [255, 255, 255],
            prefer_alpha: false,
            png: PngCompression::Auto,
            jpeg_quality: 92,
            dither: true,
            name_template: "{name}".into(),
            open_after: false,
            reuse_folder: false,
            folder: None,
        }
    }
}

impl ExportSettings {
    /// Output format for `src`. With background removal, "as the source" keeps
    /// transparency: JPEG and BMP sources are written as PNG.
    pub fn out_format(&self, src: &Path) -> OutFormat {
        let f = self.format.resolve(src);
        if self.remove_bg && self.format == OutFormat::Match && matches!(f, OutFormat::Jpeg | OutFormat::Bmp) { OutFormat::Png } else { f }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ExportProfile {
    pub name: String,
    pub settings: ExportSettings,
}

/// What to export from one source file.
#[derive(Clone)]
pub struct Job {
    pub path: PathBuf,
    /// View rotation (quarter turns) and mirroring, applied for the file on screen.
    pub rot: u8,
    pub flip: bool,
    /// Crop rectangle in normalized coordinates of the oriented image: [x0, y0, x1, y1].
    pub crop: Option<[f32; 4]>,
}

pub struct Encoded {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
    /// Added to the file name when several sizes are exported.
    pub suffix: String,
    pub pixels: DynamicImage,
}

/// Decodes, transforms and encodes `job` once per requested size.
pub fn render(job: &Job, s: &ExportSettings) -> Result<Vec<Encoded>, String> {
    let mut img = decode::load_dynamic(&job.path)?;
    img = match job.rot % 4 {
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => img,
    };
    if job.flip {
        img = img.fliph();
    }
    if s.crop_on {
        let (w, h) = img.dimensions();
        let rect = match s.crop {
            CropMode::Ratio => Some(ratio_rect(w, h, s.ratio)),
            _ => job.crop.map(|c| {
                let x0 = (c[0].clamp(0.0, 1.0) * w as f32).round() as u32;
                let y0 = (c[1].clamp(0.0, 1.0) * h as f32).round() as u32;
                let x1 = (c[2].clamp(0.0, 1.0) * w as f32).round() as u32;
                let y1 = (c[3].clamp(0.0, 1.0) * h as f32).round() as u32;
                (x0.min(x1), y0.min(y1), x1.abs_diff(x0).max(1), y1.abs_diff(y0).max(1))
            }),
        };
        if let Some((x, y, cw, ch)) = rect {
            img = img.crop_imm(x, y, cw.min(w - x.min(w - 1)), ch.min(h - y.min(h - 1)));
        }
    }
    if s.remove_bg {
        img = crate::cutout::remove(&img)?;
    }

    let targets: Vec<Option<(u32, u32)>> = if !s.resize_on {
        vec![None]
    } else if s.resize_by == ResizeBy::Scale {
        let (w, h) = img.dimensions();
        let k = (s.scale_pct / 100.0).max(0.001);
        vec![Some((((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1)))]
    } else {
        s.sizes.iter().map(|&(w, h)| (w > 0 || h > 0).then_some((w, h))).collect()
    };
    let multi = targets.len() > 1;
    let fmt = s.out_format(&job.path);
    let src_bytes = std::fs::read(&job.path).unwrap_or_default();
    let mut out = Vec::new();
    for t in targets {
        let resized = match t {
            None => img.clone(),
            Some((tw, th)) => resize(&img, tw, th, s.scale_mode, s.interp.filter()),
        };
        let suffix = if multi { format!("_{}x{}", resized.width(), resized.height()) } else { String::new() };
        let prepared = prepare_pixels(resized, fmt, s);
        let bytes = encode(&prepared, fmt, s, &src_bytes, &job.path)?;
        out.push(Encoded { width: prepared.width(), height: prepared.height(), bytes, ext: fmt.ext(), suffix, pixels: prepared });
    }
    Ok(out)
}

pub fn ratio_rect(w: u32, h: u32, (rw, rh): (u32, u32)) -> (u32, u32, u32, u32) {
    let (rw, rh) = (rw.max(1) as f64, rh.max(1) as f64);
    let (fw, fh) = (w as f64, h as f64);
    if fw / fh > rw / rh {
        let nw = (fh * rw / rh).round() as u32;
        ((w - nw) / 2, 0, nw.max(1), h)
    } else {
        let nh = (fw * rh / rw).round() as u32;
        (0, (h - nh) / 2, w, nh.max(1))
    }
}

fn resize(img: &DynamicImage, tw: u32, th: u32, mode: ScaleMode, f: FilterType) -> DynamicImage {
    let (w, h) = (img.width() as f64, img.height() as f64);
    // A missing side follows the aspect ratio of the image.
    let (tw, th) = match (tw, th) {
        (0, th) => (((w * th as f64 / h).round() as u32).max(1), th),
        (tw, 0) => (tw, ((h * tw as f64 / w).round() as u32).max(1)),
        t => t,
    };
    let fit = (tw as f64 / w).min(th as f64 / h);
    let cover = (tw as f64 / w).max(th as f64 / h);
    let scaled = |k: f64| img.resize_exact(((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1), f);
    match mode {
        ScaleMode::Fit => scaled(fit),
        ScaleMode::FitNoUpscale => {
            if fit >= 1.0 { img.clone() } else { scaled(fit) }
        }
        ScaleMode::FillCrop => img.resize_to_fill(tw, th, f),
        ScaleMode::FillOverflow => scaled(cover),
        ScaleMode::Stretch => img.resize_exact(tw, th, f),
    }
}

fn prepare_pixels(img: DynamicImage, fmt: OutFormat, s: &ExportSettings) -> DynamicImage {
    let alpha = img.color().has_alpha();
    let flatten = |img: &DynamicImage, bg: [u8; 3]| -> DynamicImage {
        let mut rgba = img.to_rgba8();
        for p in rgba.pixels_mut() {
            let a = p[3] as u32;
            for c in 0..3 {
                p[c] = ((p[c] as u32 * a + bg[c] as u32 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
        DynamicImage::ImageRgba8(rgba).to_rgb8().into()
    };
    if alpha && (s.bg_on || fmt == OutFormat::Jpeg) {
        let bg = if s.bg_on { s.bg } else { [255, 255, 255] };
        return flatten(&img, bg);
    }
    match fmt {
        OutFormat::Jpeg => match img.color() {
            image::ColorType::L8 => img,
            image::ColorType::L16 | image::ColorType::La8 | image::ColorType::La16 => img.to_luma8().into(),
            _ => img.to_rgb8().into(),
        },
        OutFormat::Webp | OutFormat::Bmp | OutFormat::Gif => {
            if alpha || s.prefer_alpha { img.to_rgba8().into() } else { img.to_rgb8().into() }
        }
        OutFormat::Png | OutFormat::Tiff | OutFormat::Match => {
            if s.prefer_alpha && !alpha {
                if matches!(img.color(), image::ColorType::Rgb16 | image::ColorType::L16) { img.to_rgba16().into() } else { img.to_rgba8().into() }
            } else if matches!(img.color(), image::ColorType::Rgb32F | image::ColorType::Rgba32F) {
                img.to_rgba16().into()
            } else {
                img
            }
        }
    }
}

fn encode(img: &DynamicImage, fmt: OutFormat, s: &ExportSettings, src: &[u8], src_path: &Path) -> Result<Vec<u8>, String> {
    let err = |e: image::ImageError| e.to_string();
    let exif = exif_for(src, s.keep_meta);
    let icc = if s.keep_meta == KeepMeta::None { None } else { icc_of(src_path) };
    let mut buf = Vec::new();
    let (w, h) = img.dimensions();
    match fmt {
        OutFormat::Jpeg => {
            let mut enc = JpegEncoder::new_with_quality(&mut buf, s.jpeg_quality.clamp(1, 100));
            apply_meta(&mut enc, exif, icc);
            enc.write_image(img.as_bytes(), w, h, img.color().into()).map_err(err)?;
        }
        OutFormat::Png | OutFormat::Match => {
            let (c, f) = s.png.params();
            let mut enc = png::PngEncoder::new_with_quality(&mut buf, c, f);
            apply_meta(&mut enc, exif, icc);
            enc.write_image(img.as_bytes(), w, h, img.color().into()).map_err(err)?;
            if s.keep_meta != KeepMeta::None {
                // Keep PNG text chunks (e.g. ComfyUI prompt/workflow, A1111 parameters).
                let text = meta_full::png_raw_text_chunks(src);
                if !text.is_empty() && buf.len() >= 12 {
                    let at = buf.len() - 12; // before IEND
                    buf.splice(at..at, text);
                }
            }
        }
        OutFormat::Webp => {
            let mut enc = WebPEncoder::new_lossless(&mut buf);
            apply_meta(&mut enc, exif, icc);
            enc.write_image(img.as_bytes(), w, h, img.color().into()).map_err(err)?;
        }
        OutFormat::Gif => {
            let mut rgba = img.to_rgba8();
            if s.dither {
                let nq = Quant(color_quant::NeuQuant::new(10, 256, rgba.as_raw()));
                image::imageops::dither(&mut rgba, &nq);
            }
            let mut enc = image::codecs::gif::GifEncoder::new_with_speed(&mut buf, 10);
            enc.encode(rgba.as_raw(), w, h, image::ExtendedColorType::Rgba8).map_err(err)?;
        }
        OutFormat::Bmp => img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Bmp).map_err(err)?,
        OutFormat::Tiff => img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Tiff).map_err(err)?,
    }
    Ok(buf)
}

fn apply_meta(enc: &mut impl ImageEncoder, exif: Option<Vec<u8>>, icc: Option<Vec<u8>>) {
    if let Some(e) = exif {
        let _ = enc.set_exif_metadata(e);
    }
    if let Some(p) = icc {
        let _ = enc.set_icc_profile(p);
    }
}

fn icc_of(path: &Path) -> Option<Vec<u8>> {
    let mut dec = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.into_decoder().ok()?;
    image::ImageDecoder::icc_profile(&mut dec).ok().flatten()
}

/// EXIF to write: none, a curated subset, or everything. Orientation is reset because
/// the exported pixels are already upright.
fn exif_for(src: &[u8], keep: KeepMeta) -> Option<Vec<u8>> {
    if keep == KeepMeta::None {
        return None;
    }
    let ex = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(src)).ok()?;
    if keep == KeepMeta::All {
        let mut raw = ex.buf().to_vec();
        reset_orientation(&mut raw);
        return Some(raw);
    }
    use exif::Tag;
    const COMMON: &[Tag] = &[
        Tag::Make, Tag::Model, Tag::LensModel, Tag::DateTime, Tag::DateTimeOriginal, Tag::DateTimeDigitized,
        Tag::ExposureTime, Tag::FNumber, Tag::PhotographicSensitivity, Tag::FocalLength, Tag::ExposureBiasValue,
        Tag::Flash, Tag::WhiteBalance, Tag::Artist, Tag::Copyright, Tag::ImageDescription, Tag::UserComment,
        Tag::GPSLatitude, Tag::GPSLatitudeRef, Tag::GPSLongitude, Tag::GPSLongitudeRef, Tag::GPSAltitude, Tag::GPSAltitudeRef,
    ];
    let fields: Vec<&exif::Field> = ex.fields().filter(|f| f.ifd_num == exif::In::PRIMARY && COMMON.contains(&f.tag)).collect();
    if fields.is_empty() {
        return None;
    }
    let mut w = exif::experimental::Writer::new();
    for f in &fields {
        w.push_field(f);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    w.write(&mut out, ex.little_endian()).ok()?;
    Some(out.into_inner())
}

fn reset_orientation(tiff: &mut [u8]) {
    if tiff.len() < 8 {
        return;
    }
    let le = &tiff[0..2] == b"II";
    let rd16 = |b: &[u8], i: usize| if le { u16::from_le_bytes([b[i], b[i + 1]]) } else { u16::from_be_bytes([b[i], b[i + 1]]) };
    let rd32 = |b: &[u8], i: usize| {
        let a = [b[i], b[i + 1], b[i + 2], b[i + 3]];
        if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
    };
    let ifd = rd32(tiff, 4) as usize;
    if ifd + 2 > tiff.len() {
        return;
    }
    let n = rd16(tiff, ifd) as usize;
    for k in 0..n {
        let e = ifd + 2 + k * 12;
        if e + 12 > tiff.len() {
            return;
        }
        if rd16(tiff, e) == 0x0112 {
            let one = if le { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            tiff[e + 8..e + 10].copy_from_slice(&one);
        }
    }
}

// ---------- writing ----------

pub enum Dest {
    /// Replace the source file (its old version goes to the recycle bin).
    InPlace,
    /// Write next to other exports in a folder, named by the template.
    Folder(PathBuf),
    /// A file name picked in a save dialog.
    File(PathBuf),
}

pub fn file_name(template: &str, src: &Path, e: &Encoded) -> String {
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let base = template
        .replace("{name}", &stem)
        .replace("{w}", &e.width.to_string())
        .replace("{h}", &e.height.to_string())
        .replace("{date}", &date);
    let base = if base.trim().is_empty() { stem } else { base };
    format!("{base}{}.{}", e.suffix, e.ext)
}

/// Exports one job; returns the files written.
pub fn export(job: &Job, s: &ExportSettings, dest: &Dest) -> Result<Vec<PathBuf>, String> {
    let outputs = render(job, s)?;
    let mut written = Vec::new();
    for e in &outputs {
        let target = match dest {
            Dest::InPlace => {
                let t = job.path.with_extension(e.ext);
                let tmp = job.path.with_extension(format!("{}.export-tmp", e.ext));
                std::fs::write(&tmp, &e.bytes).map_err(|x| x.to_string())?;
                // The previous version stays recoverable in the recycle bin.
                if job.path.exists() {
                    trash::delete(&job.path).map_err(|x| x.to_string())?;
                }
                if t != job.path && t.exists() {
                    trash::delete(&t).map_err(|x| x.to_string())?;
                }
                std::fs::rename(&tmp, &t).map_err(|x| x.to_string())?;
                written.push(t);
                continue;
            }
            Dest::Folder(dir) => unique_path(&dir.join(file_name(&s.name_template, &job.path, e))),
            Dest::File(f) => {
                if e.suffix.is_empty() {
                    f.with_extension(e.ext)
                } else {
                    let stem = f.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                    f.with_file_name(format!("{stem}{}.{}", e.suffix, e.ext))
                }
            }
        };
        std::fs::write(&target, &e.bytes).map_err(|x| x.to_string())?;
        written.push(target);
    }
    if s.mode == ExportMode::Move && !matches!(dest, Dest::InPlace) && !written.contains(&job.path) {
        trash::delete(&job.path).map_err(|x| x.to_string())?;
    }
    Ok(written)
}

fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = p.parent().unwrap_or(Path::new("."));
    (2..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).unwrap()
}

/// Palette for GIF dithering.
struct Quant(color_quant::NeuQuant);

impl image::imageops::ColorMap for Quant {
    type Color = image::Rgba<u8>;
    fn index_of(&self, c: &Self::Color) -> usize {
        self.0.index_of(&c.0)
    }
    fn lookup(&self, i: usize) -> Option<Self::Color> {
        self.0.lookup(i).map(image::Rgba)
    }
    fn has_lookup(&self) -> bool {
        true
    }
    fn map_color(&self, c: &mut Self::Color) {
        self.0.map_pixel(&mut c.0);
    }
}

