//! Файловый воркер: отдельный поток, работающий с агентом по каналу «Файлы».
//!
//! Используется оконным менеджером файлов (bins/rat): UI шлёт команды [`WCmd`]
//! и получает события [`WEvt`]. Соединение поднимает сам воркер; сразу после
//! подключения видео отключается (файловый режим без экрана).
//!
//! Воркер умеет одиночные файлы и папки целиком: папка сканируется (удалённая —
//! через LIST, локальная — по диску), затем последовательно создаются каталоги
//! и передаются файлы; прогресс — суммарный по заданию.

use crate::files::{
    self, CHUNK, DirEntry, WINDOW, human_size, pack_credit, pack_data, pack_id_path,
    pack_read_open, pack_tid, pack_write_close, pack_write_open, parse_credit, parse_data,
    parse_drives_resp, parse_list_resp, parse_ok_resp, parse_read_open_resp, parse_serv_error,
    parse_write_close_resp, parse_write_open_resp, st,
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
    DownloadFolder {
        id: u64,
        remote: String,
        local: String,
    },
    UploadFolder {
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
    /// Примечание к заданию (например, «папка: 12 файлов, 3,4 МБ»).
    JobNote {
        id: u64,
        text: String,
        bytes: u64,
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
const MAX_FOLDER_FILES: usize = 20000;
const MAX_DEPTH: usize = 64;

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
    /// Это LIST фазы сканирования папки (ответ уйдёт в folder, не в UI).
    scan: bool,
    /// Служебная операция папки (MKDIR): без событий в UI.
    silent: bool,
}

struct FileItem {
    src: String,
    dst: String,
    size: u64,
    mtime: i64,
}

struct FolderJob {
    id: u64,
    up: bool,
    files: VecDeque<FileItem>,
    /// Каталоги к созданию: (путь, создавать_локально).
    pending_mkdirs: VecDeque<(String, bool)>,
    total: u64,
    done_base: u64,
    skipped: u64,
    scanning: bool,
    scan_pairs: VecDeque<(String, String)>,
    cur_scan: Option<(String, String)>,
}

struct State {
    queue: VecDeque<WCmd>,
    cur: Option<CurOp>,
    xfer: Option<Xfer>,
    folder: Option<FolderJob>,
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
        folder: None,
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
        // 3) следующая задача: активная папка важнее очереди
        if st.cur.is_none() && st.xfer.is_none() {
            if st.folder.is_some() {
                advance_folder(&mut st, &mut ctx);
                did = true;
            } else if !st.queue.is_empty() {
                start_next(&mut st, &mut ctx);
                did = true;
            }
        }
        // 4) таймауты
        did |= check_timeouts(&mut st, &mut ctx);
        if !did {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

fn jp(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') || dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn folder_die(st: &mut State, ctx: &mut Ctx, msg: &str) {
    let Some(f) = st.folder.take() else { return };
    if let Some(x) = st.xfer.take() {
        if x.up {
            let _ = ctx.send(&pack_tid(files::kind::WRITE_ABORT, TID));
        } else {
            let _ = ctx.send(&pack_tid(files::kind::READ_CANCEL, TID));
            if x.del_local_on_fail {
                let _ = std::fs::remove_file(&x.local);
            }
        }
    }
    ctx.ev(WEvt::JobDone {
        id: f.id,
        result: Err(msg.to_string()),
    });
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
                    scan: false,
                    silent: false,
                });
            }
        }
        WCmd::Drives => {
            if ctx.send(&pack_tid(files::kind::DRIVES_REQ, REQ)) {
                st.cur = Some(CurOp {
                    action: "диски",
                    path: String::new(),
                    deadline: Instant::now() + Duration::from_secs(10),
                    scan: false,
                    silent: false,
                });
            }
        }
        WCmd::Mkdir { path } => {
            if ctx.send(&pack_id_path(files::kind::MKDIR_REQ, REQ, &path)) {
                st.cur = Some(CurOp {
                    action: "папка создана",
                    path,
                    deadline: Instant::now() + Duration::from_secs(10),
                    scan: false,
                    silent: false,
                });
            }
        }
        WCmd::Rename { old, new } => {
            if ctx.send(&files::pack_rename_req(REQ, &old, &new)) {
                st.cur = Some(CurOp {
                    action: "переименовано",
                    path: new,
                    deadline: Instant::now() + Duration::from_secs(10),
                    scan: false,
                    silent: false,
                });
            }
        }
        WCmd::Delete { path } => {
            if ctx.send(&pack_id_path(files::kind::DELETE_REQ, REQ, &path)) {
                st.cur = Some(CurOp {
                    action: "удалено",
                    path,
                    deadline: Instant::now() + Duration::from_secs(20),
                    scan: false,
                    silent: false,
                });
            }
        }
        WCmd::Download { id, remote, local } => {
            ctx.ev(WEvt::JobStarted { id });
            begin_download_file(st, ctx, id, &remote, &local);
        }
        WCmd::Upload { id, local, remote } => {
            ctx.ev(WEvt::JobStarted { id });
            match std::fs::metadata(&local) {
                Ok(md) => begin_upload_file(st, ctx, id, &local, &remote, md.len(), mtime_of(&md)),
                Err(e) => {
                    ctx.ev(WEvt::JobDone {
                        id,
                        result: Err(format!("локальный файл: {e}")),
                    });
                }
            }
        }
        WCmd::DownloadFolder { id, remote, local } => {
            ctx.ev(WEvt::JobStarted { id });
            let _ = std::fs::create_dir_all(&local);
            let mut pairs = VecDeque::new();
            pairs.push_back((remote, local));
            st.folder = Some(FolderJob {
                id,
                up: false,
                files: VecDeque::new(),
                pending_mkdirs: VecDeque::new(),
                total: 0,
                done_base: 0,
                skipped: 0,
                scanning: true,
                scan_pairs: pairs,
                cur_scan: None,
            });
        }
        WCmd::UploadFolder { id, local, remote } => {
            ctx.ev(WEvt::JobStarted { id });
            match walk_local(&local) {
                Ok((dirs, file_list)) => {
                    let mut pending = VecDeque::new();
                    for rel in dirs {
                        let rp = if rel.is_empty() {
                            remote.clone()
                        } else {
                            jp(&remote, &rel)
                        };
                        pending.push_back((rp, false));
                    }
                    let mut total = 0u64;
                    let mut fl = VecDeque::new();
                    for (rel, size, mt) in file_list {
                        total += size;
                        fl.push_back(FileItem {
                            src: jp(&local, &rel),
                            dst: jp(&remote, &rel),
                            size,
                            mtime: mt,
                        });
                    }
                    ctx.ev(WEvt::JobNote {
                        id,
                        text: format!("папка: {} файлов, {}", fl.len(), human_size(total)),
                        bytes: total,
                    });
                    st.folder = Some(FolderJob {
                        id,
                        up: true,
                        files: fl,
                        pending_mkdirs: pending,
                        total,
                        done_base: 0,
                        skipped: 0,
                        scanning: false,
                        scan_pairs: VecDeque::new(),
                        cur_scan: None,
                    });
                }
                Err(e) => {
                    ctx.ev(WEvt::JobDone { id, result: Err(e) });
                }
            }
        }
        WCmd::Resolve { .. } | WCmd::Cancel { .. } | WCmd::Quit => {}
    }
}

