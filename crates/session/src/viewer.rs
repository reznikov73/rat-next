//! Вьюерская сторона сессии: приём mux-кадров → H.264 декод → кадры в UI.
//!
//! Control: Hello → старт пингов; Pong → RTT. Video: мета + Annex-B → декод →
//! RGBA → `ui_tx` (SyncSender с малым буфером; переполнение = drop «самого старого»).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rat_mux::{Channel, Encoder as MuxEncoder};
use rat_proto::rat::v0::envelope::Body;

use crate::control;
use crate::ffmpeg::{Decoder, Sws, pix_fmt, sws_yuv_to_rgba};
use crate::frame;
use crate::input::{self, CursorState, InputEvent, SessionInfo};
use crate::link::Duplex;
use crate::stats::{SessionStats, TimeBase, now_us};

/// Готовый кадр для UI (RGBA8).
pub struct FrameOut {
    pub id: u64,
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub t_capture_us: u64,
}

fn send_ping(out: &mut MuxEncoder, id: u64, pending: &mut HashMap<u64, Instant>) {
    let env = control::ping(id, id);
    out.push(Channel::Control, 0, &control::encode_env(&env));
    pending.insert(id, Instant::now());
}

/// Картинка курсора агента: размеры, хотспот, вид и RGBA-пиксели.
#[derive(Clone)]
pub struct CursorImage {
    pub w: u16,
    pub h: u16,
    pub hot_x: i16,
    pub hot_y: i16,
    /// Вид курсора (CURSOR_*), к которому относится картинка.
    pub kind: u8,
    pub rgba: Vec<u8>,
}

/// Общее состояние вьюерской стороны: очередь ввода из UI, курсор и геометрия от агента.
pub struct ViewerShared {
    pub input_q: Arc<Mutex<Vec<InputEvent>>>,
    pub cursor: Arc<Mutex<CursorState>>,
    pub info: Arc<Mutex<Option<SessionInfo>>>,
    /// Список мониторов агента: (индекс, ширина, высота).
    pub monitors: Arc<Mutex<Vec<(u16, u16, u16)>>>,
    /// Запрос на переключение монитора (последний выигрывает).
    pub pending_switch: Arc<Mutex<Option<u16>>>,
    /// Последний текст буфера обмена от агента (забрать в локальный буфер).
    pub clip_in: Arc<Mutex<Option<String>>>,
    /// Текст для вставки на удалённой машине (из локального буфера).
    pub pending_paste: Arc<Mutex<Option<String>>>,
    /// Картинка системного курсора агента (см. [`CursorImage`]).
    pub cursor_img: Arc<Mutex<Option<CursorImage>>>,
    /// Версия картинки курсора (меняется при каждом обновлении).
    pub cursor_img_ver: Arc<std::sync::atomic::AtomicU64>,
}

