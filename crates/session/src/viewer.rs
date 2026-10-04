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

/// Общее состояние вьюерской стороны: очередь ввода из UI, курсор и геометрия от агента.
pub struct ViewerShared {
    pub input_q: Arc<Mutex<Vec<InputEvent>>>,
    pub cursor: Arc<Mutex<CursorState>>,
    pub info: Arc<Mutex<Option<SessionInfo>>>,
    /// Список мониторов агента: (индекс, ширина, высота).
    pub monitors: Arc<Mutex<Vec<(u16, u16, u16)>>>,
    /// Запрос на переключение монитора (последний выигрывает).
    pub pending_switch: Arc<Mutex<Option<u16>>>,
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
            // Отправить накопленный ввод (с коалессингом движений мыши).
            let batch: Vec<InputEvent> = std::mem::take(&mut *shared.input_q.lock().unwrap());
            if !batch.is_empty() {
                let coalesced = input::coalesce_moves(batch);
                let mut buf = Vec::with_capacity(2 + coalesced.len() * 9);
                input::pack_batch(&mut buf, &coalesced);
                out.push(Channel::Input, 0, &buf);
                stats.lock().unwrap().input_sent += coalesced.len() as u64;
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

fn set_above_normal() {
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
}
