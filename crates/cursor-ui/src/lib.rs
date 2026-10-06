//! Отрисовка удалённого курсора в окне просмотра.
//!
//! Крейт общий для приложения `rat` и вьюера `rat-viewer`: вид курсора не должен
//! расходиться между ними. Формы заданы координатами **относительно хотспота
//! курсора** — точки (0,0), в которую попадает щелчок, поэтому кончик/центр
//! фигуры всегда совпадает с настоящей позицией мыши.
//!
//! Каждая форма — набор **выпуклых** многоугольников (заливка egui точна только
//! для выпуклых) плюс при необходимости круги и отрезки. Части перекрываются,
//! поэтому чёрная обводка рисуется отдельным нижним слоем, а заливки ложатся
//! поверх — на стыках частей швов не видно.
//!
//! Вид приходит с агента числом `rat_session::input::CURSOR_*`; неизвестный
//! (кастомный) курсор рисуется обычной стрелкой.

use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2};

use rat_session::input as cur;

/// Точка сетки (от хотспота) → экран.
fn p(pos: Pos2, s: f32, x: f32, y: f32) -> Pos2 {
    pos2(pos.x + x * s, pos.y + y * s)
}

/// Прямоугольник по центру (cx, cy) и размерам.
fn rect(cx: f32, cy: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    vec![
        (cx - w / 2.0, cy - h / 2.0),
        (cx + w / 2.0, cy - h / 2.0),
        (cx + w / 2.0, cy + h / 2.0),
        (cx - w / 2.0, cy + h / 2.0),
    ]
}

/// Стрелка: голова + хвост. Хотспот — кончик.
fn arrow() -> Vec<Vec<(f32, f32)>> {
    vec![
        vec![(0.0, 0.0), (12.8, 16.5), (0.0, 17.0)],
        vec![(4.6, 14.5), (7.6, 13.2), (11.2, 25.2), (8.2, 26.5)],
    ]
}

/// Выпуклые части формы курсора (координаты от хотспота).
pub fn shapes_of(kind: u8) -> Vec<Vec<(f32, f32)>> {
    match kind {
        cur::CURSOR_IBEAM => vec![
            rect(0.0, -12.25, 14.0, 3.5),
            rect(0.0, 0.0, 3.0, 25.0),
            rect(0.0, 12.25, 14.0, 3.5),
        ],
        cur::CURSOR_CROSS => vec![rect(0.0, 0.0, 3.0, 28.0), rect(0.0, 0.0, 24.0, 3.0)],
        cur::CURSOR_UPARROW => vec![
            vec![(0.0, 0.0), (7.5, 10.0), (-7.5, 10.0)],
            rect(0.0, 18.5, 7.0, 19.0),
        ],
        cur::CURSOR_HAND => vec![
            rect(0.0, 5.0, 2.6, 10.5),
            rect(2.9, 5.9, 2.6, 8.5),
            rect(5.8, 6.6, 2.6, 7.0),
            rect(8.4, 7.4, 2.0, 5.5),
            vec![(-1.3, 9.0), (-1.3, 13.0), (-6.8, 15.8), (-8.0, 12.6)],
            vec![
                (-1.3, 8.5),
                (9.4, 8.5),
                (9.4, 16.5),
                (2.0, 28.0),
                (-1.3, 28.0),
            ],
        ],
        cur::CURSOR_SIZE_NS => vec![
            vec![(0.0, -14.0), (5.0, -8.0), (-5.0, -8.0)],
            rect(0.0, 0.0, 4.0, 18.0),
            vec![(0.0, 14.0), (5.0, 8.0), (-5.0, 8.0)],
        ],
        cur::CURSOR_SIZE_WE => vec![
            vec![(-12.0, 0.0), (-6.0, 5.0), (-6.0, -5.0)],
            rect(0.0, 0.0, 18.0, 4.0),
            vec![(12.0, 0.0), (6.0, 5.0), (6.0, -5.0)],
        ],
        cur::CURSOR_SIZE_NWSE => vec![
            vec![(-12.0, -12.0), (-6.0, -12.0), (-12.0, -6.0)],
            vec![(-9.0, -6.0), (6.0, 9.0), (9.0, 6.0), (-6.0, -9.0)],
            vec![(12.0, 12.0), (6.0, 12.0), (12.0, 6.0)],
        ],
        cur::CURSOR_SIZE_NESW => vec![
            vec![(12.0, -12.0), (6.0, -12.0), (12.0, -6.0)],
            vec![(9.0, -6.0), (-6.0, 9.0), (-9.0, 6.0), (6.0, -9.0)],
            vec![(-12.0, 12.0), (-12.0, 6.0), (-6.0, 12.0)],
        ],
        cur::CURSOR_SIZE_ALL => vec![
            vec![(0.0, -14.0), (5.0, -8.0), (-5.0, -8.0)],
            rect(0.0, 0.0, 4.0, 18.0),
            vec![(0.0, 14.0), (5.0, 8.0), (-5.0, 8.0)],
            vec![(-12.0, 0.0), (-6.0, 5.0), (-6.0, -5.0)],
            rect(0.0, 0.0, 18.0, 4.0),
            vec![(12.0, 0.0), (6.0, 5.0), (6.0, -5.0)],
        ],
        cur::CURSOR_WAIT => vec![
            vec![(-6.0, -12.0), (6.0, -12.0), (0.0, -0.5)],
            vec![(0.0, 0.5), (6.0, 12.0), (-6.0, 12.0)],
        ],
        cur::CURSOR_APPSTARTING => vec![
            // маленькая стрелка (хотспот — её кончик) и песочные часы рядом
            vec![(0.0, 0.0), (8.3, 10.7), (0.0, 11.1)],
            vec![(3.0, 9.4), (4.9, 8.6), (7.3, 16.4), (5.3, 17.2)],
            vec![(6.0, 3.5), (11.0, 3.5), (8.5, 8.0)],
            vec![(8.5, 9.0), (11.0, 13.5), (6.0, 13.5)],
        ],
        // У справки — стрелка плюс круг с «?» (круг и текст рисует paint_cursor).
        cur::CURSOR_HELP => arrow(),
        // Круг «запрещено» рисуется в paint_cursor.
        cur::CURSOR_NO => Vec::new(),
        // Стрелка и всё неизвестное (кастомные курсоры приложений).
        _ => arrow(),
    }
}

