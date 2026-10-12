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
    // Обычно захват — DXGI; если на машине активен защищённый стол (экран
    // входа/UAC/блокировка), DXGI может быть недоступен — тогда начинаем
    // сразу с GDI-захвата защищённого стола.
    let cap0 = Capture::new_with_output(cfg.monitor).ok();
    let mut secure0 = None;
    let (cw, ch, origin0, virt) = match cap0.as_ref() {
        Some(c) => (c.width, c.height, c.origin, c.virt),
        None => {
            let sc = crate::secure::SecureCapture::attach()
                .map_err(|e| format!("захват экрана недоступен: {e}"))?;
            let (w, h) = sc.size();
            secure0 = Some(sc);
            (
                w as usize,
                h as usize,
                (0, 0),
                RectI {
                    x: 0,
                    y: 0,
                    w: w as i32,
                    h: h as i32,
                },
            )
        }
    };
    let out_geom = OutputGeom {
        origin_x: origin0.0,
        origin_y: origin0.1,
        width: cw as i32,
        height: ch as i32,
    };

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
    // Файловые клиенты могут отключить видео (экономия канала).
    let video_off = Arc::new(AtomicBool::new(false));
    let video_off_a = video_off.clone();
    let thread_a = std::thread::spawn(move || {
        set_above_normal();
        let mut cap = cap0;
        let mut secure = secure0;
        let (mut cw, mut ch) = (cw, ch);
        let mut sws_in = sws_in;
        let mut y_buf = vec![0u8; cw * ch];
        let mut uv_buf = vec![0u8; cw * ch / 2];
        let frame_interval = Duration::from_micros(1_000_000 / fps as u64);
        let mut next_t = Instant::now();
        let mut frame_id: u64 = 0;
        let mut consec_err: u32 = 0;
        let mut desk_check_t = Instant::now() - Duration::from_secs(1);

        while !stop_a.load(Ordering::Relaxed) {
            // Контент не нужен (файловый режим) — не тратим захват и кодек.
            if video_off_a.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
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
                    let (nox, noy) = new_cap.origin;
                    cap = Some(new_cap);
                    cw = nw;
                    ch = nh;
                    sws_in = new_sws;
                    y_buf = vec![0u8; cw * ch];
                    uv_buf = vec![0u8; cw * ch / 2];
                    *enc_a.lock().unwrap() = new_enc;
                    inflight_a.lock().unwrap().clear();
                    *geom_a.lock().unwrap() = OutputGeom {
                        origin_x: nox,
                        origin_y: noy,
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

            // Защищённый стол (экран входа, UAC, блокировка): переключаемся на
            // GDI-захват и обратно (проверка не чаще раза в 250 мс).
            if desk_check_t.elapsed() >= Duration::from_millis(250) {
                desk_check_t = Instant::now();
                let secure_now = crate::secure::active_is_secure();
                if secure_now && secure.is_none() {
                    match crate::secure::SecureCapture::attach() {
                        Ok(sc) => {
                            let (nw, nh) = sc.size();
                            let mut ready = true;
                            if nw != cw as u32 || nh != ch as u32 {
                                ready = false;
                                match (
                                    Sws::new(nw, nh, pix_fmt(0), pix_fmt(1)),
                                    open_encoder(&cfg_a, nw, nh),
                                ) {
                                    (Ok(new_sws), Ok((new_enc, codec_used))) => {
                                        sws_in = new_sws;
                                        cw = nw as usize;
                                        ch = nh as usize;
                                        y_buf = vec![0u8; cw * ch];
                                        uv_buf = vec![0u8; cw * ch / 2];
                                        *enc_a.lock().unwrap() = new_enc;
                                        inflight_a.lock().unwrap().clear();
                                        {
                                            let mut st = stats_a.lock().unwrap();
                                            st.codec = codec_used;
                                        }
                                        ready = true;
                                    }
                                    (a, b) => {
                                        let msg =
                                            a.err().or_else(|| b.err()).unwrap_or_else(|| {
                                                "не удалось пересобрать конвейер".to_string()
                                            });
                                        let mut st = stats_a.lock().unwrap();
                                        st.last_error = format!("защищённый стол: {msg}");
                                    }
                                }
                            }
                            if ready {
                                *geom_a.lock().unwrap() = OutputGeom {
                                    origin_x: 0,
                                    origin_y: 0,
                                    width: cw as i32,
                                    height: ch as i32,
                                };
                                dirty_a.store(true, Ordering::Relaxed);
                                secure = Some(sc);
                                stats_a.lock().unwrap().capture_switches += 1;
                            }
                        }
                        Err(e) => {
                            let mut st = stats_a.lock().unwrap();
                            st.last_error = format!("защищённый стол: {e}");
                        }
                    }
                } else if !secure_now && secure.is_some() {
                    secure = None;
                    if let Some(c) = cap.as_mut() {
                        let _ = c.try_recreate();
                    }
                    stats_a.lock().unwrap().capture_switches += 1;
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
            let frame_res = if let Some(sc) = secure.as_mut() {
                sc.capture(|bgra, pitch| unsafe {
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
                })
            } else if let Some(c) = cap.as_mut() {
                c.with_frame(15, |bgra, pitch| {
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
                })
            } else {
                // DXGI-захват не поднялся и защищённый стол не активен — пробуем снова.
                match Capture::new_with_output(cfg_a.monitor) {
                    Ok(nc) => {
                        cap = Some(nc);
                        continue;
                    }
                    Err(e) => Err(e),
                }
            };
            let got = match frame_res {
                Ok(v) => v,
                Err(e) => {
                    if secure.is_none()
                        && e.contains("ACCESS_LOST")
                        && cap.as_mut().is_some_and(|c| c.try_recreate())
                    {
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
    let video_off_b = video_off.clone();
    let thread_b = std::thread::spawn(move || {
        set_above_normal();
        let mut mux = MuxEncoder::new();
        let mut dec = rat_mux::Decoder::new();
        let mut pkt_buf = vec![0u8; 8 * 1024 * 1024];

        // Курсор: отправляем не чаще ~30 Гц и только при изменении.
        let mut last_cursor_t = Instant::now() - Duration::from_secs(1);
        // Первый тик не считаем «паузой»: last_cursor_t намеренно в прошлом,
        // чтобы курсор ушёл сразу.
        let mut cursor_tick_seen = false;
        let mut last_cursor: Option<(u16, u16, bool, u8)> = None;
        let mut last_cursor_img_key: (usize, u8) = (0, 0);

        // Буфер обмена: следим за текстом и пересылаем вьюеру изменения.
        let mut last_clip_t = Instant::now() - Duration::from_secs(1);
        let mut clip_sent: Option<String> = None;
        let mut clip_set_by_us: Option<String> = None;

        // Защищённый стол: поток ввода привязываем к активному входному столу
        // (см. secure.rs: для SendInput нужен доступ DESKTOP_JOURNALPLAYBACK,
        // иначе события молча игнорируются).
        let mut secure_desk: Option<(String, crate::secure::DesktopAttach)> = None;
        let mut desk_check_b = Instant::now() - Duration::from_secs(1);
        // Самопроверка «события применяются»: сверяем последнюю цель движения
        // мыши с реальной позицией курсора (только на защищённом столе).
        let mut last_move_target: Option<(u16, u16, Instant)> = None;
        let mut accept_checked = false;
        // Файловые операции и передачи этой сессии (канал Files).
        let mut fsrv = crate::files::Server::new();

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

            // Переключение потока на защищённый стол (не чаще раза в 250 мс).
            if desk_check_b.elapsed() >= Duration::from_millis(250) {
                desk_check_b = Instant::now();
                match crate::secure::active_desktop_name() {
                    Some(name) if name != "Default" => {
                        let need = !matches!(secure_desk.as_ref(), Some((n, _)) if *n == name);
                        if need {
                            secure_desk = None;
                            accept_checked = false;
                            last_move_target = None;
                            match crate::secure::DesktopAttach::attach_input() {
                                Ok(a) => {
                                    crate::log::log_line(&format!(
                                        "[secure] ввод: поток привязан к столу «{name}»"
                                    ));
                                    secure_desk = Some((name, a));
                                }
                                Err(e) => {
                                    stats_b.lock().unwrap().last_error =
                                        format!("ввод: защищённый стол: {e}");
                                }
                            }
                        }
                    }
                    Some(_) => {
                        secure_desk = None;
                    }
                    None => {}
                }
            }

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
                            if f.payload.first() == Some(&input::META_MAGIC) {
                                if let Some(idx) = input::parse_switch(f.payload) {
                                    *switch_b.lock().unwrap() = Some(idx as u32);
                                } else if let Some(text) = input::parse_clip(f.payload)
                                    && clipboard_text_set(&text)
                                {
                                    clip_set_by_us = Some(text);
                                    stats_b.lock().unwrap().clip_in += 1;
                                } else if input::parse_video_off(f.payload) {
                                    video_off_b.store(true, Ordering::Relaxed);
                                    crate::log::log_line(
                                        "[files] клиент отключил видео (файловый режим)",
                                    );
                                }
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
                            if let Some(InputEvent::Move { x, y }) = evs
                                .iter()
                                .rev()
                                .find(|e| matches!(e, InputEvent::Move { .. }))
                            {
                                last_move_target = Some((*x, *y, Instant::now()));
                                accept_checked = false;
                            }
                            let g = *geom_b.lock().unwrap();
                            let (n, failed, err) = inject_events(&evs, g, virt);
                            stats_b.lock().unwrap().input_recv += n as u64;
                            if failed > 0 {
                                stats_b.lock().unwrap().last_error = format!(
                                    "ввод: SendInput отклонил частей: {failed} (ошибка {err})"
                                );
                                crate::log::log_line(&format!(
                                    "[secure] ввод: SendInput отклонил частей: {failed} (ошибка {err})"
                                ));
                            }
                        } else if f.channel == Channel::Files {
                            // Файлы: список каталогов, скачивание, загрузка.
                            let logs = fsrv.handle(f.payload, &mut |fl, b| {
                                mux.push(Channel::Files, fl, b);
                            });
                            for line in logs {
                                crate::log::log_line(&line);
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

            // 2.5) курсор агента (droppable, «последний выигрывает»).
            let cur_wait = last_cursor_t.elapsed();
            if cur_wait >= Duration::from_millis(33) {
                last_cursor_t = Instant::now();
                if cursor_tick_seen && cur_wait >= Duration::from_millis(250) {
                    // Поток B был заблокирован (например, буфер обмена или
                    // переключение стола) — маркер для разбора «залипаний».
                    crate::log::log_line(&format!(
                        "[net] курсор: поток обработки молчал {:.0} мс",
                        cur_wait.as_secs_f32() * 1000.0
                    ));
                }
                cursor_tick_seen = true;
                let g = *geom_b.lock().unwrap();
                if let Some((cx, cy, vis, kind, hcur)) = cursor_captured_norm(g) {
                    // Самопроверка ввода на защищённом столе: если события
                    // применяются, позиция курсора должна совпасть с целью
                    // последнего движения. Сообщаем один раз за эпизод.
                    if secure_desk.is_some()
                        && !accept_checked
                        && let Some((tx, ty, t_at)) = last_move_target
                        && t_at.elapsed() >= Duration::from_millis(1200)
                    {
                        accept_checked = true;
                        let dx = (cx as i32 - tx as i32).abs();
                        let dy = (cy as i32 - ty as i32).abs();
                        if dx <= 640 && dy <= 640 {
                            crate::log::log_line(
                                "[secure] ввод: самопроверка пройдена — курсор следует за событиями",
                            );
                        } else {
                            crate::log::log_line(&format!(
                                "[secure] ввод: события НЕ применяются: цель ({tx},{ty}), курсор ({cx},{cy})"
                            ));
                            stats_b.lock().unwrap().last_error =
                                "ввод: на защищённом столе события не применяются".to_string();
                        }
                    }
                    if last_cursor != Some((cx, cy, vis, kind)) {
                        let mut cb = [0u8; 6];
                        input::pack_cursor(&mut cb, cx, cy, vis, kind);
                        mux.push(Channel::Cursor, flags::DROPPABLE, &cb);
                        last_cursor = Some((cx, cy, vis, kind));
                        stats_b.lock().unwrap().cursor_sent += 1;
                    }
                    // Настоящая картинка курсора — при смене вида/хэндла; при
                    // ошибке ключ не запоминаем, чтобы повторить на следующем
                    // тике (хэндлы Windows переиспользуются — полагаться только
                    // на их значение нельзя).
                    if hcur != 0 {
                        let key = (hcur, kind);
                        if key != last_cursor_img_key {
                            match cursor_image(hcur) {
                                Ok((w, h, hx, hy, rgba)) => {
                                    let mut img = Vec::with_capacity(11 + rgba.len());
                                    input::pack_cursor_img(&mut img, w, h, hx, hy, kind, &rgba);
                                    mux.push(Channel::Control, 0, &img);
                                    last_cursor_img_key = key;
                                }
                                Err(e) => {
                                    stats_b.lock().unwrap().last_error = format!("курсор: {e}");
                                }
                            }
                        }
                    }
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

            // 2.7) буфер обмена: изменения текста → вьюеру (~2 раза в секунду).
            if last_clip_t.elapsed() >= Duration::from_millis(500) {
                last_clip_t = Instant::now();
                if let Some(text) = clipboard_text_get()
                    && !text.is_empty()
                    && clip_set_by_us.as_deref() != Some(text.as_str())
                    && clip_sent.as_deref() != Some(text.as_str())
                {
                    let mut buf = Vec::with_capacity(text.len() + 2);
                    input::pack_clip(&mut buf, &text);
                    mux.push(Channel::Control, 0, &buf);
                    clip_sent = Some(text);
                    stats_b.lock().unwrap().clip_out += 1;
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

/// Инъекция событий ввода через SendInput.
/// Возвращает (принятых частей, отклонённых частей, код последней ошибки).
fn inject_events(evs: &[InputEvent], out: OutputGeom, virt: RectI) -> (u32, u32, u32) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MOUSEEVENTF_HWHEEL,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
        MOUSEEVENTF_XUP, MOUSEINPUT, MapVirtualKeyW, SendInput, VIRTUAL_KEY,
    };

    let mut ok = 0u32;
    let mut failed = 0u32;
    let mut last_err = 0u32;
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
        } else {
            failed += 1;
            last_err = unsafe { windows::Win32::Foundation::GetLastError().0 };
        }
    }
    (ok, failed, last_err)
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

/// Текущий курсор агента в нормализованных координатах области захвата: (x, y, видим, вид).
fn cursor_captured_norm(out: OutputGeom) -> Option<(u16, u16, bool, u8, usize)> {
    use windows::Win32::UI::WindowsAndMessaging::{CURSOR_SHOWING, CURSORINFO, GetCursorInfo};
    let mut ci = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetCursorInfo(&mut ci) }.ok()?;
    let visible = (ci.flags.0 & CURSOR_SHOWING.0) != 0;
    let (nx, ny) = input::screen_to_captured_norm(ci.ptScreenPos.x, ci.ptScreenPos.y, out);
    Some((
        nx,
        ny,
        visible,
        cursor_kind(ci.hCursor),
        ci.hCursor.0 as usize,
    ))
}

/// Вид курсора по хендлу: сравнение с системными (IDC_*). Кастомный курсор — стрелка.
fn cursor_kind(h: windows::Win32::UI::WindowsAndMessaging::HCURSOR) -> u8 {
    use windows::Win32::UI::WindowsAndMessaging::{
        IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO, IDC_SIZEALL,
        IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IDC_UPARROW, IDC_WAIT, LoadCursorW,
    };

    // Системные курсоры в рамках процесса неизменны — грузим хендлы один раз.
    // Храним адреса (usize): сам HCURSOR не Send/Sync, а в статике это обязательно.
    static STD: std::sync::OnceLock<[(usize, u8); 14]> = std::sync::OnceLock::new();
    let std = STD.get_or_init(|| {
        [
            (IDC_ARROW, input::CURSOR_ARROW),
            (IDC_IBEAM, input::CURSOR_IBEAM),
            (IDC_HAND, input::CURSOR_HAND),
            (IDC_CROSS, input::CURSOR_CROSS),
            (IDC_WAIT, input::CURSOR_WAIT),
            (IDC_APPSTARTING, input::CURSOR_APPSTARTING),
            (IDC_NO, input::CURSOR_NO),
            (IDC_HELP, input::CURSOR_HELP),
            (IDC_SIZENS, input::CURSOR_SIZE_NS),
            (IDC_SIZEWE, input::CURSOR_SIZE_WE),
            (IDC_SIZENWSE, input::CURSOR_SIZE_NWSE),
            (IDC_SIZENESW, input::CURSOR_SIZE_NESW),
            (IDC_SIZEALL, input::CURSOR_SIZE_ALL),
            (IDC_UPARROW, input::CURSOR_UPARROW),
        ]
        .map(|(id, kind)| {
            let h = unsafe { LoadCursorW(None, id) }.unwrap_or_default();
            (h.0 as usize, kind)
        })
    });

    let handle = h.0 as usize;
    for (addr, kind) in std {
        if *addr != 0 && *addr == handle {
            return *kind;
        }
    }
    input::CURSOR_ARROW
}

/// Настоящая картинка курсора: (w, h, hot_x, hot_y, RGBA сверху вниз).
/// Работает и для кастомных курсоров приложений; монохромные собираются из маски.
fn cursor_image(handle: usize) -> Result<(u16, u16, i16, i16, Vec<u8>), String> {
    use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
    use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, HCURSOR, ICONINFO};
    if handle == 0 {
        return Err("пустой хэндл курсора".to_string());
    }
    unsafe {
        let mut ii: ICONINFO = std::mem::zeroed();
        if GetIconInfo(HCURSOR(handle as *mut core::ffi::c_void), &mut ii).is_err() {
            return Err("GetIconInfo: не удалось получить битмапы".to_string());
        }
        let result = build_cursor_image(&ii)
            .ok_or_else(|| "не удалось прочитать битмапы курсора".to_string());
        if !ii.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        }
        if !ii.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        }
        result
    }
}

/// Прочитать HBITMAP как 32bpp BGRA сверху вниз.
unsafe fn read_dib(hbm: windows::Win32::Graphics::Gdi::HBITMAP, w: i32, h: i32) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, ReleaseDC,
    };
    if hbm.is_invalid() || w <= 0 || h <= 0 {
        return None;
    }
    let mut bi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h; // сверху вниз
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // Конверсия битовых форматов (в т.ч. маски 1bpp → 32bpp) требует контекст
    // устройства: с NULL монохромная маска не читается, и весь прямоугольник
    // курсора выходит непрозрачным («чёрный квадрат»).
    let null = HWND(std::ptr::null_mut());
    let hdc = unsafe { GetDC(null) };
    let lines = unsafe {
        GetDIBits(
            hdc,
            hbm,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bi,
            DIB_RGB_COLORS,
        )
    };
    let _ = unsafe { ReleaseDC(null, hdc) };
    if lines == h { Some(buf) } else { None }
}

/// Собрать RGBA-картинку из ICONINFO (цвет + маска; поддержка монохромных).
unsafe fn build_cursor_image(
    ii: &windows::Win32::UI::WindowsAndMessaging::ICONINFO,
) -> Option<(u16, u16, i16, i16, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::{BITMAP, GetObjectW, HGDIOBJ};
    let mask_only = ii.hbmColor.is_invalid();
    let probe = if mask_only { ii.hbmMask } else { ii.hbmColor };
    if probe.is_invalid() {
        return None;
    }
    let mut bm: BITMAP = unsafe { std::mem::zeroed() };
    if unsafe {
        GetObjectW(
            HGDIOBJ(probe.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut core::ffi::c_void),
        )
    } == 0
    {
        return None;
    }
    let (w, h) = if mask_only {
        (bm.bmWidth, bm.bmHeight / 2)
    } else {
        (bm.bmWidth, bm.bmHeight)
    };
    if w <= 0 || h <= 0 || w > 128 || h > 128 {
        return None;
    }
    let color = if mask_only {
        None
    } else {
        unsafe { read_dib(ii.hbmColor, w, h) }
    };
    let mask = if ii.hbmMask.is_invalid() {
        None
    } else {
        unsafe { read_dib(ii.hbmMask, w, if mask_only { h * 2 } else { h }) }
    };
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (r, g, b, ca) = match &color {
            Some(c) => (c[i * 4 + 2], c[i * 4 + 1], c[i * 4], c[i * 4 + 3]),
            None => (0, 0, 0, 0),
        };
        if let Some(m) = &mask {
            if mask_only {
                // Монохромный курсор: маска двойной высоты (AND сверху, XOR снизу).
                // AND=0 — непрозрачная область, цвет берём из XOR (белый/чёрный);
                // AND=1 и XOR=1 — «инверсия фона» (у палки блокнота так задана
                // форма): на типичном светлом фоне выглядит чёрным;
                // иначе — прозрачно.
                let row = i / w as usize;
                let col = i % w as usize;
                let and_v = m[row * w as usize * 4 + col * 4];
                let xor_v = m[(row + h as usize) * w as usize * 4 + col * 4];
                if and_v == 0 {
                    let v = if xor_v != 0 { 255u8 } else { 0 };
                    px.copy_from_slice(&[v, v, v, 255]);
                } else if xor_v != 0 {
                    px.copy_from_slice(&[0, 0, 0, 255]);
                } else {
                    px.copy_from_slice(&[0, 0, 0, 0]);
                }
                continue;
            }
            // Смешанный случай: где у цвета есть альфа — берём её, где нет —
            // маску. У системных курсоров альфа бывает пустой или частичной,
            // и полагаться только на неё нельзя (иначе курсор «невидимка»).
            let a = if ca != 0 {
                ca
            } else if m[i * 4] == 0 {
                255
            } else {
                0
            };
            px.copy_from_slice(&[r, g, b, a]);
        } else {
            let a = if ca != 0 { ca } else { 255 };
            px.copy_from_slice(&[r, g, b, a]);
        }
    }
    Some((
        w as u16,
        h as u16,
        ii.xHotspot.min(i16::MAX as u32) as i16,
        ii.yHotspot.min(i16::MAX as u32) as i16,
        out,
    ))
}

// ================== буфер обмена (текст, Windows) ==================

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 0x0002;

/// Прочитать текст из буфера обмена Windows (с повторами на занятость).
fn clipboard_text_get() -> Option<String> {
    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !opened {
            return None;
        }
        let result = if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
            let handle = GetClipboardData(CF_UNICODETEXT);
            if handle.is_null() {
                None
            } else {
                let ptr = GlobalLock(handle) as *const u16;
                if ptr.is_null() {
                    None
                } else {
                    let mut len = 0usize;
                    while *ptr.add(len) != 0 && len < 65536 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(ptr, len);
                    let text = String::from_utf16_lossy(slice);
                    let _ = GlobalUnlock(handle);
                    Some(text)
                }
            }
        } else {
            None
        };
        let _ = CloseClipboard();
        result
    }
}

/// Записать текст в буфер обмена Windows.
fn clipboard_text_set(text: &str) -> bool {
    unsafe {
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !opened {
            return false;
        }
        let _ = EmptyClipboard();
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if handle.is_null() {
            let _ = CloseClipboard();
            return false;
        }
        let ptr = GlobalLock(handle) as *mut u16;
        if ptr.is_null() {
            let _ = GlobalFree(handle);
            let _ = CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(handle);
        let ok = !SetClipboardData(CF_UNICODETEXT, handle).is_null();
        if !ok {
            let _ = GlobalFree(handle);
        }
        let _ = CloseClipboard();
        ok
    }
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> *mut std::ffi::c_void;
    fn GlobalFree(handle: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn GlobalLock(handle: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn GlobalUnlock(handle: *mut std::ffi::c_void) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn OpenClipboard(hwnd: *mut std::ffi::c_void) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn GetClipboardData(format: u32) -> *mut std::ffi::c_void;
    fn SetClipboardData(format: u32, data: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
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

#[cfg(test)]
mod cursor_image_tests {
    use super::cursor_image;
    use windows::Win32::UI::WindowsAndMessaging::{IDC_ARROW, IDC_IBEAM, LoadCursorW};

    /// Системный курсор-стрелка: картинка читается, альфа настоящая,
    /// хотспот в кончике.
    #[test]
    fn system_arrow_extracts_with_alpha() {
        let hcur = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap();
        let (w, h, hx, hy, rgba) = cursor_image(hcur.0 as usize).expect("картинка стрелки");
        assert!(
            (16..=128).contains(&w) && (16..=128).contains(&h),
            "{w}x{h}"
        );
        assert_eq!(rgba.len(), w as usize * h as usize * 4);
        let alphas: Vec<u8> = rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        assert!(alphas.contains(&0), "нет прозрачных пикселей");
        assert!(alphas.contains(&255), "нет непрозрачных пикселей");
        assert!(
            hx <= 2 && hy <= 2,
            "хотспот стрелки не в кончике: {hx},{hy}"
        );
    }

    /// Диагностический дамп сырых битмапов I-beam (для отладки извлечения).
    #[test]
    fn debug_ibeam_dump() {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Gdi::{
            BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW,
            HGDIOBJ, ReleaseDC,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetIconInfo, ICONINFO, IDC_IBEAM, LoadCursorW,
        };
        let hcur = unsafe { LoadCursorW(None, IDC_IBEAM) }.unwrap();
        unsafe {
            let mut ii: ICONINFO = std::mem::zeroed();
            assert!(GetIconInfo(hcur, &mut ii).is_ok());
            println!(
                "color null={} mask null={} hotspot {},{}, fIcon={}",
                ii.hbmColor.is_invalid(),
                ii.hbmMask.is_invalid(),
                ii.xHotspot,
                ii.yHotspot,
                ii.fIcon.0
            );
            for (name, hbm) in [("color", ii.hbmColor), ("mask", ii.hbmMask)] {
                if hbm.is_invalid() {
                    continue;
                }
                let mut bm: BITMAP = std::mem::zeroed();
                let got = GetObjectW(
                    HGDIOBJ(hbm.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some(&mut bm as *mut BITMAP as *mut _),
                );
                println!(
                    "{name}: GetObject={got} {}x{} bpp={}",
                    bm.bmWidth, bm.bmHeight, bm.bmBitsPixel
                );
                let (w, h) = (bm.bmWidth, bm.bmHeight);
                let mut bi: BITMAPINFO = std::mem::zeroed();
                bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                bi.bmiHeader.biWidth = w;
                bi.bmiHeader.biHeight = -h;
                bi.bmiHeader.biPlanes = 1;
                bi.bmiHeader.biBitCount = 32;
                let mut buf = vec![0u8; (w * h * 4) as usize];
                let null = HWND(std::ptr::null_mut());
                let dc = GetDC(null);
                let lines = GetDIBits(
                    dc,
                    hbm,
                    0,
                    h as u32,
                    Some(buf.as_mut_ptr() as *mut _),
                    &mut bi,
                    DIB_RGB_COLORS,
                );
                let _ = ReleaseDC(null, dc);
                let ch = buf.as_chunks::<4>().0;
                let a0 = ch.iter().filter(|p| p[3] == 0).count();
                let am = ch.iter().filter(|p| p[3] > 0 && p[3] < 128).count();
                let ah = ch.iter().filter(|p| p[3] >= 128).count();
                let blk = ch
                    .iter()
                    .filter(|p| p[0] == 0 && p[1] == 0 && p[2] == 0)
                    .count();
                let wht = ch
                    .iter()
                    .filter(|p| p[0] == 255 && p[1] == 255 && p[2] == 255)
                    .count();
                println!(
                    "{name}: lines={lines}; alpha0={a0} mid={am} hi={ah}; black={blk} white={wht}"
                );
                if name == "mask" {
                    for row in 0..h {
                        let mut line = String::new();
                        for col in 0..w {
                            let p = ((row * w + col) * 4) as usize;
                            line.push(if buf[p] == 0 { '#' } else { '.' });
                        }
                        println!("{row:02} {line}");
                    }
                }
            }
        }
    }

    /// Все распространённые системные курсоры извлекаются с видимыми пикселями
    /// (ловит и цветные, и монохромные/инверсные форматы — как «палка» блокнота).
    #[test]
    fn system_cursors_all_visible() {
        use windows::Win32::UI::WindowsAndMessaging::{
            IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO, IDC_SIZEALL, IDC_SIZENS,
            IDC_WAIT,
        };
        for (name, id) in [
            ("arrow", IDC_ARROW),
            ("ibeam", IDC_IBEAM),
            ("cross", IDC_CROSS),
            ("hand", IDC_HAND),
            ("wait", IDC_WAIT),
            ("no", IDC_NO),
            ("sizeall", IDC_SIZEALL),
            ("sizens", IDC_SIZENS),
            ("help", IDC_HELP),
        ] {
            let hcur = unsafe { LoadCursorW(None, id) }.unwrap();
            let (w, h, _, _, rgba) = cursor_image(hcur.0 as usize).expect(name);
            let visible = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] >= 128)
                .count();
            println!("{name}: {w}x{h}, видимых пикселей {visible}");
            assert!(visible >= 4, "{name}: нет видимых пикселей ({visible})");
        }
    }

    /// Текстовый курсор: картинка читается, хотспот внутри её области.
    #[test]
    fn system_ibeam_extracts() {
        let hcur = unsafe { LoadCursorW(None, IDC_IBEAM) }.unwrap();
        let (w, h, hx, hy, _) = cursor_image(hcur.0 as usize).expect("картинка I-beam");
        println!("IBEAM {}x{} hotspot {hx},{hy}", w, h);
        assert!(hx >= 0 && hx < w as i16, "hotspot x вне картинки: {hx}");
        assert!(hy >= 0 && hy < h as i16, "hotspot y вне картинки: {hy}");
    }
}