/// Обход локальной папки: (список относительных каталогов, файлы (rel, size, mtime)).
/// Файл при обходе локальной папки: (относительный путь, размер, mtime).
type WalkFile = (String, u64, i64);

type WalkResult = Result<(Vec<String>, Vec<WalkFile>), String>;

fn walk_local(root: &str) -> WalkResult {
    let mut dirs = vec![String::new()];
    let mut files = Vec::new();
    let mut stack = vec![(std::path::PathBuf::from(root), String::new(), 0usize)];
    while let Some((dir, rel, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err("слишком глубокая вложенность папок".to_string());
        }
        let rd = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}\\{name}")
            };
            if let Ok(md) = e.metadata() {
                if md.is_dir() {
                    dirs.push(child_rel.clone());
                    stack.push((e.path(), child_rel, depth + 1));
                } else {
                    files.push((child_rel, md.len(), mtime_of(&md)));
                }
            }
            if files.len() > MAX_FOLDER_FILES {
                return Err(format!("слишком много файлов (макс. {MAX_FOLDER_FILES})"));
            }
        }
    }
    Ok((dirs, files))
}

fn begin_download_file(st: &mut State, ctx: &mut Ctx, id: u64, remote: &str, local: &str) {
    let exists = std::path::Path::new(local).exists();
    if exists {
        st.xfer = Some(Xfer {
            id,
            up: false,
            local: local.to_string(),
            remote: remote.to_string(),
            file: None,
            total: 0,
            pos: 0,
            credits: 0,
            mtime: 0,
            stage: Stage::DlWait,
            last_progress: Instant::now(),
            last_emit: Instant::now() - Duration::from_secs(1),
            del_local_on_fail: false,
        });
        ctx.ev(WEvt::Conflict {
            id,
            path: local.to_string(),
        });
    } else if ctx.send(&pack_read_open(TID, remote)) {
        st.xfer = Some(Xfer {
            id,
            up: false,
            local: local.to_string(),
            remote: remote.to_string(),
            file: None,
            total: 0,
            pos: 0,
            credits: 0,
            mtime: 0,
            stage: Stage::DlOpen,
            last_progress: Instant::now(),
            last_emit: Instant::now() - Duration::from_secs(1),
            del_local_on_fail: false,
        });
    } else {
        ctx.ev(WEvt::JobDone {
            id,
            result: Err("линк закрыт".to_string()),
        });
    }
}

