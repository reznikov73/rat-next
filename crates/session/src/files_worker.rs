//! Файловый воркер: отдельный поток, работающий с агентом по каналу «Файлы».
//!
//! Используется оконным менеджером файлов (bins/rat): UI шлёт команды [`WCmd`]
//! и получает события [`WEvt`]. Соединение поднимает сам воркер; сразу после
//! подключения видео отключается (файловый режим без экрана).

use crate::files::{
    self, CHUNK, DirEntry, WINDOW, pack_credit, pack_data, pack_id_path, pack_read_open, pack_tid,
    pack_write_close, pack_write_open, parse_credit, parse_data, parse_drives_resp,
    parse_list_resp, parse_ok_resp, parse_read_open_resp, parse_serv_error, parse_write_close_resp,
    parse_write_open_resp, st,
};
use crate::input;
use crate::link;
use rat_mux::{Channel, Decoder, Encoder};
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Куда подключаться.
pub enum Target {
    Tcp(String),
    Relay { relay: String, id: String },
}

/// Решение по конфликту имён.
#[derive(Clone)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    Rename(String),
}

/// Команды UI → воркер.
pub enum WCmd {
    List {
        path: String,
    },
    Drives,
    Mkdir {
        path: String,
    },
    Rename {
        old: String,
        new: String,
    },
    Delete {
        path: String,
    },
    Download {
        id: u64,
        remote: String,
        local: String,
    },
    Upload {
        id: u64,
        local: String,
        remote: String,
    },
    Resolve {
        id: u64,
        action: ConflictAction,
    },
    Cancel {
        id: u64,
    },
    Quit,
}

/// События воркер → UI.
pub enum WEvt {
    Connected,
    ConnectFailed {
        err: String,
    },
    List {
        path: String,
        result: Result<Vec<DirEntry>, String>,
    },
    Drives {
        drives: Vec<String>,
    },
    Op {
        action: &'static str,
        path: String,
        result: Result<(), String>,
    },
    JobStarted {
        id: u64,
    },
    Progress {
        id: u64,
        done: u64,
        total: u64,
    },
    JobDone {
        id: u64,
        result: Result<(), String>,
    },
    Conflict {
        id: u64,
        path: String,
    },
    Disconnected {
        err: String,
    },
}

const REQ: u32 = 11;
const TID: u32 = 21;

/// Запустить воркер: вернуть канал команд и канал событий.
pub fn spawn(target: Target) -> (Sender<WCmd>, Receiver<WEvt>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<WCmd>();
    let (evt_tx, evt_rx) = std::sync::mpsc::channel::<WEvt>();
    let _ = std::thread::Builder::new()
        .name("rat-files".into())
        .spawn(move || run(target, cmd_rx, evt_tx));
    (cmd_tx, evt_rx)
}

struct Ctx<'a> {
    link: &'a link::Duplex,
    enc: &'a mut Encoder,
    evt: &'a Sender<WEvt>,
}

