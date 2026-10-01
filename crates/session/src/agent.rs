//! Агентская сторона сессии: захват экрана → H.264 → mux-кадры.
//!
//! Поток A: пейсинг + захват (DXGI) + BGRA→NV12 (swscale) + отправка в энкодер.
//! Поток B: пакеты энкодера → mux-кадры (Video) → линк; Control: Ping→Pong.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rat_mux::{Channel, Encoder as MuxEncoder, flags};
use rat_proto::rat::v0::envelope::Body;

use crate::capture::Capture;
use crate::control;
use crate::ffmpeg::{Encoder, Sws, pix_fmt, sws_bgra_to_nv12};
use crate::frame;
use crate::link::Duplex;
use crate::stats::{SessionStats, TimeBase, now_us};

/// Возможности агента для Hello.
pub const AGENT_CAPABILITIES: &[&str] = &["video.h264", "mux.v1"];

#[derive(Clone)]
pub struct AgentConfig {
    pub fps: u32,
    pub bitrate: i64,
    pub gop: i32,
    pub codec: String,
    pub preset: String,
    pub async_depth: i32,
    pub low_delay: i32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            fps: 30,
            bitrate: 8_000_000,
            gop: 60,
            codec: "h264_qsv".to_string(),
            preset: "veryfast".to_string(),
            async_depth: 1,
            low_delay: 1,
        }
    }
}

