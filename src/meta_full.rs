//! Container-level metadata (file, PNG/JPEG/WebP/GIF structure, text chunks, XMP, ICC)
//! and extraction of generation prompts from AI images (ComfyUI, A1111/Forge).

use crate::meta::{human_size, MetaGroup, MetaSection};
use image::{ExtendedColorType, ImageDecoder, ImageReader};
use std::path::Path;

pub struct TextChunk {
    pub kind: &'static str,
    pub key: String,
    pub text: String,
}

pub fn read_groups(path: &Path) -> Vec<MetaGroup> {
    let mut groups = Vec::new();
    let bytes = std::fs::read(path).unwrap_or_default();

    // File properties.
    let mut rows = vec![("Путь".to_string(), path.display().to_string())];
    if let Ok(md) = std::fs::metadata(path) {
        rows.push(("Размер".into(), format!("{}  ({} байт)", human_size(md.len()), group_digits(md.len()))));
        if let Ok(t) = md.created() {
            rows.push(("Создан".into(), format_time(t)));
        }
        if let Ok(t) = md.modified() {
            rows.push(("Изменён".into(), format_time(t)));
        }
    }
    groups.push(MetaGroup { title: "Свойства файла".into(), sections: vec![MetaSection { title: None, rows, table: false }] });

    // Container.
    let container = sniff(&bytes);
    let mut core = vec![("Формат контейнера".to_string(), container.to_string())];
    let mut extra_sections = Vec::new();
    match container {
        "PNG" => {
            let chunks = png_chunks(&bytes);
            let frames = chunks
                .iter()
                .find(|(t, _)| t == "acTL")
                .and_then(|(_, d)| d.get(0..4))
                .map(|d| u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
                .unwrap_or(1);
            core.push(("Кадров".into(), frames.to_string()));
            core.push(("Чанки".into(), chunk_summary(chunks.iter().map(|(t, _)| t.as_str()))));
            for c in text_chunks_from(&chunks) {
                extra_sections.push(MetaSection {
                    title: Some(format!("Чанк {}", c.kind)),
                    rows: vec![(c.key, c.text)],
                    table: true,
                });
            }
        }
        "JPEG" => {
            let segs = jpeg_segments(&bytes);
            core.push(("Сегменты".into(), chunk_summary(segs.iter().map(|(n, _)| n.as_str()))));
            for (name, data) in &segs {
                if name == "COM" {
                    extra_sections.push(MetaSection {
                        title: Some("Комментарий JPEG".into()),
                        rows: vec![("COM".into(), String::from_utf8_lossy(data).into_owned())],
                        table: true,
                    });
                }
            }
        }
        "GIF" => core.push(("Кадров".into(), gif_frame_count(&bytes).to_string())),
        "WebP" => {
            let chunks = riff_chunks(&bytes);
            let frames = chunks.iter().filter(|(t, _)| t == "ANMF").count().max(1);
            core.push(("Кадров".into(), frames.to_string()));
            core.push(("Чанки".into(), chunk_summary(chunks.iter().map(|(t, _)| t.as_str()))));
        }
        _ => {}
    }
    if let Some(xmp) = find_xmp(&bytes) {
        extra_sections.push(MetaSection { title: Some("XMP".into()), rows: vec![("x:xmpmeta".into(), xmp)], table: true });
    }
    groups.push(MetaGroup { title: "Контейнер".into(), sections: vec![MetaSection { title: Some("Основные свойства".into()), rows: core, table: false }] });

    // Frame.
    let mut frame = Vec::new();
    if let Ok(reader) = ImageReader::open(path).and_then(|r| r.with_guessed_format()) {
        if let Ok(mut dec) = reader.into_decoder() {
            let (w, h) = dec.dimensions();
            frame.push(("Размеры".to_string(), format!("{w} × {h} пикс.")));
            frame.push(("Формат пикселей".into(), pixel_format(dec.original_color_type())));
            let icc = dec.icc_profile().ok().flatten();
            frame.push((
                "Цветовой профиль".into(),
                match icc {
                    Some(p) => format!("{} ({})", icc_description(&p).unwrap_or_else(|| "ICC".into()), human_size(p.len() as u64)),
                    None => "Нет".into(),
                },
            ));
        }
    }
    if !frame.is_empty() {
        groups.push(MetaGroup { title: "Кадр".into(), sections: vec![MetaSection { title: Some("Основные свойства".into()), rows: frame, table: false }] });
    }
    if !extra_sections.is_empty() {
        groups.push(MetaGroup { title: "Встроенные данные".into(), sections: extra_sections });
    }
    groups
}

// ---------- prompt extraction ----------

/// Finds the positive prompt of a generated image: ComfyUI graph, A1111/Forge parameters,
/// or a description stored in EXIF/JPEG comments.
pub fn extract_prompt(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut texts: Vec<(String, String)> = Vec::new();
    match sniff(&bytes) {
        "PNG" => texts.extend(text_chunks_from(&png_chunks(&bytes)).into_iter().map(|c| (c.key, c.text))),
        "JPEG" => {
            for (name, data) in jpeg_segments(&bytes) {
                if name == "COM" {
                    texts.push(("comment".into(), String::from_utf8_lossy(&data).into_owned()));
                }
            }
        }
        _ => {}
    }
    if let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(&bytes)) {
        if let Some(f) = exif.get_field(exif::Tag::UserComment, exif::In::PRIMARY) {
            if let exif::Value::Undefined(ref raw, _) = f.value {
                if let Some(t) = decode_user_comment(raw) {
                    texts.push(("parameters".into(), t));
                }
            }
        }
        if let Some(f) = exif.get_field(exif::Tag::ImageDescription, exif::In::PRIMARY) {
            texts.push(("description".into(), f.display_value().to_string().trim_matches('"').to_string()));
        }
    }
    let find = |key: &str| texts.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str());

    if let Some(p) = find("prompt").and_then(comfy_api_prompt) {
        return Some(p);
    }
    if let Some(p) = find("workflow").and_then(comfy_workflow_prompt) {
        return Some(p);
    }
    if let Some(p) = find("parameters").map(a1111_prompt).filter(|p| !p.is_empty()) {
        return Some(p);
    }
    for key in ["prompt", "description", "comment", "Description", "Comment"] {
        if let Some(v) = find(key).map(str::trim).filter(|v| !v.is_empty() && !v.starts_with('{')) {
            return Some(v.to_string());
        }
    }
    None
}

