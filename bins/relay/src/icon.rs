// Иконка relay: зелёная «плитка» с точкой и волнами — раздача/ретрансляция.
// Крупные формы — читается даже в трее 16×16.
//
// Один и тот же код рисует иконку окна, иконку трея (с цветом состояния) и
// `.ico` для exe: файл подключается как модуль (`mod icon;`) и через
// `include!` в `build.rs`. Поэтому здесь нет внешних зависимостей и
// inner-атрибутов.

/// Плитка: градиент сверху вниз.
const TILE_TOP: (u8, u8, u8) = (74, 186, 98);
const TILE_BOTTOM: (u8, u8, u8) = (28, 116, 52);
/// То же для состояния «остановлен».
const TILE_TOP_OFF: (u8, u8, u8) = (150, 156, 166);
const TILE_BOTTOM_OFF: (u8, u8, u8) = (96, 102, 112);
/// Точка-источник и «волны».
const MARK: (u8, u8, u8) = (255, 255, 255);

/// Иконка `size`×`size` в RGBA8 (для exe и окна — зелёная).
#[allow(dead_code)]
pub fn rgba(size: u32) -> Vec<u8> {
    rgba_state(size, true)
}

/// Иконка с цветом состояния: зелёная — работает, серая — остановлен.
#[allow(dead_code)]
pub fn rgba_state(size: u32, running: bool) -> Vec<u8> {
    let size = size.max(8);
    let ss = 4;
    let hi = size * ss;
    let mut buf = vec![0u8; (hi as usize) * (hi as usize) * 4];
    paint(&mut buf, hi, running);
    downscale(&buf, hi, size, ss)
}

fn paint(buf: &mut [u8], n: u32, running: bool) {
    let f = n as f32;
    let (top, bottom) = if running {
        (TILE_TOP, TILE_BOTTOM)
    } else {
        (TILE_TOP_OFF, TILE_BOTTOM_OFF)
    };

    // Плитка: скруглённый квадрат с градиентом.
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
            put(buf, n, x, y, mix(top, bottom, py / f));
        }
    }

    // Точка-источник слева и три «волны» вправо.
    let (cx, cy) = (f * 0.34, f * 0.50);
    for y in 0..n {
        for x in 0..n {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - cx;
            let dy = py - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let mut mark = d <= f * 0.10;
            if !mark {
                let ang = dy.atan2(dx).to_degrees().abs();
                if ang <= 55.0 {
                    for rr in [0.20_f32, 0.31, 0.42] {
                        if (d - f * rr).abs() <= f * 0.0425 {
                            mark = true;
                            break;
                        }
                    }
                }
            }
            if mark {
                put(buf, n, x, y, MARK);
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
