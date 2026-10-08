//! Диагностика доставки ввода (в том числе на защищённом столе).
//!
//! Подключается к агенту по адресу, читает геометрию сессии и позицию курсора
//! из канала Cursor, шлёт пару движений мыши и проверяет, что курсор следует
//! за событиями. Смещения небольшие (относительно текущей позиции), чтобы не
//! дёргать курсор без нужды; в конце курсор возвращается назад.
//!
//! Запуск: `rat.exe --input-check[=адрес]` (по умолчанию 127.0.0.1:55140).

use crate::input::{self, CursorState, InputEvent, OutputGeom, SessionInfo};
use crate::link;
use rat_mux::{Channel, Decoder, Encoder};
use std::time::{Duration, Instant};

/// Допуск сверки в нормализованных единицах (0..=65535 на ширину экрана).
const TOL: i32 = 700;

pub fn run(addr: &str) -> Result<String, String> {
    let (link, closer) =
        link::tcp_connect(addr).map_err(|e| format!("подключение к {addr}: {e}"))?;
    let mut dec = Decoder::new();
    let mut out = Encoder::new();
    let mut info: Option<SessionInfo> = None;
    let mut cur: Option<CursorState> = None;

    // 1) Ждём геометрию сессии (агент шлёт её сразу после подключения).
    let t0 = Instant::now();
    while info.is_none() && t0.elapsed() < Duration::from_secs(8) {
        pump(
            &link,
            &mut dec,
            &mut info,
            &mut cur,
            Duration::from_millis(100),
        );
    }
    let geo =
        info.ok_or_else(|| "агент не прислал геометрию сессии (нет SessionInfo)".to_string())?;
    let g: OutputGeom = geo.out;

    // Начальная позиция курсора (для относительных сдвигов).
    let t1 = Instant::now();
    while cur.is_none() && t1.elapsed() < Duration::from_secs(3) {
        pump(
            &link,
            &mut dec,
            &mut info,
            &mut cur,
            Duration::from_millis(100),
        );
    }
    let start = cur.unwrap_or(CursorState {
        x: 32768,
        y: 32768,
        visible: true,
        kind: 0,
    });

    // 2) Цели: небольшие сдвиги от текущей позиции.
    let ax = (start.x as i32 + 3000).clamp(1000, 64535) as u16;
    let ay = (start.y as i32 + 2200).clamp(1000, 64535) as u16;
    let bx = (start.x as i32 - 3000).clamp(1000, 64535) as u16;
    let by = (start.y as i32 - 2200).clamp(1000, 64535) as u16;

    send_move(&link, &mut out, ax, ay)?;
    let after_a = sample(
        &link,
        &mut dec,
        &mut info,
        &mut cur,
        Duration::from_millis(1500),
    );
    send_move(&link, &mut out, bx, by)?;
    let after_b = sample(
        &link,
        &mut dec,
        &mut info,
        &mut cur,
        Duration::from_millis(1500),
    );

    // Возврат курсора на исходную позицию (лучшее усилие).
    let _ = send_move(&link, &mut out, start.x, start.y);
    pump(
        &link,
        &mut dec,
        &mut info,
        &mut cur,
        Duration::from_millis(200),
    );

    let ok_a = after_a.is_some_and(|c| close_enough(c, ax, ay));
    let ok_b = after_b.is_some_and(|c| close_enough(c, bx, by));
    let moved = match (after_a, after_b) {
        (Some(a), Some(b)) => {
            (a.x as i32 - b.x as i32).abs() + (a.y as i32 - b.y as i32).abs() > 1500
        }
        _ => false,
    };

    let verdict = if ok_a && ok_b {
        "ПРИНЯТ: курсор следует за отправленными движениями".to_string()
    } else if moved {
        format!("ЧАСТИЧНО: курсор двигался, но координаты не совпали с целями (допуск {TOL})")
    } else if after_a.is_none() && after_b.is_none() {
        "НЕТ ДАННЫХ: агент не сообщил позицию курсора".to_string()
    } else {
        "НЕ ПРИНЯТ: курсор не реагирует на движения (ввод не доставляется)".to_string()
    };

    let show = |c: Option<CursorState>| match c {
        Some(c) => format!("({},{}) виден={}", c.x, c.y, c.visible),
        None => "нет данных".to_string(),
    };

    let report = format!(
        "ввод-проверка {addr}\n\
         геометрия: вывод=({},{},{}x{}) виртуальный=({},{},{}x{})\n\
         старт курсора: {}\n\
         цель A=({ax},{ay}) -> курсор после A: {}\n\
         цель B=({bx},{by}) -> курсор после B: {}\n\
         вердикт: {verdict}",
        g.origin_x,
        g.origin_y,
        g.width,
        g.height,
        geo.virt.x,
        geo.virt.y,
        geo.virt.w,
        geo.virt.h,
        show(Some(start)),
        show(after_a),
        show(after_b),
    );
    drop(link);
    closer.close();
    Ok(report)
}

fn close_enough(c: CursorState, tx: u16, ty: u16) -> bool {
    (c.x as i32 - tx as i32).abs() <= TOL && (c.y as i32 - ty as i32).abs() <= TOL
}

fn send_move(link: &link::Duplex, out: &mut Encoder, x: u16, y: u16) -> Result<(), String> {
    let mut buf = Vec::with_capacity(6);
    input::pack_batch(&mut buf, &[InputEvent::Move { x, y }]);
    out.push(Channel::Input, 0, &buf);
    link.tx
        .send(out.take_bytes())
        .map_err(|_| "линк закрыт при отправке ввода".to_string())
}

/// Читаем кадры ~`dur` и возвращаем последнюю известную позицию курсора.
fn sample(
    link: &link::Duplex,
    dec: &mut Decoder,
    info: &mut Option<SessionInfo>,
    cur: &mut Option<CursorState>,
    dur: Duration,
) -> Option<CursorState> {
    let t = Instant::now();
    while t.elapsed() < dur {
        pump(link, dec, info, cur, Duration::from_millis(50));
    }
    *cur
}

/// Забрать доступные байты линка и разобрать кадры (с таймаутом ожидания).
fn pump(
    link: &link::Duplex,
    dec: &mut Decoder,
    info: &mut Option<SessionInfo>,
    cur: &mut Option<CursorState>,
    wait: Duration,
) {
    match link.rx.recv_timeout(wait) {
        Ok(bytes) => dec.feed(&bytes),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
    }
    loop {
        match dec.poll_frame() {
            Ok(Some(f)) => {
                if f.channel == Channel::Control {
                    if info.is_none()
                        && let Some(si) = input::parse_session_info(f.payload)
                    {
                        *info = Some(si);
                    }
                } else if f.channel == Channel::Cursor
                    && let Some(c) = input::parse_cursor(f.payload)
                {
                    *cur = Some(c);
                }
            }
            Ok(None) => break,
            Err(_) => {
                dec.reset();
                break;
            }
        }
    }
}