impl Ctx<'_> {
    fn ev(&self, e: WEvt) {
        let _ = self.evt.send(e);
    }
    fn send(&mut self, payload: &[u8]) -> bool {
        self.enc.push(Channel::Files, 0, payload);
        self.link.tx.send(self.enc.take_bytes()).is_ok()
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Stage {
    DlWait,
    DlOpen,
    DlData,
    UlWait,
    UlOpen,
    UlData,
    UlClose,
}

struct Xfer {
    id: u64,
    up: bool,
    local: String,
    remote: String,
    file: Option<std::fs::File>,
    total: u64,
    pos: u64,
    credits: u32,
    mtime: i64,
    stage: Stage,
    last_progress: Instant,
    last_emit: Instant,
    del_local_on_fail: bool,
}

struct CurOp {
    action: &'static str,
    path: String,
    deadline: Instant,
}

struct State {
    queue: VecDeque<WCmd>,
    cur: Option<CurOp>,
    xfer: Option<Xfer>,
}

fn run(target: Target, cmd_rx: Receiver<WCmd>, evt: Sender<WEvt>) {
    // 1) Подключение (до 10 попыток — как у сеанса экрана).
    let mut conn: Option<(link::Duplex, link::TcpCloser)> = None;
    let mut last_err = String::new();
    for attempt in 0..10 {
        let r = match &target {
            Target::Tcp(addr) => link::tcp_connect(addr.as_str()),
            Target::Relay { relay, id } => {
                if relay.trim().is_empty() {
                    Err(std::io::Error::other("не задан интернет-сервер"))
                } else {
                    link::relay_connect(relay.as_str(), id)
                }
            }
        };
        match r {
            Ok(v) => {
                conn = Some(v);
                break;
            }
            Err(e) => {
                last_err = e.to_string();
                if attempt < 9 {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        }
    }
    let Some((link, closer)) = conn else {
        let _ = evt.send(WEvt::ConnectFailed { err: last_err });
        loop {
            match cmd_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(WCmd::Quit) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    };

    let mut dec = Decoder::new();
    let mut enc = Encoder::new();
    // Файловый режим: видео не нужно.
    enc.push(Channel::Control, 0, &input::video_off_payload());
    if link.tx.send(enc.take_bytes()).is_err() {
        let _ = evt.send(WEvt::ConnectFailed {
            err: "линк закрыт".to_string(),
        });
        closer.close();
        return;
    }
    let _ = evt.send(WEvt::Connected);

    let mut st = State {
        queue: VecDeque::new(),
        cur: None,
        xfer: None,
    };
    let mut ctx = Ctx {
        link: &link,
        enc: &mut enc,
        evt: &evt,
    };

    loop {
        // 1) команды UI
        let mut quit = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(WCmd::Quit) => {
                    quit = true;
                    break;
                }
                Ok(c) => handle_cmd(&mut st, c, &mut ctx),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            abort_xfer(&mut st, &mut ctx, "закрыто");
            closer.close();
            return;
        }
        // 2) линк
        let mut did = false;
        loop {
            match link.rx.try_recv() {
                Ok(b) => {
                    dec.feed(&b);
                    did = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    ctx.ev(WEvt::Disconnected {
                        err: "соединение закрыто".to_string(),
                    });
                    abort_xfer(&mut st, &mut ctx, "соединение закрыто");
                    closer.close();
                    return;
                }
            }
        }
        loop {
            match dec.poll_frame() {
                Ok(Some(f)) => {
                    if f.channel == Channel::Files {
                        handle_frame(&mut st, f.payload, &mut ctx);
                        did = true;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    dec.reset();
                    break;
                }
            }
        }
        // 3) следующая задача
        if st.cur.is_none() && st.xfer.is_none() && !st.queue.is_empty() {
            start_next(&mut st, &mut ctx);
            did = true;
        }
        // 4) таймауты
        did |= check_timeouts(&mut st, &mut ctx);
        if !did {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

fn start_next(st: &mut State, ctx: &mut Ctx) {
    let Some(c) = st.queue.pop_front() else {
        return;
    };
    match c {
        WCmd::List { path } => {
            if ctx.send(&pack_id_path(files::kind::LIST_REQ, REQ, &path)) {
                st.cur = Some(CurOp {
                    action: "список",
                    path,
                    deadline: Instant::now() + Duration::from_secs(10),
                });
            }
        }
        WCmd::Drives => {
            if ctx.send(&pack_tid(files::kind::DRIVES_REQ, REQ)) {
                st.cur = Some(CurOp {
                    action: "диски",
                    path: String::new(),
                    deadline: Instant::now() + Duration::from_secs(10),
                });
            }
        }
        WCmd::Mkdir { path } => {
            if ctx.send(&pack_id_path(files::kind::MKDIR_REQ, REQ, &path)) {
                st.cur = Some(CurOp {
                    action: "папка создана",
                    path,
                    deadline: Instant::now() + Duration::from_secs(10),
                });
            }
        }
        WCmd::Rename { old, new } => {
            if ctx.send(&files::pack_rename_req(REQ, &old, &new)) {
                st.cur = Some(CurOp {
                    action: "переименовано",
                    path: new,
                    deadline: Instant::now() + Duration::from_secs(10),
                });
            }
        }
        WCmd::Delete { path } => {
            if ctx.send(&pack_id_path(files::kind::DELETE_REQ, REQ, &path)) {
                st.cur = Some(CurOp {
                    action: "удалено",
                    path,
                    deadline: Instant::now() + Duration::from_secs(20),
                });
            }
        }
        WCmd::Download { id, remote, local } => {
            ctx.ev(WEvt::JobStarted { id });
            let exists = std::path::Path::new(&local).exists();
            if exists {
                let x = Xfer {
                    id,
                    up: false,
                    local,
                    remote,
                    file: None,
                    total: 0,
                    pos: 0,
                    credits: 0,
                    mtime: 0,
                    stage: Stage::DlWait,
                    last_progress: Instant::now(),
                    last_emit: Instant::now() - Duration::from_secs(1),
                    del_local_on_fail: false,
                };
                let conflict_path = x.local.clone();
                st.xfer = Some(x);
                ctx.ev(WEvt::Conflict {
                    id,
                    path: conflict_path,
                });
            } else {
                let mut x = Xfer {
                    id,
                    up: false,
                    local,
                    remote,
                    file: None,
                    total: 0,
                    pos: 0,
                    credits: 0,
                    mtime: 0,
                    stage: Stage::DlOpen,
                    last_progress: Instant::now(),
                    last_emit: Instant::now() - Duration::from_secs(1),
                    del_local_on_fail: false,
                };
                if ctx.send(&pack_read_open(TID, &x.remote)) {
                    x.stage = Stage::DlOpen;
                    st.xfer = Some(x);
                } else {
                    ctx.ev(WEvt::JobDone {
                        id,
                        result: Err("линк закрыт".to_string()),
                    });
                }
            }
        }
        WCmd::Upload { id, local, remote } => match std::fs::metadata(&local) {
            Ok(md) => {
                let total = md.len();
                let mtime = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                ctx.ev(WEvt::JobStarted { id });
                let mut x = Xfer {
                    id,
                    up: true,
                    local,
                    remote,
                    file: None,
                    total,
                    pos: 0,
                    credits: 0,
                    mtime,
                    stage: Stage::UlOpen,
                    last_progress: Instant::now(),
                    last_emit: Instant::now() - Duration::from_secs(1),
                    del_local_on_fail: false,
                };
                if ctx.send(&pack_write_open(TID, &x.remote, total, false)) {
                    x.stage = Stage::UlOpen;
                    st.xfer = Some(x);
                } else {
                    ctx.ev(WEvt::JobDone {
                        id,
                        result: Err("линк закрыт".to_string()),
                    });
                }
            }
            Err(e) => {
                ctx.ev(WEvt::JobStarted { id });
                ctx.ev(WEvt::JobDone {
                    id,
                    result: Err(format!("локальный файл: {e}")),
                });
            }
        },
        WCmd::Resolve { .. } | WCmd::Cancel { .. } | WCmd::Quit => {}
    }
}

fn handle_cmd(st: &mut State, c: WCmd, ctx: &mut Ctx) {
    match c {
        WCmd::List { .. }
        | WCmd::Drives
        | WCmd::Mkdir { .. }
        | WCmd::Rename { .. }
        | WCmd::Delete { .. }
        | WCmd::Download { .. }
        | WCmd::Upload { .. } => {
            st.queue.push_back(c);
        }
        WCmd::Resolve { id, action } => {
            let Some(x) = st.xfer.as_mut() else { return };
            if x.id != id {
                return;
            }
            match (x.up, x.stage) {
                (false, Stage::DlWait) => {
                    let action = match action {
                        ConflictAction::Overwrite => Some(()),
                        ConflictAction::Skip => None,
                        ConflictAction::Rename(n) => {
                            let parent = x
                                .local
                                .rsplit_once('\\')
                                .map(|(d, _)| d.to_string())
                                .unwrap_or_default();
                            x.local = format!("{parent}\\{n}");
                            Some(())
                        }
                    };
                    if action.is_some() {
                        let id = x.id;
                        let remote = x.remote.clone();
                        if ctx.send(&pack_read_open(TID, &remote)) {
                            x.stage = Stage::DlOpen;
                        } else {
                            st.xfer = None;
                            ctx.ev(WEvt::JobDone {
                                id,
                                result: Err("линк закрыт".to_string()),
                            });
                        }
                    } else {
                        let id = x.id;
                        st.xfer = None;
                        ctx.ev(WEvt::JobDone {
                            id,
                            result: Err("пропущено".to_string()),
                        });
                    }
                }
                (true, Stage::UlWait) => {
                    let (ow, rename_to): (bool, Option<String>) = match action {
                        ConflictAction::Overwrite => (true, None),
                        ConflictAction::Skip => {
                            let id = x.id;
                            st.xfer = None;
                            ctx.ev(WEvt::JobDone {
                                id,
                                result: Err("пропущено".to_string()),
                            });
                            return;
                        }
                        ConflictAction::Rename(n) => (false, Some(n)),
                    };
                    if let Some(n) = rename_to {
                        let parent = x
                            .remote
                            .rsplit_once('\\')
                            .map(|(d, _)| d.to_string())
                            .unwrap_or_default();
                        x.remote = format!("{parent}\\{n}");
                    }
                    if ctx.send(&pack_write_open(TID, &x.remote, x.total, ow)) {
                        x.stage = Stage::UlOpen;
                    } else {
                        let id = x.id;
                        st.xfer = None;
                        ctx.ev(WEvt::JobDone {
                            id,
                            result: Err("линк закрыт".to_string()),
                        });
                    }
                }
                _ => {}
            }
        }
        WCmd::Cancel { id } => {
            if let Some(x) = st.xfer.as_ref()
                && x.id == id
            {
                abort_xfer(st, ctx, "отменено");
                return;
            }
            let before = st.queue.len();
            st.queue.retain(|q| match q {
                WCmd::Download { id: j, .. } | WCmd::Upload { id: j, .. } => *j != id,
                _ => true,
            });
            if st.queue.len() != before {
                ctx.ev(WEvt::JobDone {
                    id,
                    result: Err("отменено".to_string()),
                });
            }
        }
        WCmd::Quit => {}
    }
}

fn abort_xfer(st: &mut State, ctx: &mut Ctx, why: &str) {
    let Some(x) = st.xfer.take() else { return };
    if x.up {
        let _ = ctx.send(&pack_tid(files::kind::WRITE_ABORT, TID));
    } else {
        let _ = ctx.send(&pack_tid(files::kind::READ_CANCEL, TID));
        if x.del_local_on_fail {
            let _ = std::fs::remove_file(&x.local);
        }
    }
    ctx.ev(WEvt::JobDone {
        id: x.id,
        result: Err(why.to_string()),
    });
}

fn fail_xfer(st: &mut State, ctx: &mut Ctx, id: u64, msg: String) {
    if let Some(x) = st.xfer.as_ref()
        && x.id == id
    {
        abort_xfer(st, ctx, &msg);
    }
}

fn check_timeouts(st: &mut State, ctx: &mut Ctx) -> bool {
    let mut did = false;
    if let Some(op) = &st.cur
        && Instant::now() >= op.deadline
    {
        let (action, path) = (op.action, op.path.clone());
        st.cur = None;
        did = true;
        ctx.ev(WEvt::Op {
            action,
            path,
            result: Err("таймаут ответа".to_string()),
        });
    }
    if let Some(x) = st.xfer.as_ref()
        && x.last_progress.elapsed() > Duration::from_secs(60)
    {
        did = true;
        abort_xfer(st, ctx, "таймаут передачи");
    }
    did
}

fn handle_frame(st: &mut State, payload: &[u8], ctx: &mut Ctx) {
    let Some(&k) = payload.first() else { return };
    match k {
        files::kind::LIST_RESP => {
            if let Some((_id, status, entries)) = parse_list_resp(payload) {
                let path = st
                    .cur
                    .as_ref()
                    .filter(|o| o.action == "список")
                    .map(|o| o.path.clone())
                    .unwrap_or_default();
                st.cur = None;
                let result = if status == st::OK {
                    Ok(entries)
                } else {
                    Err(format!("каталог недоступен (статус {status})"))
                };
                ctx.ev(WEvt::List { path, result });
            }
        }
        files::kind::DRIVES_RESP => {
            if let Some((_id, drives)) = parse_drives_resp(payload) {
                st.cur = None;
                ctx.ev(WEvt::Drives { drives });
            }
        }
        files::kind::OK_RESP => {
            if let Some((_id, status, msg)) = parse_ok_resp(payload)
                && let Some(op) = st.cur.take()
            {
                let result = if status == st::OK {
                    Ok(())
                } else if msg.is_empty() {
                    Err(format!("отказ (статус {status})"))
                } else {
                    Err(msg)
                };
                ctx.ev(WEvt::Op {
                    action: op.action,
                    path: op.path,
                    result,
                });
            }
        }
        files::kind::READ_OPEN_RESP => {
            if let Some((tid, status, size, mtime)) = parse_read_open_resp(payload)
                && tid == TID
                && let Some(x) = st.xfer.as_mut()
                && x.stage == Stage::DlOpen
            {
                if status != st::OK {
                    let id = x.id;
                    st.xfer = None;
                    ctx.ev(WEvt::JobDone {
                        id,
                        result: Err(format!("не удалось открыть файл (статус {status})")),
                    });
                    return;
                }
                match std::fs::File::create(&x.local) {
                    Ok(f) => {
                        x.file = Some(f);
                        x.total = size;
                        x.mtime = mtime;
                        x.pos = 0;
                        x.del_local_on_fail = true;
                        x.last_progress = Instant::now();
                        x.stage = Stage::DlData;
                        let chunks = size.div_ceil(CHUNK as u64) as u16;
                        let initial = chunks.min(WINDOW);
                        let _ = ctx.send(&pack_credit(TID, initial));
                    }
                    Err(e) => {
                        let id = x.id;
                        st.xfer = None;
                        ctx.ev(WEvt::JobDone {
                            id,
                            result: Err(format!("локальный файл: {e}")),
                        });
                    }
                }
            }
        }
        files::kind::DATA => {
            let Some((tid, offset, bytes)) = parse_data(payload) else {
                return;
            };
            if tid != TID {
                return;
            }
            let mut outcome: Option<(u64, Result<(), String>)> = None;
            {
                let Some(x) = st.xfer.as_mut() else { return };
                if x.stage != Stage::DlData {
                    return;
                }
                if offset != x.pos {
                    outcome = Some((x.id, Err("нарушен порядок данных".to_string())));
                } else if let Some(f) = x.file.as_mut() {
                    use std::io::Write as _;
                    match f.write_all(bytes) {
                        Ok(()) => {
                            x.pos += bytes.len() as u64;
                            x.last_progress = Instant::now();
                            if x.pos < x.total {
                                let _ = ctx.send(&pack_credit(TID, 1));
                            } else {
                                let _ = f.flush();
                                if x.mtime > 0 {
                                    let ts = UNIX_EPOCH + Duration::from_secs(x.mtime as u64);
                                    let _ = f.set_modified(ts);
                                }
                                outcome = Some((x.id, Ok(())));
                            }
                        }
                        Err(e) => {
                            outcome = Some((x.id, Err(format!("локальный файл: {e}"))));
                        }
                    }
                } else {
                    outcome = Some((x.id, Err("файл не открыт".to_string())));
                }
            }
            match outcome {
                Some((id, Ok(()))) => {
                    st.xfer = None;
                    ctx.ev(WEvt::JobDone { id, result: Ok(()) });
                }
                Some((id, Err(m))) => fail_xfer(st, ctx, id, m),
                None => {
                    let prog = st
                        .xfer
                        .as_ref()
                        .filter(|x| x.last_emit.elapsed() >= Duration::from_millis(120))
                        .map(|x| (x.id, x.pos, x.total));
                    if let Some((id, done, total)) = prog {
                        if let Some(xm) = st.xfer.as_mut() {
                            xm.last_emit = Instant::now();
                        }
                        ctx.ev(WEvt::Progress { id, done, total });
                    }
                }
            }
        }
        files::kind::WRITE_OPEN_RESP => {
            if let Some((tid, status)) = parse_write_open_resp(payload)
                && tid == TID
                && let Some(x) = st.xfer.as_mut()
                && x.stage == Stage::UlOpen
            {
                if status == st::EXISTS {
                    x.stage = Stage::UlWait;
                    let (id, path) = (x.id, x.remote.clone());
                    ctx.ev(WEvt::Conflict { id, path });
                    return;
                }
                if status != st::OK {
                    let id = x.id;
                    st.xfer = None;
                    ctx.ev(WEvt::JobDone {
                        id,
                        result: Err(format!("не удалось начать загрузку (статус {status})")),
                    });
                    return;
                }
                match std::fs::File::open(&x.local) {
                    Ok(f) => {
                        x.file = Some(f);
                        x.stage = Stage::UlData;
                        x.last_progress = Instant::now();
                    }
                    Err(e) => {
                        let id = x.id;
                        let _ = ctx.send(&pack_tid(files::kind::WRITE_ABORT, TID));
                        st.xfer = None;
                        ctx.ev(WEvt::JobDone {
                            id,
                            result: Err(format!("локальный файл: {e}")),
                        });
                    }
                }
            }
        }
        files::kind::CREDIT => {
            let Some((tid, n)) = parse_credit(payload) else {
                return;
            };
            if tid != TID {
                return;
            }
            let mut failed: Option<String> = None;
            let mut started_close = false;
            {
                let Some(x) = st.xfer.as_mut() else { return };
                if x.stage != Stage::UlData {
                    return;
                }
                x.credits = x.credits.saturating_add(n as u32);
                use std::io::Read as _;
                while x.credits > 0 && x.pos < x.total && failed.is_none() {
                    let want = ((x.total - x.pos).min(CHUNK as u64)) as usize;
                    let mut buf = vec![0u8; want];
                    let read_ok = x
                        .file
                        .as_mut()
                        .map(|f| f.read_exact(&mut buf).is_ok())
                        .unwrap_or(false);
                    if !read_ok {
                        failed = Some("ошибка чтения локального файла".to_string());
                        break;
                    }
                    let frame = pack_data(TID, x.pos, &buf);
                    ctx.enc.push(Channel::Files, 0, &frame);
                    if ctx.link.tx.send(ctx.enc.take_bytes()).is_err() {
                        failed = Some("линк закрыт".to_string());
                        break;
                    }
                    x.pos += want as u64;
                    x.credits -= 1;
                    x.last_progress = Instant::now();
                }
                if failed.is_none() && x.pos >= x.total {
                    started_close = true;
                }
            }
            if let Some(m) = failed {
                abort_xfer(st, ctx, &m);
                return;
            }
            if started_close {
                let mtime = st.xfer.as_ref().map(|x| x.mtime).unwrap_or(0);
                if ctx.send(&pack_write_close(TID, mtime)) {
                    if let Some(x) = st.xfer.as_mut() {
                        x.stage = Stage::UlClose;
                    }
                } else {
                    abort_xfer(st, ctx, "линк закрыт");
                }
            } else {
                let prog = st
                    .xfer
                    .as_ref()
                    .filter(|x| x.last_emit.elapsed() >= Duration::from_millis(120))
                    .map(|x| (x.id, x.pos, x.total));
                if let Some((id, done, total)) = prog {
                    if let Some(xm) = st.xfer.as_mut() {
                        xm.last_emit = Instant::now();
                    }
                    ctx.ev(WEvt::Progress { id, done, total });
                }
            }
        }
        files::kind::WRITE_CLOSE_RESP => {
            if let Some((tid, status)) = parse_write_close_resp(payload)
                && tid == TID
                && let Some(x) = st.xfer.as_mut()
                && x.stage == Stage::UlClose
            {
                let id = x.id;
                st.xfer = None;
                let result = if status == st::OK {
                    Ok(())
                } else {
                    Err(format!("не завершено (статус {status})"))
                };
                ctx.ev(WEvt::JobDone { id, result });
            }
        }
        files::kind::SERV_ERROR => {
            if let Some((tid, msg)) = parse_serv_error(payload)
                && tid == TID
                && let Some(x) = st.xfer.as_ref()
            {
                let id = x.id;
                fail_xfer(st, ctx, id, msg);
            }
        }
        _ => {}
    }
}
