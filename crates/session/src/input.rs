//! Ввод и курсор: бинарные форматы поверх mux-каналов + геометрия экранов.
//!
//! Канал Input (reliable): пачки событий `[count u16 BE][событие...]`, где событие —
//! `[tag u8][поля BE]`:
//! - `1 Move`   — `[x u16][y u16]` (нормализованные 0..=65535 по области захвата);
//! - `2 Button` — `[btn u8][down u8][x u16][y u16]` (btn: 0 левая, 1 правая, 2 средняя, 3/4 X);
//! - `3 Wheel`  — `[dx i16][dy i16][x u16][y u16]`;
//! - `4 Key`    — `[vk u16][down u8]`.
//!
//! Канал Cursor (droppable): `[x u16][y u16][visible u8][kind u8]` — «последний выигрывает».
//! Байт `kind` добавлен позже: приёмник принимает и 5-байтовый пакет (тогда вид — стрелка).
//!
//! Meta-сообщения в канале Control: `[0xAA][type u8][...]` — служебная информация без
//! расширения protobuf v0. type 1 = SessionInfo:
//! `[ox i16][oy i16][w u16][h u16][vx i16][vy i16][vw i16][vh i16]`.

/// Магический первый байт meta-сообщений в канале Control.
pub const META_MAGIC: u8 = 0xAA;
/// Meta-тип: геометрия захвата + виртуальный рабочий стол.
pub const META_SESSION_INFO: u8 = 1;
/// Meta-тип: запрос вьюера «переключи захват на монитор N» (viewer → agent).
pub const META_SWITCH: u8 = 2;
/// Meta-тип: список мониторов агента (agent → viewer).
pub const META_MONITORS: u8 = 3;
/// Meta-тип: текст буфера обмена (в обе стороны).
pub const META_CLIP: u8 = 4;
/// Картинка системного курсора агента (RGBA + хотспот), канал Control.
pub const META_CURSOR_IMG: u8 = 5;

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
    /// Индекс активного монитора (DXGI output).
    pub active: u16,
}

/// Типы курсора (поле `kind` в канале Cursor). Неизвестное значение = обычная стрелка.
pub const CURSOR_ARROW: u8 = 0;
pub const CURSOR_IBEAM: u8 = 1;
pub const CURSOR_HAND: u8 = 2;
pub const CURSOR_CROSS: u8 = 3;
pub const CURSOR_WAIT: u8 = 4;
pub const CURSOR_APPSTARTING: u8 = 5;
pub const CURSOR_NO: u8 = 6;
pub const CURSOR_HELP: u8 = 7;
pub const CURSOR_SIZE_NS: u8 = 8;
pub const CURSOR_SIZE_WE: u8 = 9;
pub const CURSOR_SIZE_NWSE: u8 = 10;
pub const CURSOR_SIZE_NESW: u8 = 11;
pub const CURSOR_SIZE_ALL: u8 = 12;
pub const CURSOR_UPARROW: u8 = 13;

/// Состояние курсора агента (нормализованные координаты области захвата).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    /// Вид курсора, один из `CURSOR_*`; 0 (Default) — стрелка.
    pub kind: u8,
}

/// События ввода (координаты — нормализованные 0..=65535 по области захвата).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    Move { x: u16, y: u16 },
    Button { btn: u8, down: bool, x: u16, y: u16 },
    Wheel { dx: i16, dy: i16, x: u16, y: u16 },
    Key { vk: u16, down: bool },
}

