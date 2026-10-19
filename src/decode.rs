//! Decoding of image files into RGBA pixel buffers. Runs on worker threads.

use crate::formats;
use eframe::egui::ColorImage;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageReader};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

pub struct Frame {
    pub image: ColorImage,
    pub delay_ms: u32,
}

pub struct Decoded {
    pub frames: Vec<Frame>,
    /// Size of the source image in pixels (after EXIF orientation). The textures
    /// may be smaller when the source exceeds the GPU texture limit.
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub vector: bool,
    /// Red, green, blue and luminance histograms of the first frame.
    pub histogram: Box<[[u32; 256]; 4]>,
}

/// Animations are kept fully in memory; above this budget only the first frame is shown.
const ANIMATION_BUDGET: usize = 1_200_000_000;

pub fn decode_full(path: &Path, max_side: u32) -> Result<Decoded, String> {
    match formats::kind_of(path) {
        Some(formats::Kind::Video) => return Err(msg_video()),
        Some(formats::Kind::Pending) => return Err(msg_pending(path)),
        _ => {}
    }
    if formats::is_svg(path) {
        return decode_svg(path, max_side);
    }
    let ext = formats::ext_of(path);
    match ext.as_str() {
        "gif" => {
            if let Some(d) = try_animation(path, max_side, AnimKind::Gif)? {
                return Ok(d);
            }
        }
        "webp" => {
            if let Some(d) = try_animation(path, max_side, AnimKind::WebP)? {
                return Ok(d);
            }
        }
        "png" | "apng" => {
            if let Some(d) = try_animation(path, max_side, AnimKind::Png)? {
                return Ok(d);
            }
        }
        _ => {}
    }
    let img = if ext == "jxl" {
        let f = File::open(path).map_err(err)?;
        let dec = jxl_oxide::integration::JxlDecoder::new(BufReader::new(f)).map_err(err)?;
        DynamicImage::from_decoder(dec).map_err(err)?
    } else {
        open_dynamic(path)?
    };
    let format = ext.to_uppercase();
    Ok(single(img, max_side, format))
}

fn open_dynamic(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::open(path).map_err(err)?.with_guessed_format().map_err(err)?;
    reader.no_limits();
    let mut decoder = reader.into_decoder().map_err(err)?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(err)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

fn single(img: DynamicImage, max_side: u32, format: String) -> Decoded {
    let (width, height) = (img.width(), img.height());
    let img = fit_texture(img, max_side);
    let rgba = img.to_rgba8();
    let histogram = histogram(rgba.as_raw());
    let image = ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    );
    Decoded {
        frames: vec![Frame { image, delay_ms: 0 }],
        width,
        height,
        format,
        vector: false,
        histogram,
    }
}

fn fit_texture(img: DynamicImage, max_side: u32) -> DynamicImage {
    if img.width() <= max_side && img.height() <= max_side {
        img
    } else {
        img.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    }
}

enum AnimKind {
    Gif,
    WebP,
    Png,
}

