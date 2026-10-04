//! Агентская сторона сессии: захват экрана → H.264 → mux-кадры.
//!
//! Поток A: пейсинг + захват (DXGI) + BGRA→NV12 (swscale) + отправка в энкодер.
//! Поток B: пакеты энкодера → mux-кадры (Video) → линк; Control: Ping→Pong.

use std::collections::VecDeque;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rat_mux::{Channel, Encoder as MuxEncoder, flags};
use rat_proto::rat::v0::envelope::Body;

use crate::capture::Capture;
use crate::control;
use crate::ffmpeg::{Encoder, Sws, pix_fmt, sws_bgra_to_nv12};
use crate::frame;
use crate::input::{self, InputEvent, OutputGeom, RectI};
use crate::link::Duplex;
use crate::stats::{SessionStats, TimeBase, now_us};

/// Возможности агента для Hello.
pub const AGENT_CAPABILITIES: &[&str] = &[
    "video.h264",
    "mux.v1",
    "input.v0",
    "cursor.v0",
    "monitors.v0",
];

/// Кадры «в полёте»: (frame_id, ts_us_capture, t_sent, cap_ms).
type Inflight = VecDeque<(u64, u64, Instant, f32)>;

#[derive(Clone)]
pub struct AgentConfig {
    pub fps: u32,
    pub bitrate: i64,
    pub gop: i32,
    pub codec: String,
    pub preset: String,
    pub async_depth: i32,
    pub low_delay: i32,
    /// Индекс вывода (монитора) для захвата.
    pub monitor: u32,
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
            monitor: 0,
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
    let cap = Capture::new_with_output(cfg.monitor)?;
    let (cw, ch) = (cap.width, cap.height);
    let out_geom = OutputGeom {
        origin_x: cap.origin.0,
        origin_y: cap.origin.1,
        width: cw as i32,
        height: ch as i32,
    };
    let virt = cap.virt;

    // Общее состояние для переключения монитора «на лету».
    let sh_geom: Arc<Mutex<OutputGeom>> = Arc::new(Mutex::new(out_geom));
    let sh_active = Arc::new(AtomicU32::new(cfg.monitor));
    let switch_req: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
    let geom_dirty = Arc::new(AtomicBool::new(false));

    let (enc, codec_used) = open_encoder(&cfg, cw as u32, ch as u32)?;
    stats.lock().unwrap().codec = codec_used;

    let sws_in = Sws::new(cw as u32, ch as u32, pix_fmt(0), pix_fmt(1))?;
    let enc_shared = Arc::new(Mutex::new(enc));
    let inflight: Arc<Mutex<Inflight>> = Arc::new(Mutex::new(VecDeque::new()));