/// Длина ТЕЛА события (без тега) в байтах.
fn event_len(tag: u8) -> Option<usize> {
    Some(match tag {
        1 => 4,
        2 => 6,
        3 => 8,
        4 => 3,
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
        if let InputEvent::Move { x, y } = e
            && let Some(InputEvent::Move { x: lx, y: ly }) = out.last_mut()
        {
            *lx = x;
            *ly = y;
            continue;
        }
        out.push(e);
    }
    out
}

/// Упаковать состояние курсора (канал Cursor).
pub fn pack_cursor(out: &mut [u8; 6], x: u16, y: u16, visible: bool, kind: u8) {
    out[0..2].copy_from_slice(&x.to_be_bytes());
    out[2..4].copy_from_slice(&y.to_be_bytes());
    out[4] = u8::from(visible);
    out[5] = kind;
}

/// Разобрать состояние курсора. Пакет без байта вида (старый агент) — стрелка.
pub fn parse_cursor(payload: &[u8]) -> Option<CursorState> {
    if payload.len() < 5 {
        return None;
    }
    Some(CursorState {
        x: u16::from_be_bytes([payload[0], payload[1]]),
        y: u16::from_be_bytes([payload[2], payload[3]]),
        visible: payload[4] != 0,
        kind: payload.get(5).copied().unwrap_or(CURSOR_ARROW),
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
    out.extend_from_slice(&si.active.to_be_bytes());
}

/// Разобрать SessionInfo (None — не meta или битые данные).
pub fn parse_session_info(payload: &[u8]) -> Option<SessionInfo> {
    // 18 байт — старый формат (без active), 20 — новый.
    if (payload.len() != 18 && payload.len() != 20)
        || payload[0] != META_MAGIC
        || payload[1] != META_SESSION_INFO
    {
        return None;
    }
    let i16b = |b: &[u8]| i16::from_be_bytes([b[0], b[1]]) as i32;
    let u16b = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]) as i32;
    let active = if payload.len() == 20 {
        u16::from_be_bytes([payload[18], payload[19]])
    } else {
        0
    };
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
        active,
    })
}

/// Упаковать запрос переключения монитора (вьюер → агент).
pub fn pack_switch(out: &mut [u8; 4], index: u16) {
    out[0] = META_MAGIC;
    out[1] = META_SWITCH;
    out[2..4].copy_from_slice(&index.to_be_bytes());
}

/// Разобрать запрос переключения монитора.
pub fn parse_switch(payload: &[u8]) -> Option<u16> {
    if payload.len() != 4 || payload[0] != META_MAGIC || payload[1] != META_SWITCH {
        return None;
    }
    Some(u16::from_be_bytes([payload[2], payload[3]]))
}

/// Упаковать список мониторов агента: (индекс, ширина, высота).
pub fn pack_monitors(out: &mut Vec<u8>, list: &[(u16, u16, u16)]) {
    out.push(META_MAGIC);
    out.push(META_MONITORS);
    let n = list.len().min(255);
    out.push(n as u8);
    for &(idx, w, h) in list.iter().take(n) {
        out.extend_from_slice(&idx.to_be_bytes());
        out.extend_from_slice(&w.to_be_bytes());
        out.extend_from_slice(&h.to_be_bytes());
    }
}

/// Упаковать текст буфера обмена (обе стороны; канал Control).
pub fn pack_clip(out: &mut Vec<u8>, text: &str) {
    out.push(META_MAGIC);
    out.push(META_CLIP);
    out.extend_from_slice(text.as_bytes());
}

/// Разобрать текст буфера обмена (не больше 128 КиБ).
pub fn parse_clip(payload: &[u8]) -> Option<String> {
    if payload.len() < 2 || payload[0] != META_MAGIC || payload[1] != META_CLIP {
        return None;
    }
    if payload.len() > 2 + 128 * 1024 {
        return None;
    }
    String::from_utf8(payload[2..].to_vec()).ok()
}

/// Картинка курсора агента: RGBA (сверху вниз) и хотспот.
pub struct CursorImg<'a> {
    pub w: u16,
    pub h: u16,
    pub hot_x: i16,
    pub hot_y: i16,
    /// Вид курсора (CURSOR_*), к которому относится картинка.
    pub kind: u8,
    pub rgba: &'a [u8],
}

/// Упаковать картинку курсора: `[0xAA][5][w u16][h u16][hot_x i16][hot_y i16][rgba…]`.
pub fn pack_cursor_img(
    out: &mut Vec<u8>,
    w: u16,
    h: u16,
    hot_x: i16,
    hot_y: i16,
    kind: u8,
    rgba: &[u8],
) {
    out.push(META_MAGIC);
    out.push(META_CURSOR_IMG);
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&hot_x.to_be_bytes());
    out.extend_from_slice(&hot_y.to_be_bytes());
    out.push(kind);
    out.extend_from_slice(rgba);
}