fn try_animation(path: &Path, max_side: u32, kind: AnimKind) -> Result<Option<Decoded>, String> {
    let open = || -> Result<BufReader<File>, String> { Ok(BufReader::new(File::open(path).map_err(err)?)) };
    let (frames, format): (Vec<image::Frame>, &str) = match kind {
        AnimKind::Gif => {
            let dec = image::codecs::gif::GifDecoder::new(open()?).map_err(err)?;
            let (w, h) = dec.dimensions();
            let frames = collect_frames(dec.into_frames(), w, h)?;
            (frames, "GIF")
        }
        AnimKind::WebP => {
            let dec = image::codecs::webp::WebPDecoder::new(open()?).map_err(err)?;
            if !dec.has_animation() {
                return Ok(None);
            }
            let (w, h) = dec.dimensions();
            (collect_frames(dec.into_frames(), w, h)?, "WebP")
        }
        AnimKind::Png => {
            let dec = image::codecs::png::PngDecoder::new(open()?).map_err(err)?;
            if !dec.is_apng().unwrap_or(false) {
                return Ok(None);
            }
            let (w, h) = dec.dimensions();
            let apng = dec.apng().map_err(err)?;
            (collect_frames(apng.into_frames(), w, h)?, "APNG")
        }
    };
    if frames.len() <= 1 {
        return Ok(frames.into_iter().next().map(|f| {
            single(DynamicImage::ImageRgba8(f.into_buffer()), max_side, format.to_string())
        }));
    }
    let first = frames[0].buffer();
    let (width, height) = (first.width(), first.height());
    let histogram = histogram(first.as_raw());
    let out = frames
        .into_iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            let mut delay = if d == 0 { 100 } else { n / d };
            if delay < 20 {
                delay = 100; // browsers treat 0-10 ms delays as 100 ms
            }
            let img = fit_texture(DynamicImage::ImageRgba8(f.into_buffer()), max_side).to_rgba8();
            Frame {
                image: ColorImage::from_rgba_unmultiplied(
                    [img.width() as usize, img.height() as usize],
                    img.as_raw(),
                ),
                delay_ms: delay,
            }
        })
        .collect();
    Ok(Some(Decoded {
        frames: out,
        width,
        height,
        format: format.to_string(),
        vector: false,
        histogram,
    }))
}

fn collect_frames(frames: image::Frames<'_>, w: u32, h: u32) -> Result<Vec<image::Frame>, String> {
    let per_frame = (w as usize) * (h as usize) * 4;
    let mut out = Vec::new();
    for f in frames {
        let f = f.map_err(err)?;
        out.push(f);
        if out.len() * per_frame > ANIMATION_BUDGET {
            out.truncate(1);
            break;
        }
    }
    Ok(out)
}

fn decode_svg(path: &Path, max_side: u32) -> Result<Decoded, String> {
    use resvg::{tiny_skia, usvg};
    let data = read_maybe_gz(path)?;
    let mut opt = usvg::Options::default();
    opt.resources_dir = path.parent().map(|p| p.to_path_buf());
    opt.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_data(&data, &opt).map_err(err)?;
    let size = tree.size();
    let (w, h) = (size.width().max(1.0), size.height().max(1.0));
    // Render at up to 2x the natural size so moderate zoom stays crisp.
    let target = (w.max(h) * 2.0).clamp(1024.0, max_side.min(4096) as f32);
    let scale = target / w.max(h);
    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("SVG: пустой размер")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let image = ColorImage::from_rgba_premultiplied([pw as usize, ph as usize], pixmap.data());
    let histogram = histogram(pixmap.data());
    Ok(Decoded {
        frames: vec![Frame { image, delay_ms: 0 }],
        width: w.round() as u32,
        height: h.round() as u32,
        format: "SVG".into(),
        vector: true,
        histogram,
    })
}

fn read_maybe_gz(path: &Path) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    File::open(path).map_err(err)?.read_to_end(&mut data).map_err(err)?;
    Ok(data)
}

/// Fast thumbnail: JPEGs use DCT-domain downscaling, everything else a full decode.
pub fn decode_thumb(path: &Path, size: u32) -> Result<ColorImage, String> {
    let ext = formats::ext_of(path);
    let img = if matches!(ext.as_str(), "jpg" | "jpeg" | "jpe" | "jfif") {
        match jpeg_thumb(path, size) {
            Some(img) => img,
            None => open_dynamic(path)?,
        }
    } else if formats::is_svg(path) {
        let d = decode_svg(path, size.max(256))?;
        let f = d.frames.into_iter().next().ok_or("SVG")?;
        return Ok(shrink_color_image(f.image, size));
    } else if ext == "jxl" {
        let f = File::open(path).map_err(err)?;
        let dec = jxl_oxide::integration::JxlDecoder::new(BufReader::new(f)).map_err(err)?;
        DynamicImage::from_decoder(dec).map_err(err)?
    } else if matches!(formats::kind_of(path), Some(formats::Kind::Image)) {
        open_dynamic(path)?
    } else {
        return Err("нет декодера".into());
    };
    let t = img.thumbnail(size, size).to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [t.width() as usize, t.height() as usize],
        t.as_raw(),
    ))
}