    // --- Поток A: пейсинг + захват + отправка в энкодер ---
    let stop_a = stop.clone();
    let stats_a = stats.clone();
    let enc_a = enc_shared.clone();
    let inflight_a = inflight.clone();
    let cfg_a = cfg.clone();
    let geom_a = sh_geom.clone();
    let active_a = sh_active.clone();
    let switch_a = switch_req.clone();
    let dirty_a = geom_dirty.clone();
    let fps = cfg.fps.max(1);
    let thread_a = std::thread::spawn(move || {
        set_above_normal();
        let mut cap = cap;
        let (mut cw, mut ch) = (cw, ch);
        let mut sws_in = sws_in;
        let mut y_buf = vec![0u8; cw * ch];
        let mut uv_buf = vec![0u8; cw * ch / 2];
        let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
        let mut next_t = Instant::now();
        let mut frame_id: u64 = 0;
        let mut consec_err: u32 = 0;

        while !stop_a.load(Ordering::Relaxed) {
            // Переключение монитора по запросу вьюера: пересоздаём весь конвейер.
            if let Some(idx) = switch_a.lock().unwrap().take() {
                let res: Result<(), String> = 'sw: {
                    let new_cap = match Capture::new_with_output(idx) {
                        Ok(c) => c,
                        Err(e) => break 'sw Err(e),
                    };
                    let (nw, nh) = (new_cap.width, new_cap.height);
                    let new_sws = match Sws::new(nw as u32, nh as u32, pix_fmt(0), pix_fmt(1)) {
                        Ok(s) => s,
                        Err(e) => break 'sw Err(e),
                    };
                    let (new_enc, codec_used) = match open_encoder(&cfg_a, nw as u32, nh as u32) {
                        Ok(v) => v,
                        Err(e) => break 'sw Err(e),
                    };
                    cap = new_cap;
                    cw = nw;
                    ch = nh;
                    sws_in = new_sws;
                    y_buf = vec![0u8; cw * ch];
                    uv_buf = vec![0u8; cw * ch / 2];
                    *enc_a.lock().unwrap() = new_enc;
                    inflight_a.lock().unwrap().clear();
                    *geom_a.lock().unwrap() = OutputGeom {
                        origin_x: cap.origin.0,
                        origin_y: cap.origin.1,
                        width: cw as i32,
                        height: ch as i32,
                    };
                    active_a.store(idx, Ordering::Relaxed);
                    dirty_a.store(true, Ordering::Relaxed);
                    {
                        let mut st = stats_a.lock().unwrap();
                        st.capture_switches += 1;
                        st.codec = codec_used;
                    }
                    Ok(())
                };
                if let Err(e) = res {
                    let mut st = stats_a.lock().unwrap();
                    st.agent_errors += 1;
                    st.last_error = format!("переключение монитора {idx}: {e}");
                }
            }

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
    let geom_b = sh_geom.clone();
    let active_b = sh_active.clone();
    let switch_b = switch_req.clone();
    let dirty_b = geom_dirty.clone();
    let thread_b = std::thread::spawn(move || {
        set_above_normal();
        let mut mux = MuxEncoder::new();
        let mut dec = rat_mux::Decoder::new();
        let mut pkt_buf = vec![0u8; 8 * 1024 * 1024];

        // Курсор: отправляем не чаще ~30 Гц и только при изменении.
        let mut last_cursor_t = Instant::now() - Duration::from_secs(1);
        let mut last_cursor: Option<(u16, u16, bool)> = None;

        // Hello (capabilities) сразу при старте.
        let hello = control::hello(
            concat!("rat-agent-", env!("CARGO_PKG_VERSION")),
            AGENT_CAPABILITIES,
        );
        mux.push(Channel::Control, 0, &control::encode_env(&hello));
        let mut meta = Vec::with_capacity(24);
        {
            let g = *geom_b.lock().unwrap();
            input::pack_session_info(
                &mut meta,
                input::SessionInfo {
                    out: g,
                    virt,
                    active: active_b.load(Ordering::Relaxed) as u16,
                },
            );
        }
        mux.push(Channel::Control, 0, &meta);
        // Список мониторов — для кнопок переключения у вьюера.
        if let Ok(list) = crate::capture::list_outputs() {
            let tuples: Vec<(u16, u16, u16)> = list
                .iter()
                .map(|m| (m.index as u16, m.width as u16, m.height as u16))
                .collect();
            let mut mb = Vec::with_capacity(3 + tuples.len() * 6);
            input::pack_monitors(&mut mb, &tuples);
            mux.push(Channel::Control, 0, &mb);
        }
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
                            if f.payload.first() == Some(&input::META_MAGIC)
                                && let Some(idx) = input::parse_switch(f.payload)
                            {
                                *switch_b.lock().unwrap() = Some(idx as u32);
                            } else if let Some(env) = control::decode_env(f.payload) {
                                stats_b.lock().unwrap().control_msgs += 1;
                                if let Some(Body::Ping(p)) = env.body {
                                    let pong = control::pong(env.request_id, p.nonce);
                                    mux.push(Channel::Control, 0, &control::encode_env(&pong));
                                }
                            }
                        } else if f.channel == Channel::Input
                            && let Some(evs) = input::parse_batch(f.payload)
                        {
                            let g = *geom_b.lock().unwrap();
                            let n = inject_events(&evs, g, virt);
                            stats_b.lock().unwrap().input_recv += n as u64;
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

            // 2.5) курсор агента (droppable, «последний выигрывает»).
            if last_cursor_t.elapsed() >= Duration::from_millis(33) {
                last_cursor_t = Instant::now();
                let g = *geom_b.lock().unwrap();
                if let Some((cx, cy, vis)) = cursor_captured_norm(g)
                    && last_cursor != Some((cx, cy, vis))
                {
                    let mut cb = [0u8; 5];
                    input::pack_cursor(&mut cb, cx, cy, vis);
                    mux.push(Channel::Cursor, flags::DROPPABLE, &cb);
                    last_cursor = Some((cx, cy, vis));
                    stats_b.lock().unwrap().cursor_sent += 1;
                }
            }

            // 2.6) после переключения монитора — обновлённый SessionInfo вьюеру.
            if dirty_b.swap(false, Ordering::Relaxed) {
                let mut meta = Vec::with_capacity(24);
                let g = *geom_b.lock().unwrap();
                input::pack_session_info(
                    &mut meta,
                    input::SessionInfo {
                        out: g,
                        virt,
                        active: active_b.load(Ordering::Relaxed) as u16,
                    },
                );
                mux.push(Channel::Control, 0, &meta);
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

/// Открыть энкодер под размер кадра (с фолбэком на libx264).
fn open_encoder(cfg: &AgentConfig, w: u32, h: u32) -> Result<(Encoder, String), String> {
    match Encoder::new(
        &cfg.codec,
        w,
        h,
        cfg.fps,
        cfg.bitrate,
        cfg.gop,
        &cfg.preset,
        cfg.async_depth,
        cfg.low_delay,
    ) {
        Ok(e) => Ok((e, cfg.codec.clone())),
        Err(e1) => {
            let _ = writeln!(std::io::stdout(), "[agent] {e1} — fallback на libx264");
            Encoder::new(
                "libx264",
                w,
                h,
                cfg.fps,
                cfg.bitrate,
                cfg.gop,
                "veryfast",
                0,
                0,
            )
            .map(|e| (e, "libx264".to_string()))
        }
    }
}

/// Инъекция событий ввода через SendInput. Возвращает число обработанных событий.
fn inject_events(evs: &[InputEvent], out: OutputGeom, virt: RectI) -> u32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MOUSEEVENTF_HWHEEL,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
        MOUSEEVENTF_XUP, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY,
    };

    let mut ok = 0u32;
    let mut buf: Vec<INPUT> = Vec::with_capacity(4);
    for ev in evs {
        buf.clear();
        match *ev {
            InputEvent::Move { x, y } => {
                let (vx, vy) = input::captured_norm_to_virtual_norm(x, y, out, virt);
                buf.push(mouse_move_input(vx, vy));
            }
            InputEvent::Button { btn, down, x, y } => {
                let (vx, vy) = input::captured_norm_to_virtual_norm(x, y, out, virt);
                buf.push(mouse_move_input(vx, vy));
                let (flag, data) = match (btn, down) {
                    (0, true) => (MOUSEEVENTF_LEFTDOWN, 0u32),
                    (0, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (1, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (1, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (2, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (2, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (3, true) => (MOUSEEVENTF_XDOWN, 1),
                    (3, false) => (MOUSEEVENTF_XUP, 1),
                    (4, true) => (MOUSEEVENTF_XDOWN, 2),
                    (4, false) => (MOUSEEVENTF_XUP, 2),
                    _ => continue,
                };
                buf.push(INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: data,
                            dwFlags: flag,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }
            InputEvent::Wheel { dx, dy, x, y } => {
                let (vx, vy) = input::captured_norm_to_virtual_norm(x, y, out, virt);
                buf.push(mouse_move_input(vx, vy));
                if dy != 0 {
                    buf.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: dy as i32 as u32,
                                dwFlags: MOUSEEVENTF_WHEEL,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
                if dx != 0 {
                    buf.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dx: 0,
                                dy: 0,
                                mouseData: dx as i32 as u32,
                                dwFlags: MOUSEEVENTF_HWHEEL,
                                time: 0,
                                dwExtraInfo: 0,
                            },
                        },
                    });
                }
            }
            InputEvent::Key { vk, down } => {
                let base = if is_extended_key(vk) {
                    KEYEVENTF_EXTENDEDKEY
                } else {
                    KEYBD_EVENT_FLAGS(0)
                };
                let flags = if down { base } else { base | KEYEVENTF_KEYUP };
                buf.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(vk),
                            wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                });
            }
        }
        if buf.is_empty() {
            continue;
        }
        let sent = unsafe { SendInput(&buf, std::mem::size_of::<INPUT>() as i32) };
        if sent == buf.len() as u32 {
            ok += 1;
        }
    }
    ok
}

/// Абсолютный перенос курсора (виртуальный рабочий стол, 0..=65535).
fn mouse_move_input(vx: u16, vy: u16) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: vx as i32,
                dy: vy as i32,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// VK-коды, требующие флага extended (стрелки/Ins/Del/Home/End/PgUp/PgDn и др.).
fn is_extended_key(vk: u16) -> bool {
    matches!(vk, 0x21..=0x28 | 0x2D | 0x2E | 0x5B..=0x5D | 0x6F)
}

/// Текущий курсор агента в нормализованных координатах области захвата.
fn cursor_captured_norm(out: OutputGeom) -> Option<(u16, u16, bool)> {
    use windows::Win32::UI::WindowsAndMessaging::{CURSOR_SHOWING, CURSORINFO, GetCursorInfo};
    let mut ci = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetCursorInfo(&mut ci) }.ok()?;
    let visible = (ci.flags.0 & CURSOR_SHOWING.0) != 0;
    let (nx, ny) = input::screen_to_captured_norm(ci.ptScreenPos.x, ci.ptScreenPos.y, out);
    Some((nx, ny, visible))
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
