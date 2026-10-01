//! Ввод и курсор: бинарные форматы поверх mux-каналов + геометрия экранов.
//!
//! Канал Input (reliable): пачки событий `[count u16 BE][событие...]`, где событие —
//! `[tag u8][поля BE]`:
//! - `1 Move`   — `[x u16][y u16]` (нормализованные 0..=65535 по области захвата);
//! - `2 Button` — `[btn u8][down u8][x u16][y u16]` (btn: 0 левая, 1 правая, 2 средняя, 3/4 X);
//! - `3 Wheel`  — `[dx i16][dy i16][x u16][y u16]`;
//! - `4 Key`    — `[vk u16][down u8]`.
//!
//! Канал Cursor (droppable): `[x u16][y u16][visible u8]` — «последний выигрывает».
//!
//! Meta-сообщения в канале Control: `[0xAA][type u8][...]` — служебная информация без
//! расширения protobuf v0. type 1 = SessionInfo:
//! `[ox i16][oy i16][w u16][h u16][vx i16][vy i16][vw i16][vh i16]`.

/// Магический первый байт meta-сообщений в канале Control.
pub const META_MAGIC: u8 = 0xAA;
/// Meta-тип: геометрия захвата + виртуальный рабочий стол.
pub const META_SESSION_INFO: u8 = 1;

/// Прямоугольник в экранных координатах.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Геометрия захваченного вывода (монитора) внутри виртуального рабочего стола.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputGeom {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: i32,
    pub height: i32,
}

/// SessionInfo: что захватывает агент и каково виртуальное пространство ввода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionInfo {
    pub out: OutputGeom,
    pub virt: RectI,
}

/// Состояние курсора агента (нормализованные координаты области захвата).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
}

/// События ввода (координаты — нормализованные 0..=65535 по области захвата).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Move { x: u16, y: u16 },
    Button { btn: u8, down: bool, x: u16, y: u16 },
    Wheel { dx: i16, dy: i16, x: u16, y: u16 },
    Key { vk: u16, down: bool },
}

fn event_len(tag: u8) -> Option<usize> {
    Some(match tag {
        1 => 5,
        2 => 7,
        3 => 9,
        4 => 5,
        _ => return None,
    })
}