fn shrink_color_image(img: ColorImage, size: u32) -> ColorImage {
    let [w, h] = img.size;
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    match image::RgbaImage::from_raw(w as u32, h as u32, rgba) {
        Some(buf) => {
            let t = DynamicImage::ImageRgba8(buf).thumbnail(size, size).to_rgba8();
            ColorImage::from_rgba_premultiplied([t.width() as usize, t.height() as usize], t.as_raw())
        }
        None => img,
    }
}

fn jpeg_thumb(path: &Path, size: u32) -> Option<DynamicImage> {
    let file = File::open(path).ok()?;
    let mut dec = jpeg_decoder::Decoder::new(BufReader::new(file));
    let target = size.min(u16::MAX as u32) as u16;
    let (w, h) = dec.scale(target, target).ok()?;
    let pixels = dec.decode().ok()?;
    let info = dec.info()?;
    let (w, h) = (w as u32, h as u32);
    let img = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, pixels)?),
        jpeg_decoder::PixelFormat::L8 => DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, pixels)?),
        jpeg_decoder::PixelFormat::CMYK32 => {
            let rgb: Vec<u8> = pixels
                .chunks_exact(4)
                .flat_map(|p| {
                    let k = p[3] as u32;
                    [(p[0] as u32 * k / 255) as u8, (p[1] as u32 * k / 255) as u8, (p[2] as u32 * k / 255) as u8]
                })
                .collect();
            DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, rgb)?)
        }
        _ => return None,
    };
    let mut img = img;
    if let Some(o) = crate::meta::read_orientation(path) {
        if let Some(o) = image::metadata::Orientation::from_exif(o) {
            img.apply_orientation(o);
        }
    }
    Some(img)
}

fn histogram(rgba: &[u8]) -> Box<[[u32; 256]; 4]> {
    let mut h = Box::new([[0u32; 256]; 4]);
    let pixels = rgba.len() / 4;
    let step = (pixels / 400_000).max(1);
    for px in rgba.chunks_exact(4).step_by(step) {
        if px[3] == 0 {
            continue;
        }
        h[0][px[0] as usize] += 1;
        h[1][px[1] as usize] += 1;
        h[2][px[2] as usize] += 1;
        let l = (px[0] as u32 * 54 + px[1] as u32 * 183 + px[2] as u32 * 19) >> 8;
        h[3][l.min(255) as usize] += 1;
    }
    h
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn msg_video() -> String {
    "Видео пока открывается во внешнем проигрывателе (Enter или двойной щелчок).".into()
}

fn msg_pending(path: &Path) -> String {
    format!(
        "Формат {} пока не поддерживается встроенным декодером на Rust.",
        formats::ext_of(path).to_uppercase()
    )
}

/// Full-resolution pixels (EXIF orientation applied) for export; animations give the first frame.
pub fn load_dynamic(path: &Path) -> Result<DynamicImage, String> {
    if let Some(formats::Kind::Pending | formats::Kind::Video) = formats::kind_of(path) {
        return Err(msg_pending(path));
    }
    if formats::is_svg(path) {
        let d = decode_svg(path, 8192)?;
        let f = d.frames.into_iter().next().ok_or("SVG")?;
        let [w, h] = f.image.size;
        let rgba: Vec<u8> = f.image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let buf = image::RgbaImage::from_raw(w as u32, h as u32, rgba).ok_or("SVG")?;
        return Ok(DynamicImage::ImageRgba8(buf));
    }
    if formats::ext_of(path) == "jxl" {
        let f = File::open(path).map_err(err)?;
        let dec = jxl_oxide::integration::JxlDecoder::new(BufReader::new(f)).map_err(err)?;
        return DynamicImage::from_decoder(dec).map_err(err);
    }
    open_dynamic(path)
}