/// Нарисовать курсор: `pos` — хотспот в экранных пикселях, `scale` — размер.
pub fn paint_cursor(painter: &Painter, pos: Pos2, scale: f32, kind: u8) {
    let s = scale.max(0.2);
    let stroke_w = (1.4 * s).max(1.2);
    let polys = shapes_of(kind);

    let mut circles: Vec<(f32, f32, f32)> = Vec::new(); // x, y, r
    let mut slash: Option<((f32, f32), (f32, f32))> = None;
    if kind == cur::CURSOR_NO {
        circles.push((0.0, 0.0, 10.5));
        slash = Some(((-7.4, 7.4), (7.4, -7.4)));
    }
    let help_at = if kind == cur::CURSOR_HELP {
        circles.push((15.0, 19.0, 6.5));
        Some(p(pos, s, 15.0, 19.0))
    } else {
        None
    };

    // 1) нижний слой — чёрные контуры: на стыках перекрытых частей их закроют
    // заливки, снаружи остаётся аккуратная окантовка.
    for &(x, y, r) in &circles {
        painter.add(Shape::circle_stroke(
            p(pos, s, x, y),
            r * s,
            Stroke::new(stroke_w, Color32::BLACK),
        ));
    }
    for poly in &polys {
        let pts: Vec<Pos2> = poly.iter().map(|&(x, y)| p(pos, s, x, y)).collect();
        painter.add(Shape::closed_line(
            pts,
            Stroke::new(stroke_w, Color32::BLACK),
        ));
    }

    // 2) заливки.
    for &(x, y, r) in &circles {
        painter.add(Shape::circle_filled(p(pos, s, x, y), r * s, Color32::WHITE));
    }
    for poly in &polys {
        let pts: Vec<Pos2> = poly.iter().map(|&(x, y)| p(pos, s, x, y)).collect();
        painter.add(Shape::convex_polygon(pts, Color32::WHITE, Stroke::NONE));
    }

    // 3) детали поверх заливок.
    if let Some((a, b)) = slash {
        painter.add(Shape::line_segment(
            [p(pos, s, a.0, a.1), p(pos, s, b.0, b.1)],
            Stroke::new((2.0 * s).max(1.5), Color32::BLACK),
        ));
    }
    if let Some(c) = help_at {
        painter.text(
            c,
            egui::Align2::CENTER_CENTER,
            "?",
            egui::FontId::proportional(9.0 * s),
            Color32::BLACK,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_convex(pts: &[(f32, f32)]) -> bool {
        let n = pts.len();
        if n < 3 {
            return false;
        }
        let mut sign = 0.0_f32;
        for i in 0..n {
            let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
            let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
            if cross.abs() < 1e-4 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }

    #[test]
    fn all_parts_are_convex() {
        for kind in 0..=13_u8 {
            for part in shapes_of(kind) {
                assert!(part.len() >= 3, "вид {kind}: часть без площади");
                assert!(is_convex(&part), "вид {kind}: невыпуклая часть {part:?}");
            }
        }
    }

    #[test]
    fn custom_falls_back_to_arrow() {
        assert!(!shapes_of(255).is_empty());
        assert_eq!(shapes_of(255), shapes_of(cur::CURSOR_ARROW));
        assert_eq!(shapes_of(cur::CURSOR_ARROW), arrow());
    }
}
