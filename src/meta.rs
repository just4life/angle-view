//! EXIF and file metadata for the inspector panel.

use exif::{In, Reader, Tag, Value};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub struct Metadata {
    /// Curated, human-readable rows.
    pub summary: Vec<(String, String)>,
    /// Everything found in the file, grouped for the full metadata panel.
    pub groups: Vec<MetaGroup>,
    /// Generation prompt for AI images, when one could be found.
    pub prompt: Option<String>,
}

pub struct MetaGroup {
    pub title: String,
    pub sections: Vec<MetaSection>,
}

pub struct MetaSection {
    pub title: Option<String>,
    pub rows: Vec<(String, String)>,
    /// Shown as a two-column table with a header instead of "key: value" lines.
    pub table: bool,
}

fn read(path: &Path) -> Option<exif::Exif> {
    let file = File::open(path).ok()?;
    Reader::new().read_from_container(&mut BufReader::new(file)).ok()
}

pub fn read_orientation(path: &Path) -> Option<u8> {
    let exif = read(path)?;
    let f = exif.get_field(Tag::Orientation, In::PRIMARY)?;
    f.value.get_uint(0).map(|v| v as u8)
}

pub fn read_metadata(path: &Path) -> Metadata {
    let mut summary = Vec::new();
    let mut all = Vec::new();
    let mut groups = crate::meta_full::read_groups(path);
    let prompt = crate::meta_full::extract_prompt(path);
    if let Ok(md) = std::fs::metadata(path) {
        summary.push(("Размер файла".into(), human_size(md.len())));
        if let Ok(t) = md.modified() {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            summary.push(("Изменён".into(), dt.format("%d.%m.%Y %H:%M:%S").to_string()));
        }
    }
    let Some(exif) = read(path) else {
        return Metadata { summary, groups, prompt };
    };
    let get = |tag: Tag| exif.get_field(tag, In::PRIMARY);
    let text = |tag: Tag| get(tag).map(|f| f.display_value().with_unit(&exif).to_string().trim_matches('"').trim().to_string());

    if let Some(v) = text(Tag::DateTimeOriginal) {
        summary.push(("Дата съёмки".into(), v));
    }
    let make = text(Tag::Make).unwrap_or_default();
    let model = text(Tag::Model).unwrap_or_default();
    if !make.is_empty() || !model.is_empty() {
        let cam = if model.starts_with(&make) { model } else { format!("{make} {model}") };
        summary.push(("Камера".into(), cam.trim().to_string()));
    }
    if let Some(v) = text(Tag::LensModel) {
        summary.push(("Объектив".into(), v));
    }
    if let Some(f) = get(Tag::ExposureTime) {
        if let Value::Rational(ref r) = f.value {
            if let Some(r) = r.first() {
                let secs = r.to_f64();
                let s = if secs > 0.0 && secs < 0.5 { format!("1/{:.0} с", 1.0 / secs) } else { format!("{secs:.1} с") };
                summary.push(("Выдержка".into(), s));
            }
        }
    }
    if let Some(f) = get(Tag::FNumber) {
        if let Value::Rational(ref r) = f.value {
            if let Some(r) = r.first() {
                summary.push(("Диафрагма".into(), format!("f/{:.1}", r.to_f64())));
            }
        }
    }
    if let Some(f) = get(Tag::PhotographicSensitivity) {
        if let Some(v) = f.value.get_uint(0) {
            summary.push(("ISO".into(), v.to_string()));
        }
    }
    if let Some(f) = get(Tag::FocalLength) {
        if let Value::Rational(ref r) = f.value {
            if let Some(r) = r.first() {
                summary.push(("Фокусное расстояние".into(), format!("{:.0} мм", r.to_f64())));
            }
        }
    }
    if let Some(v) = text(Tag::ExposureBiasValue) {
        summary.push(("Экспокоррекция".into(), v));
    }
    if let Some(v) = text(Tag::Flash) {
        summary.push(("Вспышка".into(), v));
    }
    if let Some(v) = text(Tag::WhiteBalance) {
        summary.push(("Баланс белого".into(), v));
    }
    if let (Some(lat), Some(lon)) = (gps(&exif, Tag::GPSLatitude, Tag::GPSLatitudeRef), gps(&exif, Tag::GPSLongitude, Tag::GPSLongitudeRef)) {
        summary.push(("GPS".into(), format!("{lat:.6}, {lon:.6}")));
    }
    if let Some(v) = text(Tag::Artist) {
        summary.push(("Автор".into(), v));
    }
    if let Some(v) = text(Tag::Copyright) {
        summary.push(("Авторские права".into(), v));
    }
    if let Some(v) = text(Tag::ImageDescription) {
        summary.push(("Описание".into(), v));
    }
    if let Some(v) = text(Tag::Software) {
        summary.push(("Программа".into(), v));
    }

    let mut thumb = Vec::new();
    for f in exif.fields() {
        let row = (f.tag.to_string(), f.display_value().with_unit(&exif).to_string());
        if f.ifd_num == In::PRIMARY { all.push(row) } else { thumb.push(row) }
    }
    let mut sections = vec![MetaSection { title: Some("Основное изображение".into()), rows: all, table: true }];
    if !thumb.is_empty() {
        sections.push(MetaSection { title: Some("Встроенная миниатюра".into()), rows: thumb, table: true });
    }
    groups.push(MetaGroup { title: "EXIF".into(), sections });
    Metadata { summary, groups, prompt }
}

fn gps(exif: &exif::Exif, tag: Tag, ref_tag: Tag) -> Option<f64> {
    let f = exif.get_field(tag, In::PRIMARY)?;
    let Value::Rational(ref v) = f.value else { return None };
    if v.len() < 3 {
        return None;
    }
    let mut deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
    if let Some(r) = exif.get_field(ref_tag, In::PRIMARY) {
        let s = r.display_value().to_string();
        if s.contains('S') || s.contains('W') {
            deg = -deg;
        }
    }
    Some(deg)
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} ГБ", b / 1024.0 / 1024.0 / 1024.0)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} МБ", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.0} КБ", b / 1024.0)
    } else {
        format!("{bytes} Б")
    }
}