/// Упаковать пачку событий.
pub fn pack_batch(out: &mut Vec<u8>, events: &[InputEvent]) {
    out.reserve(2 + events.len() * 9);
    out.extend_from_slice(&(events.len() as u16).to_be_bytes());
    for e in events {
        match *e {
            InputEvent::Move { x, y } => {
                out.push(1);
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            InputEvent::Button { btn, down, x, y } => {
                out.push(2);
                out.push(btn);
                out.push(u8::from(down));
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            InputEvent::Wheel { dx, dy, x, y } => {
                out.push(3);
                out.extend_from_slice(&dx.to_be_bytes());
                out.extend_from_slice(&dy.to_be_bytes());
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            InputEvent::Key { vk, down } => {
                out.push(4);
                out.extend_from_slice(&vk.to_be_bytes());
                out.push(u8::from(down));
            }
        }
    }
}

/// Разобрать пачку событий (None — битые данные).
pub fn parse_batch(payload: &[u8]) -> Option<Vec<InputEvent>> {
    if payload.len() < 2 {
        return None;
    }
    let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    let mut events = Vec::with_capacity(count.min(4096));
    let mut i = 2;
    for _ in 0..count {
        let tag = *payload.get(i)?;
        let len = event_len(tag)?;
        let body = payload.get(i + 1..i + 1 + len)?;
        let e = match tag {
            1 => InputEvent::Move {
                x: u16::from_be_bytes([body[0], body[1]]),
                y: u16::from_be_bytes([body[2], body[3]]),
            },
            2 => InputEvent::Button {
                btn: body[0],
                down: body[1] != 0,
                x: u16::from_be_bytes([body[2], body[3]]),
                y: u16::from_be_bytes([body[4], body[5]]),
            },
            3 => InputEvent::Wheel {
                dx: i16::from_be_bytes([body[0], body[1]]),
                dy: i16::from_be_bytes([body[2], body[3]]),
                x: u16::from_be_bytes([body[4], body[5]]),
                y: u16::from_be_bytes([body[6], body[7]]),
            },
            4 => InputEvent::Key {
                vk: u16::from_be_bytes([body[0], body[1]]),
                down: body[2] != 0,
            },
            _ => unreachable!(),
        };
        events.push(e);
        i += 1 + len;
    }
    Some(events)
}

/// Коалессинг: подряд идущие `Move` схлопываются в последний.
pub fn coalesce_moves(events: Vec<InputEvent>) -> Vec<InputEvent> {
    let mut out: Vec<InputEvent> = Vec::with_capacity(events.len());
    for e in events {
        if let InputEvent::Move { x, y } = e {
            if let Some(InputEvent::Move { x: lx, y: ly }) = out.last_mut() {
                *lx = x;
                *ly = y;
                continue;
            }
        }
        out.push(e);
    }
    out
}

/// Упаковать состояние курсора (канал Cursor).
pub fn pack_cursor(out: &mut [u8; 5], x: u16, y: u16, visible: bool) {
    out[0..2].copy_from_slice(&x.to_be_bytes());
    out[2..4].copy_from_slice(&y.to_be_bytes());
    out[4] = u8::from(visible);
}

/// Разобрать состояние курсора.
pub fn parse_cursor(payload: &[u8]) -> Option<CursorState> {
    if payload.len() < 5 {
        return None;
    }
    Some(CursorState {
        x: u16::from_be_bytes([payload[0], payload[1]]),
        y: u16::from_be_bytes([payload[2], payload[3]]),
        visible: payload[4] != 0,
    })
}

/// Упаковать SessionInfo (meta в канале Control).
pub fn pack_session_info(out: &mut Vec<u8>, si: SessionInfo) {
    out.push(META_MAGIC);
    out.push(META_SESSION_INFO);
    out.extend_from_slice(&(si.out.origin_x as i16).to_be_bytes());
    out.extend_from_slice(&(si.out.origin_y as i16).to_be_bytes());
    out.extend_from_slice(&(si.out.width as u16).to_be_bytes());
    out.extend_from_slice(&(si.out.height as u16).to_be_bytes());
    out.extend_from_slice(&(si.virt.x as i16).to_be_bytes());
    out.extend_from_slice(&(si.virt.y as i16).to_be_bytes());
    out.extend_from_slice(&(si.virt.w as i16).to_be_bytes());
    out.extend_from_slice(&(si.virt.h as i16).to_be_bytes());
}

/// Разобрать SessionInfo (None — не meta или битые данные).
pub fn parse_session_info(payload: &[u8]) -> Option<SessionInfo> {
    if payload.len() != 18 || payload[0] != META_MAGIC || payload[1] != META_SESSION_INFO {
        return None;
    }
    let i16b = |b: &[u8]| i16::from_be_bytes([b[0], b[1]]) as i32;
    let u16b = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]) as i32;
    Some(SessionInfo {
        out: OutputGeom {
            origin_x: i16b(&payload[2..4]),
            origin_y: i16b(&payload[4..6]),
            width: u16b(&payload[6..8]),
            height: u16b(&payload[8..10]),
        },
        virt: RectI {
            x: i16b(&payload[10..12]),
            y: i16b(&payload[12..14]),
            w: i16b(&payload[14..16]),
            h: i16b(&payload[16..18]),
        },
    })
}

/// Нормализованные координаты области захвата → экранные пиксели агента.
pub fn captured_norm_to_screen(nx: u16, ny: u16, out: OutputGeom) -> (i32, i32) {
    if out.width <= 0 || out.height <= 0 {
        return (out.origin_x, out.origin_y);
    }
    let px = out.origin_x + ((nx as i64 * out.width as i64) / 65536) as i32;
    let py = out.origin_y + ((ny as i64 * out.height as i64) / 65536) as i32;
    (
        px.min(out.origin_x + out.width - 1),
        py.min(out.origin_y + out.height - 1),
    )
}

/// Экранные пиксели агента → нормализованные координаты области захвата.
pub fn screen_to_captured_norm(px: i32, py: i32, out: OutputGeom) -> (u16, u16) {
    if out.width <= 1 || out.height <= 1 {
        return (0, 0);
    }
    let nx = (((px - out.origin_x) as i64) * 65535 / (out.width as i64 - 1)).clamp(0, 65535) as u16;
    let ny =
        (((py - out.origin_y) as i64) * 65535 / (out.height as i64 - 1)).clamp(0, 65535) as u16;
    (nx, ny)
}

/// Нормализованные координаты области захвата → нормализованные координаты
/// виртуального рабочего стола (для `SendInput` с `MOUSEEVENTF_VIRTUALDESK`).
pub fn captured_norm_to_virtual_norm(nx: u16, ny: u16, out: OutputGeom, virt: RectI) -> (u16, u16) {
    if virt.w <= 1 || virt.h <= 1 {
        return (0, 0);
    }
    let (px, py) = captured_norm_to_screen(nx, ny, out);
    let vx = (((px - virt.x) as i64) * 65535 / (virt.w as i64 - 1)).clamp(0, 65535) as u16;
    let vy = (((py - virt.y) as i64) * 65535 / (virt.h as i64 - 1)).clamp(0, 65535) as u16;
    (vx, vy)
}

/// Позиция в UI (пиксели внутри отрисованного видео) → нормализованные координаты захвата.
pub fn ui_px_to_norm(px: f32, py: f32, w: f32, h: f32) -> (u16, u16) {
    let cl = |v: f32, m: f32| (v / m * 65535.0).round().clamp(0.0, 65535.0) as u16;
    (cl(px, w.max(1.0)), cl(py, h.max(1.0)))
}

/// Нормализованные координаты захвата → позиция в UI (для отрисовки курсора).
pub fn norm_to_ui_px(nx: u16, ny: u16, w: f32, h: f32) -> (f32, f32) {
    (nx as f32 / 65535.0 * w, ny as f32 / 65535.0 * h)
}

/// Символ → (VK, shift, ctrl, alt) по текущей раскладке системы. None — прямого VK нет.
#[cfg(windows)]
pub fn vk_for_char(ch: char) -> Option<(u16, bool, bool, bool)> {
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;
        let r = VkKeyScanW(ch as u16);
        if r < 0 {
            return None;
        }
        let st = ((r >> 8) & 0xFF) as u8;
        Some(((r & 0xFF) as u16, st & 1 != 0, st & 2 != 0, st & 4 != 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_roundtrip() {
        let evs = vec![
            InputEvent::Move { x: 100, y: 200 },
            InputEvent::Button {
                btn: 0,
                down: true,
                x: 32768,
                y: 32768,
            },
            InputEvent::Wheel {
                dx: -120,
                dy: 240,
                x: 1,
                y: 2,
            },
            InputEvent::Key {
                vk: 0x41,
                down: false,
            },
        ];
        let mut buf = Vec::new();
        pack_batch(&mut buf, &evs);
        assert_eq!(buf.len(), 2 + 5 + 7 + 9 + 5);
        assert_eq!(parse_batch(&buf).unwrap(), evs);
    }

    #[test]
    fn batch_garbage_is_none() {
        let mut buf = Vec::new();
        pack_batch(&mut buf, &[InputEvent::Move { x: 1, y: 2 }]);
        assert!(parse_batch(&buf[..buf.len() - 1]).is_none());
        assert!(parse_batch(&[0, 9, 1, 0, 0]).is_none());
        assert!(parse_batch(&[]).is_none());
    }

    #[test]
    fn coalesce() {
        let evs = vec![
            InputEvent::Move { x: 1, y: 1 },
            InputEvent::Move { x: 2, y: 2 },
            InputEvent::Button {
                btn: 0,
                down: true,
                x: 2,
                y: 2,
            },
            InputEvent::Move { x: 3, y: 3 },
            InputEvent::Move { x: 4, y: 4 },
        ];
        let out = coalesce_moves(evs);
        assert_eq!(
            out,
            vec![
                InputEvent::Move { x: 2, y: 2 },
                InputEvent::Button {
                    btn: 0,
                    down: true,
                    x: 2,
                    y: 2
                },
                InputEvent::Move { x: 4, y: 4 },
            ]
        );
    }

    #[test]
    fn cursor_roundtrip() {
        let mut buf = [0u8; 5];
        pack_cursor(&mut buf, 64000, 32, true);
        assert_eq!(
            parse_cursor(&buf).unwrap(),
            CursorState {
                x: 64000,
                y: 32,
                visible: true
            }
        );
        assert!(parse_cursor(&buf[..4]).is_none());
    }

    #[test]
    fn session_info_roundtrip() {
        let si = SessionInfo {
            out: OutputGeom {
                origin_x: 1920,
                origin_y: 0,
                width: 1920,
                height: 1080,
            },
            virt: RectI {
                x: 0,
                y: 0,
                w: 3840,
                h: 1080,
            },
        };
        let mut buf = Vec::new();
        pack_session_info(&mut buf, si);
        assert_eq!(buf.len(), 18);
        assert_eq!(parse_session_info(&buf).unwrap(), si);
        // protobuf-подобные данные не распознаются как meta
        assert!(
            parse_session_info(&[
                0x0A, 0x05, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16
            ])
            .is_none()
        );
    }

    #[test]
    fn mapping_multimonitor() {
        let out = OutputGeom {
            origin_x: 1920,
            origin_y: 0,
            width: 1920,
            height: 1080,
        };
        let virt = RectI {
            x: 0,
            y: 0,
            w: 3840,
            h: 1080,
        };
        // левый верх второго монитора — правая половина виртуального стола
        let (vx, _vy) = captured_norm_to_virtual_norm(0, 0, out, virt);
        assert!(vx > 32000, "второй монитор — правая половина: {vx}");
        // правый нижний угол второго монитора — край виртуального стола
        let (vx, vy) = captured_norm_to_virtual_norm(65535, 65535, out, virt);
        assert_eq!((vx, vy), (65535, 65535));
        // обратное отображение курсора
        assert_eq!(screen_to_captured_norm(1920, 0, out), (0, 0));
        assert_eq!(screen_to_captured_norm(3839, 1079, out), (65535, 65535));
    }

    #[test]
    fn ui_norm_roundtrip() {
        let (nx, ny) = ui_px_to_norm(640.0, 360.0, 1280.0, 720.0);
        assert!((nx as i32 - 32768).abs() < 2 && (ny as i32 - 32768).abs() < 2);
        let (px, py) = norm_to_ui_px(nx, ny, 1280.0, 720.0);
        assert!((px - 640.0).abs() < 1.0 && (py - 360.0).abs() < 1.0);
    }
}