/// Разобрать картинку курсора (не больше 128×128).
pub fn parse_cursor_img(payload: &[u8]) -> Option<CursorImg<'_>> {
    if payload.len() < 11 || payload[0] != META_MAGIC || payload[1] != META_CURSOR_IMG {
        return None;
    }
    let w = u16::from_be_bytes([payload[2], payload[3]]);
    let h = u16::from_be_bytes([payload[4], payload[5]]);
    if w == 0 || h == 0 || w > 128 || h > 128 {
        return None;
    }
    let need = w as usize * h as usize * 4;
    if payload.len() < 11 + need {
        return None;
    }
    Some(CursorImg {
        w,
        h,
        hot_x: i16::from_be_bytes([payload[6], payload[7]]),
        hot_y: i16::from_be_bytes([payload[8], payload[9]]),
        kind: payload[10],
        rgba: &payload[11..11 + need],
    })
}

/// Разобрать список мониторов.
pub fn parse_monitors(payload: &[u8]) -> Option<Vec<(u16, u16, u16)>> {
    if payload.len() < 3 || payload[0] != META_MAGIC || payload[1] != META_MONITORS {
        return None;
    }
    let count = payload[2] as usize;
    if payload.len() != 3 + count * 6 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let b = &payload[3 + i * 6..9 + i * 6];
        out.push((
            u16::from_be_bytes([b[0], b[1]]),
            u16::from_be_bytes([b[2], b[3]]),
            u16::from_be_bytes([b[4], b[5]]),
        ));
    }
    Some(out)
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
        assert_eq!(buf.len(), 2 + 5 + 7 + 9 + 4);
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
        let mut buf = [0u8; 6];
        pack_cursor(&mut buf, 64000, 32, true, CURSOR_IBEAM);
        assert_eq!(
            parse_cursor(&buf).unwrap(),
            CursorState {
                x: 64000,
                y: 32,
                visible: true,
                kind: CURSOR_IBEAM,
            }
        );
        assert!(parse_cursor(&buf[..4]).is_none());
        // Пакет старого агента (5 байт) читается как обычная стрелка.
        assert_eq!(parse_cursor(&buf[..5]).unwrap().kind, CURSOR_ARROW);
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
            active: 1,
        };
        let mut buf = Vec::new();
        pack_session_info(&mut buf, si);
        assert_eq!(buf.len(), 20);
        assert_eq!(parse_session_info(&buf).unwrap(), si);
        // старый формат (18 байт, без active) по-прежнему принимаем
        let mut old_buf = Vec::new();
        pack_session_info(&mut old_buf, SessionInfo { active: 0, ..si });
        assert_eq!(parse_session_info(&old_buf[..18]).unwrap().active, 0);
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
    #[test]
    fn clip_roundtrip() {
        let mut buf = Vec::new();
        pack_clip(&mut buf, "привет, мир!");
        assert_eq!(parse_clip(&buf).unwrap(), "привет, мир!");
        assert!(parse_clip(&[0xAA, 4, 0xFF, 0xFE]).is_none());
        assert!(parse_clip(&[0xAA]).is_none());
        assert!(parse_clip(&[0x0A, 4, 1]).is_none());
    }

    #[test]
    fn cursor_img_roundtrip() {
        let rgba = vec![7u8; 2 * 2 * 4];
        let mut buf = Vec::new();
        pack_cursor_img(&mut buf, 2, 2, -1, 3, 5, &rgba);
        let img = parse_cursor_img(&buf).unwrap();
        assert_eq!(
            (img.w, img.h, img.hot_x, img.hot_y, img.kind),
            (2, 2, -1, 3, 5)
        );
        assert_eq!(img.rgba, &rgba[..]);
        assert!(parse_cursor_img(&buf[..10]).is_none());
        assert!(parse_cursor_img(&buf[..buf.len() - 1]).is_none());
    }

    #[test]
    fn switch_roundtrip() {
        let mut b = [0u8; 4];
        pack_switch(&mut b, 1234);
        assert_eq!(parse_switch(&b).unwrap(), 1234);
        assert!(parse_switch(&b[..3]).is_none());
        assert!(parse_switch(&[0, 1, 0, 2]).is_none());
    }

    #[test]
    fn monitors_roundtrip() {
        let list = vec![(0u16, 1920u16, 1080u16), (1, 2560, 1440)];
        let mut buf = Vec::new();
        pack_monitors(&mut buf, &list);
        assert_eq!(buf.len(), 3 + 12);
        assert_eq!(parse_monitors(&buf).unwrap(), list);
        assert!(parse_monitors(&buf[..buf.len() - 1]).is_none());
        assert!(parse_monitors(&[]).is_none());
    }
}
