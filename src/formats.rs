use std::path::Path;

/// Formats decoded by the built-in pure-Rust decoders.
pub const IMAGE_EXT: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "dib", "tif", "tiff", "ico",
    "jxl", "svg", "svgz", "tga", "qoi",
];

/// Formats that are listed and thumbnailed but need a codec that is not built in yet.
pub const PENDING_EXT: &[&str] = &[
    "heic", "heif", "avif", "jxr", "wdp", "hdp", "dng", "cr2", "cr3", "nef", "nrw", "arw", "orf",
    "rw2", "raf", "pef", "srw",
];

pub const VIDEO_EXT: &[&str] = &[
    "mp4", "m4v", "mkv", "mts", "m2ts", "avi", "mov", "webm", "wmv", "mpg", "mpeg", "mpe",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Pending,
    Video,
}

pub fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn kind_of(path: &Path) -> Option<Kind> {
    let ext = ext_of(path);
    let e = ext.as_str();
    if IMAGE_EXT.contains(&e) {
        Some(Kind::Image)
    } else if PENDING_EXT.contains(&e) {
        Some(Kind::Pending)
    } else if VIDEO_EXT.contains(&e) {
        Some(Kind::Video)
    } else {
        None
    }
}

pub fn is_svg(path: &Path) -> bool {
    matches!(ext_of(path).as_str(), "svg" | "svgz")
}

pub fn is_session(path: &Path) -> bool {
    ext_of(path) == "ivsession"
}
