//! Thin line icons drawn with the painter on a 24×24 grid, so the app needs no icon font.

use eframe::egui::{epaint::PathShape, pos2, Color32, Painter, Pos2, Rect, Shape, Stroke};
use std::f32::consts::TAU;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    More,
    Settings,
    Folder,
    File,
    Grid,
    Single,
    Zoom,
    ZoomIn,
    ZoomOut,
    First,
    Prev,
    Play,
    Pause,
    Next,
    Last,
    Tools,
    Fullscreen,
    ExitFullscreen,
    ChevronUp,
    ChevronRight,
    Info,
    Trash,
    Close,
    Copy,
    Rotate,
    Flip,
    OpenWith,
    Explorer,
    Flag,
    Sort,
    Star,
    Save,
    Clock,
    Frame,
    Picture,
    Refresh,
    Check,
    StepBack,
    StepForward,
    Export,
    Prompt,
    Wand,
}

/// A solid star: the favourite mark on thumbnails and next to the file name.
pub fn star_filled(painter: &Painter, rect: Rect, color: Color32) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    let pt = |i: usize, rr: f32| {
        let a = -TAU / 4.0 + i as f32 * TAU / 10.0;
        pos2(c.x + rr * a.cos(), c.y + 0.04 * r + rr * a.sin())
    };
    let inner: Vec<Pos2> = (0..5).map(|k| pt(2 * k + 1, r * 0.45)).collect();
    painter.add(Shape::convex_polygon(inner.clone(), color, Stroke::NONE));
    for k in 0..5 {
        let tip = pt(2 * k, r);
        let a = inner[(k + 4) % 5];
        let b = inner[k];
        painter.add(Shape::convex_polygon(vec![tip, b, a], color, Stroke::NONE));
    }
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - eframe::egui::vec2(12.0 * s, 12.0 * s);
    let p = |x: f32, y: f32| -> Pos2 { pos2(o.x + x * s, o.y + y * s) };
    let w = (1.5 * s).max(1.0);
    let st = Stroke::new(w, color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), st));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::Path(PathShape::closed_line(pts.iter().map(|&(x, y)| p(x, y)).collect(), st)));
    };
    let filled = |pts: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| p(x, y)).collect(), color, Stroke::NONE));
    };
    let circle = |x: f32, y: f32, r: f32| {
        painter.circle_stroke(p(x, y), r * s, st);
    };
    let dot = |x: f32, y: f32, r: f32| {
        painter.circle_filled(p(x, y), r * s, color);
    };
    let tri_right = |x: f32| closed(&[(x, 6.0), (x + 10.0, 12.0), (x, 18.0)]);
    let tri_left = |x: f32| closed(&[(x, 6.0), (x - 10.0, 12.0), (x, 18.0)]);

    match icon {
        Icon::More => {
            dot(5.0, 12.0, 1.6);
            dot(12.0, 12.0, 1.6);
            dot(19.0, 12.0, 1.6);
        }
        Icon::Settings => {
            circle(12.0, 12.0, 3.0);
            circle(12.0, 12.0, 7.0);
            for i in 0..8 {
                let a = i as f32 / 8.0 * TAU;
                let (c, sn) = (a.cos(), a.sin());
                line(&[(12.0 + c * 7.0, 12.0 + sn * 7.0), (12.0 + c * 9.5, 12.0 + sn * 9.5)]);
            }
        }
        Icon::Folder => closed(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0)]),
        Icon::File => {
            closed(&[(6.0, 3.0), (14.0, 3.0), (19.0, 8.0), (19.0, 21.0), (6.0, 21.0)]);
            line(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)]);
        }
        Icon::Grid => {
            for (x, y) in [(4.0, 4.0), (13.0, 4.0), (4.0, 13.0), (13.0, 13.0)] {
                closed(&[(x, y), (x + 7.0, y), (x + 7.0, y + 7.0), (x, y + 7.0)]);
            }
        }
        Icon::Single | Icon::Picture => {
            closed(&[(3.0, 5.0), (21.0, 5.0), (21.0, 19.0), (3.0, 19.0)]);
            line(&[(3.0, 16.0), (9.0, 11.0), (13.0, 15.0), (16.0, 12.0), (21.0, 16.0)]);
            if icon == Icon::Picture {
                circle(16.0, 9.0, 1.3);
            }
        }
        Icon::Zoom | Icon::ZoomIn | Icon::ZoomOut => {
            circle(10.0, 10.0, 6.0);
            line(&[(14.5, 14.5), (20.0, 20.0)]);
            if icon != Icon::Zoom {
                line(&[(7.0, 10.0), (13.0, 10.0)]);
            }
            if icon == Icon::ZoomIn {
                line(&[(10.0, 7.0), (10.0, 13.0)]);
            }
        }
        Icon::First => {
            line(&[(5.0, 5.0), (5.0, 19.0)]);
            line(&[(20.0, 12.0), (8.0, 12.0)]);
            line(&[(13.0, 7.0), (8.0, 12.0), (13.0, 17.0)]);
        }
        Icon::Last => {
            line(&[(19.0, 5.0), (19.0, 19.0)]);
            line(&[(4.0, 12.0), (16.0, 12.0)]);
            line(&[(11.0, 7.0), (16.0, 12.0), (11.0, 17.0)]);
        }
        Icon::Prev => {
            line(&[(6.0, 6.0), (6.0, 18.0)]);
            tri_left(18.0);
        }
        Icon::Next => {
            line(&[(18.0, 6.0), (18.0, 18.0)]);
            tri_right(6.0);
        }
        Icon::Play => closed(&[(7.0, 5.0), (19.0, 12.0), (7.0, 19.0)]),
        Icon::Pause => {
            line(&[(8.0, 5.0), (8.0, 19.0)]);
            line(&[(16.0, 5.0), (16.0, 19.0)]);
        }
        Icon::StepBack => {
            line(&[(5.0, 6.0), (5.0, 18.0)]);
            filled(&[(17.0, 6.0), (8.0, 12.0), (17.0, 18.0)]);
        }
        Icon::StepForward => {
            line(&[(19.0, 6.0), (19.0, 18.0)]);
            filled(&[(7.0, 6.0), (16.0, 12.0), (7.0, 18.0)]);
        }
        Icon::Tools => {
            // wrench: open jaw at the top right, handle to the bottom left
            line(&[(13.0, 4.5), (11.5, 7.5), (13.0, 10.5), (16.5, 11.0), (19.5, 9.0), (20.5, 6.0)]);
            line(&[(13.0, 4.5), (15.5, 3.5), (17.0, 3.8), (15.0, 6.8), (17.2, 8.8), (20.2, 6.8), (20.5, 6.0)]);
            line(&[(12.5, 10.0), (4.5, 18.0), (6.0, 19.5), (14.0, 11.5)]);
        }
        Icon::Fullscreen => {
            line(&[(14.0, 4.0), (20.0, 4.0), (20.0, 10.0)]);
            line(&[(20.0, 4.0), (13.0, 11.0)]);
            line(&[(10.0, 20.0), (4.0, 20.0), (4.0, 14.0)]);
            line(&[(4.0, 20.0), (11.0, 13.0)]);
        }
        Icon::ExitFullscreen => {
            line(&[(14.0, 4.0), (14.0, 10.0), (20.0, 10.0)]);
            line(&[(14.0, 10.0), (20.0, 4.0)]);
            line(&[(10.0, 20.0), (10.0, 14.0), (4.0, 14.0)]);
            line(&[(10.0, 14.0), (4.0, 20.0)]);
        }
        Icon::ChevronUp => line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Icon::ChevronRight => line(&[(9.0, 6.0), (15.0, 12.0), (9.0, 18.0)]),
        Icon::Info => {
            circle(12.0, 12.0, 9.0);
            line(&[(12.0, 11.0), (12.0, 17.0)]);
            dot(12.0, 7.8, 1.1);
        }
        Icon::Trash => {
            line(&[(4.0, 6.0), (20.0, 6.0)]);
            line(&[(9.0, 6.0), (9.0, 3.5), (15.0, 3.5), (15.0, 6.0)]);
            line(&[(6.0, 6.0), (7.0, 21.0), (17.0, 21.0), (18.0, 6.0)]);
            line(&[(10.0, 10.0), (10.0, 17.0)]);
            line(&[(14.0, 10.0), (14.0, 17.0)]);
        }
        Icon::Close => {
            line(&[(5.0, 5.0), (19.0, 19.0)]);
            line(&[(19.0, 5.0), (5.0, 19.0)]);
        }
        Icon::Export => {
            line(&[(12.0, 3.0), (12.0, 15.0)]);
            line(&[(7.5, 7.5), (12.0, 3.0), (16.5, 7.5)]);
            line(&[(4.0, 13.0), (4.0, 21.0), (20.0, 21.0), (20.0, 13.0)]);
        }
        Icon::Prompt => {
            closed(&[(3.0, 4.0), (21.0, 4.0), (21.0, 17.0), (10.0, 17.0), (6.0, 21.0), (6.0, 17.0), (3.0, 17.0)]);
            line(&[(7.0, 8.5), (17.0, 8.5)]);
            line(&[(7.0, 12.5), (14.0, 12.5)]);
        }
        Icon::Copy => {
            closed(&[(8.0, 8.0), (20.0, 8.0), (20.0, 21.0), (8.0, 21.0)]);
            line(&[(5.0, 17.0), (4.0, 17.0), (4.0, 3.0), (16.0, 3.0), (16.0, 5.0)]);
        }
        Icon::Rotate => {
            let pts: Vec<(f32, f32)> = (0..=20)
                .map(|i| {
                    let a = -TAU * 0.2 + i as f32 / 20.0 * TAU * 0.78;
                    (12.0 + 7.5 * a.cos(), 12.0 + 7.5 * a.sin())
                })
                .collect();
            line(&pts);
            let (ex, ey) = pts[0];
            line(&[(ex - 4.0, ey - 1.0), (ex, ey), (ex - 1.0, ey + 4.0)]);
        }
        Icon::Flip => {
            line(&[(12.0, 3.0), (12.0, 21.0)]);
            closed(&[(9.0, 6.0), (9.0, 18.0), (3.0, 18.0)]);
            closed(&[(15.0, 6.0), (15.0, 18.0), (21.0, 18.0)]);
        }
        Icon::OpenWith => {
            closed(&[(4.0, 4.0), (11.0, 4.0), (11.0, 11.0), (4.0, 11.0)]);
            line(&[(4.0, 15.0), (4.0, 20.0), (20.0, 20.0), (20.0, 4.0), (15.0, 4.0)]);
            line(&[(11.0, 13.0), (17.0, 13.0)]);
            line(&[(14.0, 10.0), (17.0, 13.0), (14.0, 16.0)]);
        }
        Icon::Explorer => {
            closed(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.0), (21.0, 8.0), (21.0, 19.0), (3.0, 19.0)]);
            line(&[(3.0, 11.0), (21.0, 11.0)]);
        }
        Icon::Wand => {
            line(&[(4.0, 20.0), (15.0, 9.0)]);
            line(&[(13.0, 7.0), (17.0, 11.0)]);
            line(&[(18.0, 2.5), (18.0, 6.5)]);
            line(&[(16.0, 4.5), (20.0, 4.5)]);
            line(&[(20.5, 11.0), (20.5, 14.0)]);
            line(&[(19.0, 12.5), (22.0, 12.5)]);
            line(&[(10.0, 3.0), (10.0, 5.5)]);
            line(&[(8.75, 4.25), (11.25, 4.25)]);
        }
        Icon::Flag => {
            line(&[(5.0, 21.0), (5.0, 3.0)]);
            closed(&[(5.0, 4.0), (18.0, 4.0), (15.0, 8.5), (18.0, 13.0), (5.0, 13.0)]);
        }
        Icon::Sort => {
            line(&[(7.0, 4.0), (7.0, 20.0)]);
            line(&[(3.5, 16.5), (7.0, 20.0), (10.5, 16.5)]);
            line(&[(17.0, 20.0), (17.0, 4.0)]);
            line(&[(13.5, 7.5), (17.0, 4.0), (20.5, 7.5)]);
        }
        Icon::Star => {
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -TAU / 4.0 + i as f32 * TAU / 10.0;
                    let r = if i % 2 == 0 { 9.0 } else { 4.0 };
                    (12.0 + r * a.cos(), 12.5 + r * a.sin())
                })
                .collect();
            closed(&pts);
        }
        Icon::Save => {
            closed(&[(4.0, 4.0), (17.0, 4.0), (20.0, 7.0), (20.0, 20.0), (4.0, 20.0)]);
            line(&[(8.0, 4.0), (8.0, 9.0), (15.0, 9.0), (15.0, 4.0)]);
            closed(&[(7.0, 13.0), (17.0, 13.0), (17.0, 20.0), (7.0, 20.0)]);
        }
        Icon::Clock => {
            circle(12.0, 12.0, 9.0);
            line(&[(12.0, 6.5), (12.0, 12.0), (16.0, 14.0)]);
        }
        Icon::Frame => {
            closed(&[(3.0, 3.0), (21.0, 3.0), (21.0, 21.0), (3.0, 21.0)]);
            closed(&[(6.5, 6.5), (17.5, 6.5), (17.5, 17.5), (6.5, 17.5)]);
        }
        Icon::Refresh => {
            let pts: Vec<(f32, f32)> = (0..=20)
                .map(|i| {
                    let a = TAU * 0.05 + i as f32 / 20.0 * TAU * 0.8;
                    (12.0 + 7.5 * a.cos(), 12.0 + 7.5 * a.sin())
                })
                .collect();
            line(&pts);
            let (ex, ey) = pts[20];
            line(&[(ex - 3.5, ey - 1.5), (ex, ey), (ex + 1.0, ey - 4.0)]);
        }
        Icon::Check => line(&[(5.0, 12.5), (10.0, 17.5), (19.5, 7.0)]),
    }
}
