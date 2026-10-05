// Иконка приложения Rat Next: тёмный скруглённый квадрат, голубой «экран»
// с белым курсором и зелёная точка активной сессии.
//
// Один и тот же код рисует иконку окна, иконку трея и `.ico` для exe: файл
// подключается как модуль приложения (`mod icon;`) и через `include!` в
// `build.rs`. Поэтому здесь нет внешних зависимостей и inner-атрибутов.

/// Фон: градиент сверху вниз.
const BG_TOP: (u8, u8, u8) = (44, 52, 66);
const BG_BOTTOM: (u8, u8, u8) = (21, 25, 32);
/// Рамка «экрана».
const SCREEN: (u8, u8, u8) = (76, 140, 255);
/// Курсор внутри экрана.
const CURSOR: (u8, u8, u8) = (255, 255, 255);
/// Индикатор активной сессии.
const LIVE: (u8, u8, u8) = (76, 175, 80);

/// Иконка `size`×`size` в RGBA8.
///
/// Рисуется в четырёхкратном разрешении и усредняется — так обходятся без
/// отдельного сглаживания контуров.
#[allow(dead_code)]
pub fn rgba(size: u32) -> Vec<u8> {
    let size = size.max(8);
    let ss = 4;
    let hi = size * ss;
    let mut buf = vec![0u8; (hi as usize) * (hi as usize) * 4];
    paint(&mut buf, hi);
    downscale(&buf, hi, size, ss)
}

fn paint(buf: &mut [u8], n: u32) {
    let f = n as f32;

    // Фон со скруглёнными углами.
    for y in 0..n {
        for x in 0..n {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !rounded(
                px,
                py,
                f * 0.5,
                f * 0.5,
                f * 0.5 - 0.5,
                f * 0.5 - 0.5,
                f * 0.22,
            ) {
                continue;
            }
            put(buf, n, x, y, mix(BG_TOP, BG_BOTTOM, py / f));
        }
    }

    // Монитор: рамка, ножка, подставка.
    let (scx, scy) = (f * 0.5, f * 0.40);
    let (hw, hh) = (f * 0.31, f * 0.22);
    let border = f * 0.06;
    let round = f * 0.07;
    for y in 0..n {
        for x in 0..n {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let outer = rounded(px, py, scx, scy, hw, hh, round);
            let inner = rounded(
                px,
                py,
                scx,
                scy,
                hw - border,
                hh - border,
                (round - border).max(0.0),
            );
            if outer && !inner {
                put(buf, n, x, y, SCREEN);
                continue;
            }
            let stem =
                py >= scy + hh - 0.5 && py <= scy + hh + f * 0.075 && (px - scx).abs() <= f * 0.05;
            let base =
                (py - (scy + hh + f * 0.075)).abs() <= f * 0.025 && (px - scx).abs() <= f * 0.16;
            if stem || base {
                put(buf, n, x, y, SCREEN);
            }
        }
    }

    // Курсор внутри экрана.
    let cursor = [
        (0.00_f32, 0.00_f32),
        (0.00, 0.94),
        (0.22, 0.74),
        (0.35, 1.00),
        (0.47, 0.93),
        (0.35, 0.68),
        (0.58, 0.64),
    ];
    let scale = f * 0.34;
    let (ox, oy) = (scx - hw * 0.52, scy - hh * 0.62);
    for y in 0..n {
        for x in 0..n {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let lx = (px - ox) / scale;
            let ly = (py - oy) / scale;
            if (0.0..=1.0).contains(&lx) && (0.0..=1.0).contains(&ly) && in_polygon(lx, ly, &cursor)
            {
                put(buf, n, x, y, CURSOR);
            }
        }
    }

    // Точка сессии — только на крупных размерах, иначе сливается в пятно.
    if f >= 24.0 {
        for y in 0..n {
            for x in 0..n {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let d = ((px - f * 0.735).powi(2) + (py - f * 0.73).powi(2)).sqrt();
                if d <= f * 0.135 {
                    put(buf, n, x, y, BG_BOTTOM);
                }
                if d <= f * 0.10 {
                    put(buf, n, x, y, LIVE);
                }
            }
        }
    }
}

fn put(buf: &mut [u8], n: u32, x: u32, y: u32, (r, g, b): (u8, u8, u8)) {
    let i = ((y * n + x) * 4) as usize;
    buf[i] = r;
    buf[i + 1] = g;
    buf[i + 2] = b;
    buf[i + 3] = 255;
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let one = |from: u8, to: u8| (from as f32 + (to as f32 - from as f32) * t).round() as u8;
    (one(a.0, b.0), one(a.1, b.1), one(a.2, b.2))
}

fn rounded(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> bool {
    let ax = (x - cx).abs();
    let ay = (y - cy).abs();
    if ax > hw || ay > hh {
        return false;
    }
    let dx = ax - (hw - r);
    let dy = ay - (hh - r);
    if dx <= 0.0 || dy <= 0.0 {
        return true;
    }
    dx * dx + dy * dy <= r * r
}

fn in_polygon(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn downscale(src: &[u8], hi: u32, size: u32, ss: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size as usize) * (size as usize) * 4];
    let count = ss * ss;
    for y in 0..size {
        for x in 0..size {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = (((y * ss + sy) * hi + (x * ss + sx)) * 4) as usize;
                    let alpha = src[i + 3] as u32;
                    r += src[i] as u32 * alpha / 255;
                    g += src[i + 1] as u32 * alpha / 255;
                    b += src[i + 2] as u32 * alpha / 255;
                    a += alpha;
                }
            }
            let o = ((y * size + x) * 4) as usize;
            if a == 0 {
                continue;
            }
            out[o] = (r * 255 / a) as u8;
            out[o + 1] = (g * 255 / a) as u8;
            out[o + 2] = (b * 255 / a) as u8;
            out[o + 3] = (a / count) as u8;
        }
    }
    out
}