/// Parses JSON as ComfyUI writes it: Python's json module emits bare `NaN` and `Infinity`,
/// which strict JSON rejects. Those tokens outside strings become `null`.
pub fn parse_json_lenient(text: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str(text) {
        return Some(v);
    }
    let mut out = String::with_capacity(text.len());
    let (mut in_str, mut esc) = (false, false);
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else {
            let tok = ["-Infinity", "Infinity", "NaN"].into_iter().find(|t| rest.starts_with(t));
            if let Some(t) = tok {
                out.push_str("null");
                rest = &rest[t.len()..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    serde_json::from_str(&out).ok()
}

fn a1111_prompt(params: &str) -> String {
    let end = params
        .find("\nNegative prompt:")
        .or_else(|| params.find("\nSteps:"))
        .unwrap_or(params.len());
    params[..end].trim().to_string()
}

/// ComfyUI API-format graph: follow each sampler's `positive` input back to a text node.
fn comfy_api_prompt(json: &str) -> Option<String> {
    let v = parse_json_lenient(json)?;
    let nodes = v.as_object()?;
    let mut found = Vec::new();
    for node in nodes.values() {
        let Some(inputs) = node.get("inputs").and_then(|i| i.as_object()) else { continue };
        if let Some(link) = inputs.get("positive") {
            if let Some(t) = follow_text(nodes, link, 0) {
                found.push(t);
            }
        }
    }
    if found.is_empty() {
        // No sampler wired up: take the longest text from any text-encode node.
        for node in nodes.values() {
            let class = node.get("class_type").and_then(|c| c.as_str()).unwrap_or("");
            if class.contains("TextEncode") || class.contains("Prompt") {
                if let Some(t) = node_text(nodes, node, 0) {
                    found.push(t);
                }
            }
        }
        found.sort_by_key(|t| std::cmp::Reverse(t.len()));
        found.truncate(1);
    }
    found.dedup();
    (!found.is_empty()).then(|| found.join("\n\n"))
}

fn follow_text(nodes: &serde_json::Map<String, serde_json::Value>, link: &serde_json::Value, depth: u32) -> Option<String> {
    if depth > 12 {
        return None;
    }
    if let Some(s) = link.as_str() {
        return Some(s.to_string());
    }
    let id = link.as_array()?.first()?;
    let id = id.as_str().map(|s| s.to_string()).or_else(|| id.as_u64().map(|n| n.to_string()))?;
    node_text(nodes, nodes.get(&id)?, depth + 1)
}

fn node_text(nodes: &serde_json::Map<String, serde_json::Value>, node: &serde_json::Value, depth: u32) -> Option<String> {
    let inputs = node.get("inputs")?.as_object()?;
    for key in ["text", "text_g", "prompt", "positive", "string", "value", "text_positive", "wildcard_text", "populated_text"] {
        if let Some(v) = inputs.get(key) {
            if let Some(s) = v.as_str().filter(|s| !s.trim().is_empty()) {
                return Some(s.trim().to_string());
            }
            if v.is_array() {
                if let Some(t) = follow_text(nodes, v, depth) {
                    return Some(t);
                }
            }
        }
    }
    // Pass-through nodes (guidance, conditioning combine, etc.).
    for (k, v) in inputs {
        if (k.contains("conditioning") || k.contains("positive") || k.starts_with("text")) && v.is_array() {
            if let Some(t) = follow_text(nodes, v, depth) {
                return Some(t);
            }
        }
    }
    None
}

/// ComfyUI UI-format workflow: text-encode nodes keep the prompt in `widgets_values[0]`.
fn comfy_workflow_prompt(json: &str) -> Option<String> {
    let v = parse_json_lenient(json)?;
    let mut texts: Vec<String> = v
        .get("nodes")?
        .as_array()?
        .iter()
        .filter(|n| n.get("type").and_then(|t| t.as_str()).is_some_and(|t| t.contains("TextEncode") || t.contains("Prompt")))
        .filter_map(|n| n.get("widgets_values")?.as_array()?.iter().find_map(|w| w.as_str().map(|s| s.trim().to_string())))
        .filter(|s| !s.is_empty())
        .collect();
    texts.sort_by_key(|t| std::cmp::Reverse(t.len()));
    texts.into_iter().next()
}

fn decode_user_comment(raw: &[u8]) -> Option<String> {
    if raw.len() < 8 {
        return None;
    }
    let (head, body) = raw.split_at(8);
    let text = if head.starts_with(b"UNICODE") {
        let even_zeros = body.iter().step_by(2).filter(|b| **b == 0).count();
        let odd_zeros = body.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
        let units: Vec<u16> = body
            .chunks_exact(2)
            .map(|c| if even_zeros > odd_zeros { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(body).into_owned()
    };
    let t = text.trim_matches(char::from(0)).trim().to_string();
    (!t.is_empty()).then_some(t)
}

// ---------- container parsing ----------

fn sniff(b: &[u8]) -> &'static str {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        "PNG"
    } else if b.starts_with(&[0xFF, 0xD8]) {
        "JPEG"
    } else if b.starts_with(b"GIF8") {
        "GIF"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "WebP"
    } else if b.starts_with(b"BM") {
        "BMP"
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        "TIFF"
    } else if b.starts_with(&[0xFF, 0x0A]) || b.starts_with(b"\0\0\0\x0cJXL ") {
        "JPEG XL"
    } else if b.len() > 12 && &b[4..8] == b"ftyp" {
        match &b[8..12] {
            b"avif" | b"avis" => "AVIF",
            b"heic" | b"heix" | b"mif1" | b"msf1" => "HEIF",
            _ => "ISO BMFF",
        }
    } else if b.starts_with(b"<?xml") || b.starts_with(b"<svg") || b.starts_with(&[0x1f, 0x8b]) {
        "SVG"
    } else if b.starts_with(&[0, 0, 1, 0]) {
        "ICO"
    } else {
        "неизвестен"
    }
}

/// PNG chunks as (type, data).
pub fn png_chunks(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 8;
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let kind = String::from_utf8_lossy(&b[i + 4..i + 8]).into_owned();
        let end = i + 8 + len;
        if end + 4 > b.len() {
            break;
        }
        out.push((kind.clone(), b[i + 8..end].to_vec()));
        i = end + 4;
        if kind == "IEND" {
            break;
        }
    }
    out
}

/// Raw bytes (length, type, data, CRC) of every text chunk, for copying into an exported PNG.
pub fn png_raw_text_chunks(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if !b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return out;
    }
    let mut i = 8;
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let end = i + 12 + len;
        if end > b.len() {
            break;
        }
        if matches!(&b[i + 4..i + 8], b"tEXt" | b"zTXt" | b"iTXt") {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    out
}

pub fn text_chunks_from(chunks: &[(String, Vec<u8>)]) -> Vec<TextChunk> {
    let mut out = Vec::new();
    for (kind, data) in chunks {
        let (kind, key, text) = match kind.as_str() {
            "tEXt" => {
                let Some(z) = data.iter().position(|&c| c == 0) else { continue };
                ("tEXt", latin1(&data[..z]), latin1(&data[z + 1..]))
            }
            "zTXt" => {
                let Some(z) = data.iter().position(|&c| c == 0) else { continue };
                let Some(raw) = data.get(z + 2..) else { continue };
                let text = miniz_oxide::inflate::decompress_to_vec_zlib(raw).map(|d| latin1(&d)).unwrap_or_default();
                ("zTXt", latin1(&data[..z]), text)
            }
            "iTXt" => {
                let Some(z) = data.iter().position(|&c| c == 0) else { continue };
                let key = latin1(&data[..z]);
                let Some(&compressed) = data.get(z + 1) else { continue };
                let rest = &data[z + 3..];
                let Some(l) = rest.iter().position(|&c| c == 0) else { continue };
                let rest = &rest[l + 1..];
                let Some(t) = rest.iter().position(|&c| c == 0) else { continue };
                let body = &rest[t + 1..];
                let text = if compressed == 1 {
                    miniz_oxide::inflate::decompress_to_vec_zlib(body).map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default()
                } else {
                    String::from_utf8_lossy(body).into_owned()
                };
                ("iTXt", key, text)
            }
            _ => continue,
        };
        out.push(TextChunk { kind, key, text });
    }
    out
}

fn latin1(b: &[u8]) -> String {
    // Many tools write UTF-8 into tEXt despite the spec; prefer it when valid.
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

/// JPEG marker segments before the image data, as (name, payload).
fn jpeg_segments(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= b.len() && b[i] == 0xFF {
        let m = b[i + 1];
        if m == 0xD9 || m == 0xDA {
            out.push((if m == 0xDA { "SOS" } else { "EOI" }.to_string(), Vec::new()));
            break;
        }
        if (0xD0..=0xD7).contains(&m) || m == 0x01 || m == 0xFF {
            i += if m == 0xFF { 1 } else { 2 };
            continue;
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let end = (i + 2 + len).min(b.len());
        let data = b.get(i + 4..end).unwrap_or_default().to_vec();
        let name = match m {
            0xE0..=0xEF => {
                let tag = data.iter().take(12).take_while(|&&c| c != 0).map(|&c| c as char).collect::<String>();
                let n = m - 0xE0;
                if tag.is_empty() { format!("APP{n}") } else { format!("APP{n} {}", tag.split('/').next().unwrap_or(&tag)) }
            }
            0xFE => "COM".into(),
            0xDB => "DQT".into(),
            0xC4 => "DHT".into(),
            0xDD => "DRI".into(),
            0xC0 => "SOF0".into(),
            0xC1 => "SOF1".into(),
            0xC2 => "SOF2 (progressive)".into(),
            _ => format!("FF{m:02X}"),
        };
        out.push((name, data));
        i = end;
    }
    out
}

fn riff_chunks(b: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 12;
    while i + 8 <= b.len() {
        let kind = String::from_utf8_lossy(&b[i..i + 4]).trim().to_string();
        let len = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let end = (i + 8 + len).min(b.len());
        out.push((kind, b[i + 8..end].to_vec()));
        i = end + (len & 1);
    }
    out
}

fn gif_frame_count(b: &[u8]) -> usize {
    if b.len() < 13 {
        return 0;
    }
    let mut i = 13;
    if b[10] & 0x80 != 0 {
        i += 3 << ((b[10] & 7) + 1);
    }
    let skip_blocks = |mut i: usize| -> usize {
        while i < b.len() && b[i] != 0 {
            i += b[i] as usize + 1;
        }
        i + 1
    };
    let mut frames = 0;
    while i < b.len() {
        match b[i] {
            0x21 => i = skip_blocks(i + 2),
            0x2C => {
                frames += 1;
                let flags = *b.get(i + 9).unwrap_or(&0);
                i += 10;
                if flags & 0x80 != 0 {
                    i += 3 << ((flags & 7) + 1);
                }
                i = skip_blocks(i + 1);
            }
            _ => break,
        }
    }
    frames
}

fn find_xmp(b: &[u8]) -> Option<String> {
    let start = find(b, b"<x:xmpmeta")?;
    let end = find(&b[start..], b"</x:xmpmeta>")? + start + 12;
    Some(String::from_utf8_lossy(&b[start..end]).into_owned())
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn chunk_summary<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut out: Vec<(String, usize)> = Vec::new();
    for n in names {
        match out.last_mut() {
            Some((last, c)) if last == n => *c += 1,
            _ => out.push((n.to_string(), 1)),
        }
    }
    out.iter().map(|(n, c)| if *c > 1 { format!("{n} ×{c}") } else { n.clone() }).collect::<Vec<_>>().join(", ")
}

fn pixel_format(t: ExtendedColorType) -> String {
    use ExtendedColorType::*;
    match t {
        L1 => "1 бит, ч/б".into(),
        L8 => "8 бит, оттенки серого".into(),
        La8 => "16 бит, серый + альфа".into(),
        L16 => "16 бит, оттенки серого".into(),
        La16 => "32 бит, серый + альфа".into(),
        Rgb8 => "24 бит RGB, 8 бит на канал".into(),
        Rgba8 => "32 бит RGBA, 8 бит на канал".into(),
        Rgb16 => "48 бит RGB, 16 бит на канал".into(),
        Rgba16 => "64 бит RGBA, 16 бит на канал".into(),
        Rgb32F => "96 бит RGB, float".into(),
        Rgba32F => "128 бит RGBA, float".into(),
        Cmyk8 => "32 бит CMYK".into(),
        Unknown(bits) => format!("{bits} бит"),
        other => format!("{other:?}"),
    }
}

/// Reads the profile name from an ICC 'desc' tag (v2 text or v4 multi-localized).
fn icc_description(p: &[u8]) -> Option<String> {
    let count = u32::from_be_bytes(p.get(128..132)?.try_into().ok()?) as usize;
    for k in 0..count.min(100) {
        let e = 132 + k * 12;
        let entry = p.get(e..e + 12)?;
        if &entry[0..4] != b"desc" {
            continue;
        }
        let off = u32::from_be_bytes(entry[4..8].try_into().ok()?) as usize;
        let data = p.get(off..)?;
        return match data.get(0..4)? {
            b"desc" => {
                let n = u32::from_be_bytes(data.get(8..12)?.try_into().ok()?) as usize;
                Some(String::from_utf8_lossy(data.get(12..12 + n)?).trim_matches(char::from(0)).to_string())
            }
            b"mluc" => {
                let len = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?) as usize;
                let soff = u32::from_be_bytes(data.get(24..28)?.try_into().ok()?) as usize;
                let units: Vec<u16> = data.get(soff..soff + len)?.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                Some(String::from_utf16_lossy(&units))
            }
            _ => None,
        };
    }
    None
}

fn format_time(t: std::time::SystemTime) -> String {
    use chrono::{Datelike, Local};
    const WD: [&str; 7] = ["пн", "вт", "ср", "чт", "пт", "сб", "вс"];
    let dt: chrono::DateTime<Local> = t.into();
    format!("{} {}", WD[dt.weekday().num_days_from_monday() as usize], dt.format("%d.%m.%Y %H:%M:%S%.3f (UTC%:z)"))
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}


