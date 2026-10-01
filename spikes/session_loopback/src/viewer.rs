//! Вьюерская сторона loopback-сессии: приём mux-кадров → H.264 декод → кадры в UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use prost::Message;
use rat_mux::{Channel, Encoder as MuxEncoder};
use rat_proto::rat::v0::{envelope::Body, Envelope, Ping};

use crate::ffi::Decoder;
use crate::link::Duplex;
use crate::stats::{now_us, SessionStats, TimeBase};

pub struct FrameOut {
    pub id: u64,
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
    pub t_capture_us: u64,
}

fn encode_env(env: &Envelope) -> Vec<u8> {
    let mut b = Vec::with_capacity(64);
    env.encode(&mut b).expect("encode envelope");
    b
}

fn send_ping(out: &mut MuxEncoder, id: u64, pending: &mut HashMap<u64, Instant>) {
    let env = Envelope {
        ver: 1,
        request_id: id,
        body: Some(Body::Ping(Ping { nonce: id })),
    };
    out.push(Channel::Control, 0, &encode_env(&env));
    pending.insert(id, Instant::now());
}

pub fn spawn_viewer(
    base: TimeBase,
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<SessionStats>>,
    link: Duplex,
    ui_tx: SyncSender<FrameOut>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        unsafe {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
            };
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        }

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
        let mut sws_out: Option<crate::ffi::Sws> = None;
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
                            if let Ok(env) = Envelope::decode(f.payload) {
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
                            if f.payload.len() < 16 {
                                continue;
                            }
                            let id =
                                u64::from_be_bytes(f.payload[0..8].try_into().unwrap());
                            let ts_us =
                                u64::from_be_bytes(f.payload[8..16].try_into().unwrap());
                            let data = &f.payload[16..];

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
                                if dec.pix_fmt() != crate::ffi::pix_fmt(2) {
                                    let mut st = stats.lock().unwrap();
                                    st.last_error =
                                        format!("unexpected pix_fmt {}", dec.pix_fmt());
                                    st.viewer_errors += 1;
                                    break;
                                }
                                if let Some(pv) = dec.planes() {
                                    if sws_out.is_none() {
                                        match crate::ffi::Sws::new(
                                            pv.w as u32,
                                            pv.h as u32,
                                            crate::ffi::pix_fmt(2),
                                            crate::ffi::pix_fmt(3),
                                        ) {
                                            Ok(s) => sws_out = Some(s),
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
                                        crate::ffi::sws_yuv_to_rgba(
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
                                    let e2e = ((now_us(&base) as i64 - ts_us as i64).max(0))
                                        as f32
                                        / 1000.0;
                                    {
                                        let mut st = stats.lock().unwrap();
                                        SessionStats::push_sample(&mut st.e2e_ms, e2e);
                                        st.shown += 1;
                                    }
                                    fps_count += 1;

                                    let out_rgba = std::mem::replace(
                                        &mut rgba,
                                        vec![0u8; pv.w * pv.h * 4],
                                    );
                                    let frame = FrameOut {
                                        id,
                                        w: pv.w,
                                        h: pv.h,
                                        rgba: out_rgba,
                                        t_capture_us: ts_us,
                                    };
                                    if ui_tx.try_send(frame).is_err() {
                                        stats.lock().unwrap().dropped += 1;
                                    }
                                }
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