/// Запустить агентскую половину сессии. Возвращает хендлы потоков A и B.
pub fn spawn_agent(
    base: TimeBase,
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<SessionStats>>,
    link: Duplex,
    cfg: AgentConfig,
) -> Result<Vec<std::thread::JoinHandle<()>>, String> {
    let cap = Capture::new()?;
    let (cw, ch) = (cap.width, cap.height);

    let open_enc = |codec: &str, preset: &str, async_depth: i32, low_delay: i32| {
        Encoder::new(
            codec,
            cw as u32,
            ch as u32,
            cfg.fps,
            cfg.bitrate,
            cfg.gop,
            preset,
            async_depth,
            low_delay,
        )
    };
    let (enc, codec_used) = match open_enc(&cfg.codec, &cfg.preset, cfg.async_depth, cfg.low_delay)
    {
        Ok(e) => (e, cfg.codec.clone()),
        Err(e1) => {
            println!("[agent] {e1} — fallback на libx264");
            let e = open_enc("libx264", "veryfast", 0, 0)?;
            (e, "libx264".to_string())
        }
    };
    stats.lock().unwrap().codec = codec_used;

    let sws_in = Sws::new(cw as u32, ch as u32, pix_fmt(0), pix_fmt(1))?;
    let enc_shared = Arc::new(Mutex::new(enc));
    // (frame_id, ts_us_capture, t_sent, cap_ms)
    let inflight: Arc<Mutex<VecDeque<(u64, u64, Instant, f32)>>> =
        Arc::new(Mutex::new(VecDeque::new()));

    // --- Поток A: пейсинг + захват + отправка в энкодер ---
    let stop_a = stop.clone();
    let stats_a = stats.clone();
    let enc_a = enc_shared.clone();
    let inflight_a = inflight.clone();
    let fps = cfg.fps.max(1);
    let thread_a = std::thread::spawn(move || {
        set_above_normal();
        let mut cap = cap;
        let mut y_buf = vec![0u8; cw * ch];
        let mut uv_buf = vec![0u8; cw * ch / 2];
        let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
        let mut next_t = Instant::now();
        let mut frame_id: u64 = 0;
        let mut consec_err: u32 = 0;

        while !stop_a.load(Ordering::Relaxed) {
            let now = Instant::now();
            if next_t > now {
                std::thread::sleep(next_t - now);
            }
            if next_t < Instant::now() {
                next_t = Instant::now() + frame_interval;
            } else {
                next_t += frame_interval;
            }

            let t0 = Instant::now();
            let sws_ptr = sws_in.ptr();
            let got = match cap.with_frame(15, |bgra, pitch| {
                unsafe {
                    sws_bgra_to_nv12(
                        sws_ptr,
                        bgra.as_ptr(),
                        pitch,
                        y_buf.as_mut_ptr(),
                        cw,
                        uv_buf.as_mut_ptr(),
                        cw,
                        ch,
                    );
                }
                Ok(())
            }) {
                Ok(v) => v,
                Err(e) => {
                    if e.contains("ACCESS_LOST") && cap.try_recreate() {
                        continue;
                    }
                    consec_err += 1;
                    {
                        let mut st = stats_a.lock().unwrap();
                        st.agent_errors += 1;
                        st.last_error = e;
                    }
                    if consec_err > 120 {
                        break;
                    }
                    continue;
                }
            };
            if !got {
                continue;
            }
            consec_err = 0;
            let cap_ms = (Instant::now() - t0).as_secs_f32() * 1000.0;
            let ts_us = now_us(&base);

            let t_sent = Instant::now();
            let r = {
                let mut e = enc_a.lock().unwrap();
                e.send(&y_buf, cw, &uv_buf, cw, frame_id as i64)
            };
            match r {
                Ok(0) => {
                    inflight_a
                        .lock()
                        .unwrap()
                        .push_back((frame_id, ts_us, t_sent, cap_ms));
                    frame_id += 1;
                    stats_a.lock().unwrap().captured += 1;
                }
                Ok(_) => {
                    stats_a.lock().unwrap().agent_dropped += 1;
                }
                Err(e) => {
                    let mut st = stats_a.lock().unwrap();
                    st.agent_errors += 1;
                    st.last_error = e;
                }
            }
        }
        stop_a.store(true, Ordering::Relaxed);
    });

    // --- Поток B: пакеты энкодера → mux → линк; Ping→Pong ---
    let stop_b = stop.clone();
    let stats_b = stats.clone();
    let enc_b = enc_shared.clone();
    let inflight_b = inflight.clone();
    let thread_b = std::thread::spawn(move || {
        set_above_normal();
        let mut mux = MuxEncoder::new();
        let mut dec = rat_mux::Decoder::new();
        let mut pkt_buf = vec![0u8; 8 * 1024 * 1024];

        // Hello (capabilities) сразу при старте.
        let hello = control::hello(
            concat!("rat-agent-", env!("CARGO_PKG_VERSION")),
            AGENT_CAPABILITIES,
        );
        mux.push(Channel::Control, 0, &control::encode_env(&hello));
        if !flush(&mut mux, &link) {
            stop_b.store(true, Ordering::Relaxed);
            return;
        }

        while !stop_b.load(Ordering::Relaxed) {
            let mut did = false;

            // 1) входящие кадры линка (Control от вьюера).
            loop {
                match link.rx.try_recv() {
                    Ok(bytes) => {
                        dec.feed(&bytes);
                        did = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        // Вьюер отключился — завершаем сессию.
                        stop_b.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            }
            let mut reset_dec = false;
            loop {
                match dec.poll_frame() {
                    Ok(Some(f)) => {
                        did = true;
                        if f.channel == Channel::Control {
                            if let Some(env) = control::decode_env(f.payload) {
                                stats_b.lock().unwrap().control_msgs += 1;
                                if let Some(Body::Ping(p)) = env.body {
                                    let pong = control::pong(env.request_id, p.nonce);
                                    mux.push(Channel::Control, 0, &control::encode_env(&pong));
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => {
                        reset_dec = true;
                        break;
                    }
                }
            }
            if reset_dec {
                dec.reset();
            }

            // 2) пакеты энкодера → видеокадры mux.
            let recv_res = {
                let mut e = enc_b.lock().unwrap();
                e.recv(&mut pkt_buf)
            };
            match recv_res {
                Ok(Some(sz)) => {
                    did = true;
                    let (id, ts_us, _t_sent, _cap_ms) = inflight_b
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or((u64::MAX, 0, Instant::now(), 0.0));
                    let mut payload = Vec::with_capacity(frame::META_LEN + sz);
                    frame::pack(&mut payload, id, ts_us, &pkt_buf[..sz]);
                    let fl = flags::DROPPABLE
                        | if frame::is_idr(&pkt_buf[..sz]) {
                            flags::KEYFRAME
                        } else {
                            0
                        };
                    mux.push(Channel::Video, fl, &payload);
                    {
                        let mut st = stats_b.lock().unwrap();
                        st.sent_pkts += 1;
                        st.sent_bytes += (payload.len() + rat_mux::HEADER_LEN) as u64;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let mut st = stats_b.lock().unwrap();
                    st.agent_errors += 1;
                    st.last_error = e;
                }
            }

            // 3) выгрузить накопленные mux-кадры в линк.
            if !mux.is_empty() {
                did = true;
                if !flush(&mut mux, &link) {
                    stop_b.store(true, Ordering::Relaxed);
                    return;
                }
            }

            if !did {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    });

    Ok(vec![thread_a, thread_b])
}

fn flush(mux: &mut MuxEncoder, link: &Duplex) -> bool {
    if mux.is_empty() {
        return true;
    }
    link.tx.send(mux.take_bytes()).is_ok()
}

fn set_above_normal() {
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
}