fn begin_upload_file(
    st: &mut State,
    ctx: &mut Ctx,
    id: u64,
    local: &str,
    remote: &str,
    size: u64,
    mtime: i64,
) {
    if ctx.send(&pack_write_open(TID, remote, size, false)) {
        st.xfer = Some(Xfer {
            id,
            up: true,
            local: local.to_string(),
            remote: remote.to_string(),
            file: None,
            total: size,
            pos: 0,
            credits: 0,
            mtime,
            stage: Stage::UlOpen,
            last_progress: Instant::now(),
            last_emit: Instant::now() - Duration::from_secs(1),
            del_local_on_fail: false,
        });
    } else {
        ctx.ev(WEvt::JobDone {
            id,
            result: Err("линк закрыт".to_string()),
        });
    }
}

fn advance_folder(st: &mut State, ctx: &mut Ctx) {
    enum Step {
        SendList(String),
        FinalizeScan {
            id: u64,
            n: usize,
            bytes: u64,
        },
        MkLocal(String),
        MkRemote(String),
        StartFile {
            id: u64,
            up: bool,
            src: String,
            dst: String,
            size: u64,
            mtime: i64,
        },
        Finish {
            id: u64,
            skipped: u64,
        },
    }
    let step = {
        let Some(f) = st.folder.as_mut() else { return };
        if f.scanning {
            if let Some((rd, ld)) = f.scan_pairs.pop_front() {
                f.cur_scan = Some((rd.clone(), ld));
                Step::SendList(rd)
            } else {
                f.scanning = false;
                Step::FinalizeScan {
                    id: f.id,
                    n: f.files.len(),
                    bytes: f.total,
                }
            }
        } else if let Some((path, local)) = f.pending_mkdirs.pop_front() {
            if local {
                Step::MkLocal(path)
            } else {
                Step::MkRemote(path)
            }
        } else if let Some(item) = f.files.pop_front() {
            Step::StartFile {
                id: f.id,
                up: f.up,
                src: item.src,
                dst: item.dst,
                size: item.size,
                mtime: item.mtime,
            }
        } else {
            Step::Finish {
                id: f.id,
                skipped: f.skipped,
            }
        }
    };
    match step {
        Step::SendList(rd) => {
            if ctx.send(&pack_id_path(files::kind::LIST_REQ, REQ, &rd)) {
                st.cur = Some(CurOp {
                    action: "сканирование",
                    path: rd,
                    deadline: Instant::now() + Duration::from_secs(30),
                    scan: true,
                    silent: false,
                });
            } else {
                folder_die(st, ctx, "линк закрыт");
            }
        }
        Step::FinalizeScan { id, n, bytes } => {
            ctx.ev(WEvt::JobNote {
                id,
                text: format!("папка: {n} файлов, {}", human_size(bytes)),
                bytes,
            });
        }
        Step::MkLocal(path) => {
            let _ = std::fs::create_dir_all(&path);
        }
        Step::MkRemote(path) => {
            if ctx.send(&pack_id_path(files::kind::MKDIR_REQ, REQ, &path)) {
                st.cur = Some(CurOp {
                    action: "папка",
                    path,
                    deadline: Instant::now() + Duration::from_secs(10),
                    scan: false,
                    silent: true,
                });
            } else {
                folder_die(st, ctx, "линк закрыт");
            }
        }
        Step::StartFile {
            id,
            up,
            src,
            dst,
            size,
            mtime,
        } => {
            if up {
                begin_upload_file(st, ctx, id, &src, &dst, size, mtime);
            } else {
                begin_download_file(st, ctx, id, &src, &dst);
            }
        }
        Step::Finish { id, skipped } => {
            st.folder = None;
            if skipped > 0 {
                ctx.ev(WEvt::JobNote {
                    id,
                    text: format!("пропущено файлов: {skipped}"),
                    bytes: 0,
                });
            }
            ctx.ev(WEvt::JobDone { id, result: Ok(()) });
        }
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
        | WCmd::Upload { .. }
        | WCmd::DownloadFolder { .. }
        | WCmd::UploadFolder { .. } => {
            st.queue.push_back(c);
        }
        WCmd::Resolve { id, action } => {
            let Some(x) = st.xfer.as_mut() else { return };
            if x.id != id {
                return;
            }
            match (x.up, x.stage) {
                (false, Stage::DlWait) => {
                    let mut skip = false;
                    let mut fail_link = false;
                    let action = match action {
                        ConflictAction::Overwrite => Some(()),
                        ConflictAction::Skip => {
                            skip = true;
                            None
                        }
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
                    if skip {
                        // вне области заимствования ниже
                    } else if action.is_some() {
                        let remote = x.remote.clone();
                        if ctx.send(&pack_read_open(TID, &remote)) {
                            x.stage = Stage::DlOpen;
                        } else {
                            fail_link = true;
                        }
                    }
                    if skip || fail_link {
                        if skip {
                            xfer_finished(st, ctx, Err("пропущено".to_string()));
                        } else {
                            abort_xfer(st, ctx, "линк закрыт");
                        }
                    }
                }
                (true, Stage::UlWait) => {
                    let mut skip = false;
                    let mut fail_link = false;
                    let (ow, rename_to): (bool, Option<String>) = match action {
                        ConflictAction::Overwrite => (true, None),
                        ConflictAction::Skip => {
                            skip = true;
                            (false, None)
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
                    if skip {
                        // ниже
                    } else {
                        let (remote, total) = (x.remote.clone(), x.total);
                        if ctx.send(&pack_write_open(TID, &remote, total, ow)) {
                            x.stage = Stage::UlOpen;
                        } else {
                            fail_link = true;
                        }
                    }
                    if skip || fail_link {
                        if skip {
                            xfer_finished(st, ctx, Err("пропущено".to_string()));
                        } else {
                            abort_xfer(st, ctx, "линк закрыт");
                        }
                    }
                }
                _ => {}
            }
        }
        WCmd::Cancel { id } => {
            if st.folder.as_ref().is_some_and(|f| f.id == id) && st.xfer.is_none() {
                st.folder = None;
                if st.cur.as_ref().is_some_and(|o| o.scan || o.silent) {
                    st.cur = None;
                }
                ctx.ev(WEvt::JobDone {
                    id,
                    result: Err("отменено".to_string()),
                });
                return;
            }
            if let Some(x) = st.xfer.as_ref()
                && x.id == id
            {
                abort_xfer(st, ctx, "отменено");
                return;
            }
            let before = st.queue.len();
            st.queue.retain(|q| match q {
                WCmd::Download { id: j, .. }
                | WCmd::Upload { id: j, .. }
                | WCmd::DownloadFolder { id: j, .. }
                | WCmd::UploadFolder { id: j, .. } => *j != id,
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

/// Завершение одной передачи: в составе папки — продолжаем, иначе JobDone.
fn xfer_finished(st: &mut State, ctx: &mut Ctx, result: Result<(), String>) {
    let Some(x) = st.xfer.take() else { return };
    finish_result(st, ctx, x, result);
}

fn finish_result(st: &mut State, ctx: &mut Ctx, x: Xfer, result: Result<(), String>) {
    let in_folder = st.folder.as_ref().is_some_and(|f| f.id == x.id);
    if in_folder {
        match &result {
            Ok(()) => {
                if let Some(f) = st.folder.as_mut() {
                    f.done_base += x.total;
                }
                return;
            }
            Err(m) if m == "пропущено" => {
                if let Some(f) = st.folder.as_mut() {
                    f.skipped += 1;
                }
                return;
            }
            Err(_) => {
                st.folder = None;
                ctx.ev(WEvt::JobDone { id: x.id, result });
                return;
            }
        }
    }
    ctx.ev(WEvt::JobDone { id: x.id, result });
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
    finish_result(st, ctx, x, Err(why.to_string()));
}

fn fail_xfer(st: &mut State, ctx: &mut Ctx, id: u64, msg: String) {
    if st.xfer.as_ref().is_some_and(|x| x.id == id) {
        abort_xfer(st, ctx, &msg);
    }
}

fn check_timeouts(st: &mut State, ctx: &mut Ctx) -> bool {
    let mut did = false;
    if let Some(op) = &st.cur
        && Instant::now() >= op.deadline
    {
        let (action, path, special) = (op.action, op.path.clone(), op.scan || op.silent);
        st.cur = None;
        did = true;
        if special {
            folder_die(st, ctx, "таймаут подготовки папки");
        } else {
            ctx.ev(WEvt::Op {
                action,
                path,
                result: Err("таймаут ответа".to_string()),
            });
        }
    }
    if let Some(x) = st.xfer.as_ref()
        && x.last_progress.elapsed() > Duration::from_secs(60)
    {
        did = true;
        abort_xfer(st, ctx, "таймаут передачи");
    }
    did
}

/// Эффективный прогресс для событий (учитывает базовое смещение папки).
fn progress_values(st: &State) -> Option<(u64, u64, u64)> {
    let x = st.xfer.as_ref()?;
    let (base, total) = match &st.folder {
        Some(f) if f.id == x.id => (f.done_base, f.total),
        _ => (0, x.total),
    };
    let total = if total > 0 { total } else { x.total };
    Some((x.id, base + x.pos, total))
}

fn handle_frame(st: &mut State, payload: &[u8], ctx: &mut Ctx) {
    let Some(&k) = payload.first() else { return };
    match k {
        files::kind::LIST_RESP => {
            if let Some((_id, status, entries)) = parse_list_resp(payload) {
                let is_scan = st.cur.as_ref().map(|o| o.scan).unwrap_or(false);
                if is_scan {
                    st.cur = None;
                    let mut fail: Option<String> = None;
                    if status != st::OK {
                        fail = Some(format!("каталог недоступен (статус {status})"));
                    } else if let Some(f) = st.folder.as_mut() {
                        if let Some((rd, ld)) = f.cur_scan.take() {
                            for e in &entries {
                                if e.is_dir {
                                    let r2 = jp(&rd, &e.name);
                                    let l2 = jp(&ld, &e.name);
                                    let _ = std::fs::create_dir_all(&l2);
                                    f.scan_pairs.push_back((r2, l2));
                                } else {
                                    f.files.push_back(FileItem {
                                        src: jp(&rd, &e.name),
                                        dst: jp(&ld, &e.name),
                                        size: e.size,
                                        mtime: 0,
                                    });
                                    f.total += e.size;
                                }
                            }
                            if f.files.len() > MAX_FOLDER_FILES {
                                fail = Some(format!(
                                    "слишком много файлов (макс. {MAX_FOLDER_FILES})"
                                ));
                            }
                        }
                    } else {
                        fail = Some("внутренняя ошибка сканирования".to_string());
                    }
                    if let Some(m) = fail {
                        folder_die(st, ctx, &m);
                    }
                    return;
                }
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
            if let Some((_id, status, msg)) = parse_ok_resp(payload) {
                let Some(op) = st.cur.take() else { return };
                if op.silent {
                    if status != st::OK && status != st::EXISTS {
                        let m = if msg.is_empty() {
                            format!("отказ (статус {status})")
                        } else {
                            msg
                        };
                        let text = format!("папка {}: {m}", op.path);
                        folder_die(st, ctx, &text);
                    }
                    return;
                }
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
                let mut done_zero = false;
                if status != st::OK {
                    abort_xfer(
                        st,
                        ctx,
                        &format!("не удалось открыть файл (статус {status})"),
                    );
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
                        if size == 0 {
                            if let Some(f) = x.file.as_mut() {
                                use std::io::Write as _;
                                let _ = f.flush();
                                if mtime > 0 {
                                    let ts = UNIX_EPOCH + Duration::from_secs(mtime as u64);
                                    let _ = f.set_modified(ts);
                                }
                            }
                            let _ = ctx.send(&pack_tid(files::kind::READ_CANCEL, TID));
                            done_zero = true;
                        } else {
                            x.stage = Stage::DlData;
                            let chunks = size.div_ceil(CHUNK as u64) as u16;
                            let initial = chunks.min(WINDOW);
                            let _ = ctx.send(&pack_credit(TID, initial));
                        }
                    }
                    Err(e) => {
                        let msg = format!("локальный файл: {e}");
                        abort_xfer(st, ctx, &msg);
                    }
                }
                if done_zero {
                    xfer_finished(st, ctx, Ok(()));
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
            let mut outcome: Option<Result<(), String>> = None;
            {
                let Some(x) = st.xfer.as_mut() else { return };
                if x.stage != Stage::DlData {
                    return;
                }
                if offset != x.pos {
                    outcome = Some(Err("нарушен порядок данных".to_string()));
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
                                outcome = Some(Ok(()));
                            }
                        }
                        Err(e) => {
                            outcome = Some(Err(format!("локальный файл: {e}")));
                        }
                    }
                } else {
                    outcome = Some(Err("файл не открыт".to_string()));
                }
            }
            match outcome {
                Some(Ok(())) => xfer_finished(st, ctx, Ok(())),
                Some(Err(m)) => {
                    let id = st.xfer.as_ref().map(|x| x.id).unwrap_or(0);
                    fail_xfer(st, ctx, id, m);
                }
                None => {
                    let prog = progress_values(st).filter(|_| {
                        st.xfer
                            .as_ref()
                            .is_some_and(|x| x.last_emit.elapsed() >= Duration::from_millis(120))
                    });
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
                    let msg = format!("не удалось начать загрузку (статус {status})");
                    abort_xfer(st, ctx, &msg);
                    return;
                }
                match std::fs::File::open(&x.local) {
                    Ok(f) => {
                        x.file = Some(f);
                        x.stage = Stage::UlData;
                        x.last_progress = Instant::now();
                    }
                    Err(e) => {
                        let msg = format!("локальный файл: {e}");
                        abort_xfer(st, ctx, &msg);
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
                let prog = progress_values(st).filter(|_| {
                    st.xfer
                        .as_ref()
                        .is_some_and(|x| x.last_emit.elapsed() >= Duration::from_millis(120))
                });
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
                let _ = x;
                let result = if status == st::OK {
                    Ok(())
                } else {
                    Err(format!("не завершено (статус {status})"))
                };
                xfer_finished(st, ctx, result);
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

/// Самопроверка: загрузить папку `local_src` в `remote_dst`, затем скачать её в
/// `local_dl`; вернуть сводку. Используется инструментом `--files-worker-test`.
pub fn selftest(
    target: Target,
    local_src: &str,
    remote_dst: &str,
    local_dl: &str,
) -> Result<String, String> {
    let (tx, rx) = spawn(target);
    let mut out = String::new();
    // Подключение
    let t0 = Instant::now();
    loop {
        if t0.elapsed() > Duration::from_secs(20) {
            return Err("таймаут подключения".to_string());
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(WEvt::Connected) => break,
            Ok(WEvt::ConnectFailed { err }) => return Err(format!("подключение: {err}")),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("воркер остановился".to_string());
            }
        }
    }
    out.push_str("подключено\n");
    let _ = tx.send(WCmd::UploadFolder {
        id: 1,
        local: local_src.to_string(),
        remote: remote_dst.to_string(),
    });
    if let Err(e) = drive_job(&tx, &rx, 1, &mut out) {
        return Err(format!("{out}ОШИБКА: {e}"));
    }
    let _ = tx.send(WCmd::DownloadFolder {
        id: 2,
        remote: remote_dst.to_string(),
        local: local_dl.to_string(),
    });
    if let Err(e) = drive_job(&tx, &rx, 2, &mut out) {
        return Err(format!("{out}ОШИБКА: {e}"));
    }
    let _ = tx.send(WCmd::Quit);
    Ok(out)
}

fn drive_job(
    tx: &Sender<WCmd>,
    rx: &Receiver<WEvt>,
    id: u64,
    out: &mut String,
) -> Result<(), String> {
    let t0 = Instant::now();
    loop {
        if t0.elapsed() > Duration::from_secs(600) {
            return Err(format!("таймаут задания {id}"));
        }
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(WEvt::JobStarted { id: j }) if j == id => out.push_str("задание начато\n"),
            Ok(WEvt::JobNote { id: j, text, .. }) if j == id => {
                out.push_str(&format!("{text}\n"));
            }
            Ok(WEvt::Progress { id: j, done, total }) if j == id => {
                out.push_str(&format!(
                    "прогресс: {} / {}\n",
                    human_size(done),
                    human_size(total)
                ));
            }
            Ok(WEvt::Conflict { id: j, .. }) if j == id => {
                let _ = tx.send(WCmd::Resolve {
                    id: j,
                    action: ConflictAction::Overwrite,
                });
            }
            Ok(WEvt::JobDone { id: j, result }) if j == id => {
                return match result {
                    Ok(()) => {
                        out.push_str(&format!(
                            "задание {id} готово за {:.1} с\n",
                            t0.elapsed().as_secs_f32()
                        ));
                        Ok(())
                    }
                    Err(e) => Err(format!("задание {id}: {e}")),
                };
            }
            Ok(WEvt::Disconnected { err }) => return Err(err),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("воркер остановился".to_string());
            }
        }
    }
}
