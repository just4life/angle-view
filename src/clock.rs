//! Date, time and month calendar overlay (digital or analog clock).

use crate::settings::ClockStyle;
use crate::ui::shadow_text;
use chrono::{Datelike, Local, NaiveDate, Timelike};
use eframe::egui::{pos2, vec2, Align2, Color32, FontId, Painter, Rect, Stroke};
use std::f32::consts::TAU;

const WEEKDAYS: [&str; 7] = ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"];
const MONTHS_GEN: [&str; 12] = [
    "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря",
];
const MONTHS: [&str; 12] = [
    "Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь",
];

pub fn paint(p: &Painter, area: Rect, style: ClockStyle, calendar: bool) {
    let now = Local::now();
    let date = format!(
        "{}, {} {} {}",
        WEEKDAYS[now.weekday().num_days_from_monday() as usize],
        now.day(),
        MONTHS_GEN[now.month0() as usize],
        now.year()
    );
    let right = area.right() - 28.0;
    let mut bottom = area.bottom() - 24.0;
    if calendar {
        let h = paint_calendar(p, pos2(right, bottom), now.date_naive());
        bottom -= h + 18.0;
    }
    match style {
        ClockStyle::Digital => {
            shadow_text(p, pos2(right, bottom), Align2::RIGHT_BOTTOM, &date, 18.0);
            let time = format!("{:02}:{:02}", now.hour(), now.minute());
            shadow_text(p, pos2(right, bottom - 26.0), Align2::RIGHT_BOTTOM, &time, 64.0);
        }
        ClockStyle::Analog => {
            shadow_text(p, pos2(right, bottom), Align2::RIGHT_BOTTOM, &date, 16.0);
            let r = 64.0;
            let c = pos2(right - r, bottom - 30.0 - r);
            p.circle_filled(c, r + 6.0, Color32::from_black_alpha(130));
            p.circle_stroke(c, r, Stroke::new(2.0, Color32::WHITE));
            for i in 0..60 {
                let a = i as f32 / 60.0 * TAU;
                let len = if i % 5 == 0 { 9.0 } else { 3.0 };
                let d = vec2(a.sin(), -a.cos());
                p.line_segment([c + d * (r - len), c + d * (r - 2.0)], Stroke::new(if i % 5 == 0 { 2.0 } else { 1.0 }, Color32::WHITE));
            }
            let s = now.second() as f32;
            let m = now.minute() as f32 + s / 60.0;
            let h = (now.hour() % 12) as f32 + m / 60.0;
            let hand = |frac: f32, len: f32, w: f32, col: Color32| {
                let a = frac * TAU;
                p.line_segment([c, c + vec2(a.sin(), -a.cos()) * len], Stroke::new(w, col));
            };
            hand(h / 12.0, r * 0.5, 4.0, Color32::WHITE);
            hand(m / 60.0, r * 0.78, 3.0, Color32::WHITE);
            hand(s / 60.0, r * 0.85, 1.2, Color32::from_rgb(255, 120, 90));
            p.circle_filled(c, 3.5, Color32::WHITE);
        }
        ClockStyle::Off => {}
    }
}

/// Paints a month grid anchored at its bottom-right corner; returns its height.
fn paint_calendar(p: &Painter, br: eframe::egui::Pos2, today: NaiveDate) -> f32 {
    let cw = 30.0;
    let ch = 22.0;
    let first = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let offset = first.weekday().num_days_from_monday() as usize;
    let days = (1..=31).take_while(|d| NaiveDate::from_ymd_opt(today.year(), today.month(), *d).is_some()).count();
    let rows = (offset + days).div_ceil(7);
    let w = cw * 7.0;
    let h = ch * (rows as f32 + 2.0);
    let rect = Rect::from_min_max(br - vec2(w + 12.0, h + 10.0), br);
    p.rect_filled(rect, 6.0, Color32::from_black_alpha(140));
    let x0 = rect.left() + 6.0;
    let y0 = rect.top() + 5.0;
    p.text(pos2(x0 + w / 2.0, y0 + ch / 2.0), Align2::CENTER_CENTER, format!("{} {}", MONTHS[today.month0() as usize], today.year()), FontId::proportional(14.0), Color32::WHITE);
    for (i, d) in ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"].iter().enumerate() {
        let col = if i >= 5 { Color32::from_rgb(255, 150, 130) } else { Color32::from_gray(180) };
        p.text(pos2(x0 + cw * (i as f32 + 0.5), y0 + ch * 1.5), Align2::CENTER_CENTER, *d, FontId::proportional(12.0), col);
    }
    for day in 1..=days {
        let k = offset + day - 1;
        let (col, row) = (k % 7, k / 7);
        let c = pos2(x0 + cw * (col as f32 + 0.5), y0 + ch * (row as f32 + 2.5));
        if day as u32 == today.day() {
            p.circle_filled(c, 10.0, crate::style::ACCENT_DARK);
        }
        p.text(c, Align2::CENTER_CENTER, day.to_string(), FontId::proportional(13.0), Color32::WHITE);
    }
    h + 10.0
}