/// Запустить вьюерскую половину сессии.
pub fn spawn_viewer(
    base: TimeBase,
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<SessionStats>>,
    link: Duplex,
    ui_tx: SyncSender<FrameOut>,
    shared: ViewerShared,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        set_above_normal();

        let mut mux = rat_mux::Decoder::new();
        let mut dec = match Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                let mut st = stats.lock().unwrap();
                st.viewer_errors += 1;
                st.last_error = e;
                return;
            }
        };
        let mut sws_out: Option<Sws> = None;
        let mut sws_dims: Option<(usize, usize)> = None;
        let mut rgba: Vec<u8> = Vec::new();
        let mut out = MuxEncoder::new();
        let mut pending: HashMap<u64, Instant> = HashMap::new();
        let mut next_ping: u64 = 1;
        let mut last_ping = Instant::now() - Duration::from_secs(2);
        let mut hello_seen = false;
        let mut fps_t = Instant::now();
        let mut fps_count: u32 = 0;
        // Диагностика «залипаний» курсора (Шаг 0): время последнего принятого
        // пакета курсора; «пауза доставки» считается только если пользователь
        // в это время действительно двигал мышь (по счётчику движений).
        let mut last_cursor_rx: Option<Instant> = None;
        let mut first_cursor_rx: Option<Instant> = None;
        let mut moves_since_pkt: u64 = 0;

        while !stop.load(Ordering::Relaxed) {
            match link.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(bytes) => {
                    stats.lock().unwrap().recv_bytes += bytes.len() as u64;
                    mux.feed(&bytes);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            let mut reset_mux = false;
            loop {
                match mux.poll_frame() {
                    Ok(Some(f)) => match f.channel {
                        Channel::Control => {
                            if f.payload.first() == Some(&input::META_MAGIC) {
                                if let Some(si) = input::parse_session_info(f.payload) {
                                    *shared.info.lock().unwrap() = Some(si);
                                } else if let Some(list) = input::parse_monitors(f.payload) {
                                    *shared.monitors.lock().unwrap() = list;
                                } else if let Some(text) = input::parse_clip(f.payload) {
                                    *shared.clip_in.lock().unwrap() = Some(text);
                                } else if let Some(img) = input::parse_cursor_img(f.payload) {
                                    *shared.cursor_img.lock().unwrap() = Some(CursorImage {
                                        w: img.w,
                                        h: img.h,
                                        hot_x: img.hot_x,
                                        hot_y: img.hot_y,
                                        kind: img.kind,
                                        rgba: img.rgba.to_vec(),
                                    });
                                    shared
                                        .cursor_img_ver
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            } else if let Some(env) = control::decode_env(f.payload) {
                                stats.lock().unwrap().control_msgs += 1;
                                match env.body {
                                    Some(Body::Hello(_)) => {
                                        hello_seen = true;
                                        send_ping(&mut out, next_ping, &mut pending);
                                        next_ping += 1;
                                        last_ping = Instant::now();
                                    }
                                    Some(Body::Pong(_)) => {
                                        if let Some(t) = pending.remove(&env.request_id) {
                                            let rtt = t.elapsed().as_secs_f32() * 1000.0;
                                            let mut st = stats.lock().unwrap();
                                            SessionStats::push_sample(&mut st.rtt_ms, rtt);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        Channel::Video => {
                            let Some((id, ts_us, data)) = frame::parse(f.payload) else {
                                continue;
                            };

                            let mut sent = false;
                            for _ in 0..3 {
                                match dec.send(data) {
                                    Ok(0) => {
                                        sent = true;
                                        break;
                                    }
                                    Ok(1) => {
                                        let _ = dec.recv();
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        let mut st = stats.lock().unwrap();
                                        st.viewer_errors += 1;
                                        st.last_error = e;
                                        break;
                                    }
                                }
                            }
                            if !sent {
                                continue;
                            }

                            loop {
                                let gotf = match dec.recv() {
                                    Ok(v) => v,
                                    Err(e) => {
                                        let mut st = stats.lock().unwrap();
                                        st.viewer_errors += 1;
                                        st.last_error = e;
                                        break;
                                    }
                                };
                                if !gotf {
                                    break;
                                }
                                if dec.pix_fmt() != pix_fmt(2) {
                                    let mut st = stats.lock().unwrap();
                                    st.last_error = format!("unexpected pix_fmt {}", dec.pix_fmt());
                                    st.viewer_errors += 1;
                                    break;
                                }
                                if let Some(pv) = dec.planes() {
                                    // Пересоздаём масштабирование при смене разрешения
                                    // (например, после переключения монитора).
                                    if sws_dims != Some((pv.w, pv.h)) {
                                        match Sws::new(
                                            pv.w as u32,
                                            pv.h as u32,
                                            pix_fmt(2),
                                            pix_fmt(3),
                                        ) {
                                            Ok(s) => {
                                                sws_out = Some(s);
                                                sws_dims = Some((pv.w, pv.h));
                                            }
                                            Err(e) => {
                                                let mut st = stats.lock().unwrap();
                                                st.viewer_errors += 1;
                                                st.last_error = e;
                                                break;
                                            }
                                        }
                                        rgba = vec![0u8; pv.w * pv.h * 4];
                                    }
                                    unsafe {
                                        sws_yuv_to_rgba(
                                            sws_out.as_ref().unwrap().ptr(),
                                            pv.y,
                                            pv.y_stride,
                                            pv.u,
                                            pv.u_stride,
                                            pv.v,
                                            pv.v_stride,
                                            rgba.as_mut_ptr(),
                                            pv.w * 4,
                                            pv.h,
                                        );
                                    }
                                    let e2e = ((now_us(&base) as i64 - ts_us as i64).max(0)) as f32
                                        / 1000.0;
                                    {
                                        let mut st = stats.lock().unwrap();
                                        SessionStats::push_sample(&mut st.e2e_ms, e2e);
                                        st.shown += 1;
                                    }
                                    fps_count += 1;

                                    let out_rgba =
                                        std::mem::replace(&mut rgba, vec![0u8; pv.w * pv.h * 4]);
                                    let frame = FrameOut {
                                        id,
                                        w: pv.w,
                                        h: pv.h,
                                        rgba: out_rgba,
                                        t_capture_us: ts_us,
                                    };
                                    if ui_tx.try_send(frame).is_err() {
                                        stats.lock().unwrap().viewer_dropped += 1;
                                    }
                                }
                            }
                        }
                        Channel::Cursor => {
                            if let Some(c) = input::parse_cursor(f.payload) {
                                let now = Instant::now();
                                // Прогрев: первые 2 с после первого пакета не
                                // считаем паузами (сессия ещё «раскручивается»).
                                let warm = first_cursor_rx.is_some_and(|t| {
                                    now.duration_since(t) >= Duration::from_secs(2)
                                });
                                first_cursor_rx.get_or_insert(now);
                                let moves = moves_since_pkt;
                                moves_since_pkt = 0;
                                if let Some(gap_ms) = note_cursor_gap(
                                    last_cursor_rx,
                                    now,
                                    moves,
                                    warm,
                                    &mut stats.lock().unwrap(),
                                ) {
                                    crate::log::log_line(&format!(
                                        "[net] курсор: пауза доставки {gap_ms:.0} мс (движений за паузу: {moves})"
                                    ));
                                }
                                last_cursor_rx = Some(now);
                                *shared.cursor.lock().unwrap() = c;
                            }
                        }
                        _ => {}
                    },
                    Ok(None) => break,
                    Err(_) => {
                        reset_mux = true;
                        break;
                    }
                }
            }
            if reset_mux {
                mux.reset();
                stats.lock().unwrap().viewer_errors += 1;
            }

            if hello_seen && last_ping.elapsed() >= Duration::from_secs(1) && pending.len() < 8 {
                send_ping(&mut out, next_ping, &mut pending);
                next_ping += 1;
                last_ping = Instant::now();
            }
            // Запрос переключения монитора от UI (последний выигрывает).
            let sw = shared.pending_switch.lock().unwrap().take();
            if let Some(idx) = sw {
                let mut b = [0u8; 4];
                input::pack_switch(&mut b, idx);
                out.push(Channel::Control, 0, &b);
            }
            // Вставка из локального буфера: сначала текст (агент положит его в свой
            // буфер), затем клавиши Ctrl+V — они уже в очереди ввода.
            let paste = shared.pending_paste.lock().unwrap().take();
            if let Some(text) = paste {
                let mut buf = Vec::with_capacity(text.len() + 2);
                input::pack_clip(&mut buf, &text);
                out.push(Channel::Control, 0, &buf);
            }
            // Отправить накопленный ввод (с коалессингом движений мыши).
            let batch: Vec<InputEvent> = std::mem::take(&mut *shared.input_q.lock().unwrap());
            if !batch.is_empty() {
                let coalesced = input::coalesce_moves(batch);
                let mut buf = Vec::with_capacity(2 + coalesced.len() * 9);
                input::pack_batch(&mut buf, &coalesced);
                out.push(Channel::Input, 0, &buf);
                stats.lock().unwrap().input_sent += coalesced.len() as u64;
                moves_since_pkt += coalesced.len() as u64;
            }
            if !out.is_empty() && link.tx.send(out.take_bytes()).is_err() {
                break;
            }

            let dt = fps_t.elapsed().as_secs_f32();
            if dt >= 1.0 {
                stats.lock().unwrap().fps = fps_count as f32 / dt;
                fps_count = 0;
                fps_t = Instant::now();
            }
        }
    })
}

/// Учесть паузу между пакетами курсора (диагностика «залипаний», Шаг 0):
/// считаем только когда ввод недавно отправлялся (иначе курсор просто стоял
/// на месте). Возвращает длину значимой паузы в мс.
fn note_cursor_gap(
    prev: Option<Instant>,
    now: Instant,
    moves_during: u64,
    warm: bool,
    st: &mut SessionStats,
) -> Option<f32> {
    let prev = prev?;
    if !warm {
        return None;
    }
    let gap_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
    if gap_ms <= 250.0 {
        return None;
    }
    // Считаем паузой доставки только если движения шли всё это время:
    // порог растёт с длиной паузы, обычные «помолчал и сдвинул мышь»
    // (мало движений за паузу) отсекаются.
    if (moves_during as f32) < 8.0 + gap_ms / 20.0 {
        return None;
    }
    st.cursor_gaps += 1;
    if gap_ms > st.cursor_gap_max_ms {
        st.cursor_gap_max_ms = gap_ms;
    }
    Some(gap_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_gap_counting() {
        let mut st = SessionStats::default();
        let t0 = Instant::now();
        // обычная частота — не пауза
        assert!(
            note_cursor_gap(Some(t0), t0 + Duration::from_millis(40), 20, true, &mut st).is_none()
        );
        // редкие движения (человек помолчал и сдвинул мышь) — не пауза
        assert!(
            note_cursor_gap(
                Some(t0),
                t0 + Duration::from_millis(1200),
                20,
                true,
                &mut st
            )
            .is_none()
        );
        // непрерывное движение сквозь паузу — засчитывается
        let g = note_cursor_gap(
            Some(t0),
            t0 + Duration::from_millis(1000),
            80,
            true,
            &mut st,
        );
        assert!(g.is_some());
        assert_eq!(st.cursor_gaps, 1);
        assert!(st.cursor_gap_max_ms >= 900.0);
        // прогрев — не считаем
        assert!(
            note_cursor_gap(
                Some(t0),
                t0 + Duration::from_millis(900),
                90,
                false,
                &mut st
            )
            .is_none()
        );
        assert_eq!(st.cursor_gaps, 1);
        // первый пакет — без паузы
        assert!(note_cursor_gap(None, t0, 50, true, &mut st).is_none());
    }
}

fn set_above_normal() {
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
}
