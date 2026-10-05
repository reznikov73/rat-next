// Иконка приложения Rat Next: синяя «плитка» с крупным белым курсором и
// зелёной точкой активной сессии. Крупные формы — читается даже в трее 16×16.
//
// Один и тот же код рисует иконку окна, иконку трея и `.ico` для exe: файл
// подключается как модуль приложения (`mod icon;`) и через `include!` в
// `build.rs`. Поэтому здесь нет внешних зависимостей и inner-атрибутов.

/// Плитка: градиент сверху вниз.
const TILE_TOP: (u8, u8, u8) = (90, 152, 255);
const TILE_BOTTOM: (u8, u8, u8) = (38, 88, 216);
/// Курсор.
const CURSOR: (u8, u8, u8) = (255, 255, 255);
/// Индикатор активной сессии.
const LIVE: (u8, u8, u8) = (76, 175, 80);
/// Тёмная окантовка точки сессии, чтобы не сливалась с плиткой.
const LIVE_RING: (u8, u8, u8) = (28, 62, 140);

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

    // Плитка: скруглённый квадрат с синим градиентом.
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
            put(buf, n, x, y, mix(TILE_TOP, TILE_BOTTOM, py / f));
        }
    }

    // Крупный белый курсор — главный элемент, читается в любом размере.
    let cursor = [
        (0.00_f32, 0.00_f32),
        (0.00, 0.94),
        (0.22, 0.74),
        (0.35, 1.00),
        (0.47, 0.93),
        (0.35, 0.68),
        (0.58, 0.64),
    ];
    let scale = f * 0.60;
    let (ox, oy) = (f * 0.24, f * 0.16);
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

    // Точка сессии — только от 24 логических px (hi = size × 4), иначе
    // на мелких значках сливается в пятно.
    if f >= 96.0 {
        let (cx, cy, rad) = (f * 0.84, f * 0.84, f * 0.12);
        for y in 0..n {
            for x in 0..n {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                if d <= rad {
                    put(buf, n, x, y, LIVE);
                } else if d <= rad + f * 0.03 {
                    put(buf, n, x, y, LIVE_RING);
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
