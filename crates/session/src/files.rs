//! Канал «Файлы»: список каталогов, скачивание и загрузка файлов.
//!
//! Сообщения — сырые бинарные кадры канала [`rat_mux::Channel::Files`]
//! (многобайтовые поля — big-endian, строки — u16-длина + UTF-8).
//!
//! Запрос/ответ (операции с путями):
//! - `LIST_REQ {id, path}` → `LIST_RESP {id, status, entries[]}`
//! - `MKDIR_REQ {id, path}` → `OK_RESP {id, status, msg}`
//! - `RENAME_REQ {id, old, new}` → `OK_RESP`
//! - `DELETE_REQ {id, path}` → `OK_RESP`
//! - `DRIVES_REQ {id}` → `DRIVES_RESP {id, drives[]}`
//!
//! Передача данных (окно кредитов, чанки по 256 КиБ):
//! - чтение (удалённая → локальная): `READ_OPEN {tid, path}` →
//!   `READ_OPEN_RESP {tid, status, size, mtime}`; клиент шлёт `CREDIT {tid, n}`,
//!   сервер отвечает `DATA {tid, offset, bytes}`; `READ_CANCEL {tid}` — стоп;
//! - запись (локальная → удалённая): `WRITE_OPEN {tid, path, size, overwrite}` →
//!   `WRITE_OPEN_RESP {tid, status}` (при успехе сервер сразу даёт кредиты);
//!   клиент шлёт `DATA` в пределах кредитов; `WRITE_CLOSE {tid, mtime}` →
//!   `WRITE_CLOSE_RESP {tid, status}`; `WRITE_ABORT {tid}` — прервать.
//!   Файл пишется во временный «.ratpart» и переименовывается при закрытии.
//!
//! Ошибки трансфера сервер шлёт как `SERV_ERROR {tid, msg}`.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Размер чанка передачи (256 КиБ).
pub const CHUNK: usize = 256 * 1024;
/// Начальное окно кредитов (в чанках).
pub const WINDOW: u16 = 16;

/// Виды сообщений канала Files.
pub mod kind {
    pub const LIST_REQ: u8 = 1;
    pub const LIST_RESP: u8 = 2;
    pub const MKDIR_REQ: u8 = 3;
    pub const OK_RESP: u8 = 4;
    pub const RENAME_REQ: u8 = 5;
    pub const DELETE_REQ: u8 = 6;
    pub const DRIVES_REQ: u8 = 7;
    pub const DRIVES_RESP: u8 = 8;
    pub const READ_OPEN: u8 = 9;
    pub const READ_OPEN_RESP: u8 = 10;
    pub const CREDIT: u8 = 11;
    pub const DATA: u8 = 12;
    pub const READ_CANCEL: u8 = 14;
    pub const WRITE_OPEN: u8 = 15;
    pub const WRITE_OPEN_RESP: u8 = 16;
    pub const WRITE_CLOSE: u8 = 17;
    pub const WRITE_CLOSE_RESP: u8 = 18;
    pub const WRITE_ABORT: u8 = 19;
    pub const SERV_ERROR: u8 = 20;
    pub const FSINFO_REQ: u8 = 21;
    pub const FSINFO_RESP: u8 = 22;
}

/// Коды статуса ответов.
pub mod st {
    pub const OK: u8 = 0;
    pub const NOT_FOUND: u8 = 1;
    pub const DENIED: u8 = 2;
    pub const EXISTS: u8 = 3;
    pub const IO_ERR: u8 = 4;
    pub const BAD_REQ: u8 = 5;
}

/// Запись каталога.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// Размер в байтах (для каталогов — 0).
    pub size: u64,
    /// Время изменения (секунды Unix; 0 — неизвестно).
    pub mtime: i64,
}

// ===================== чтение/запись примитивов =====================

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, p: 0 }
    }
    fn left(&self) -> usize {
        self.b.len() - self.p
    }
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    fn u16v(&mut self) -> Option<u16> {
        if self.left() < 2 {
            return None;
        }
        let v = u16::from_be_bytes([self.b[self.p], self.b[self.p + 1]]);
        self.p += 2;
        Some(v)
    }
    fn u32v(&mut self) -> Option<u32> {
        if self.left() < 4 {
            return None;
        }
        let v = u32::from_be_bytes([
            self.b[self.p],
            self.b[self.p + 1],
            self.b[self.p + 2],
            self.b[self.p + 3],
        ]);
        self.p += 4;
        Some(v)
    }
    fn u64v(&mut self) -> Option<u64> {
        if self.left() < 8 {
            return None;
        }
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[self.p..self.p + 8]);
        self.p += 8;
        Some(u64::from_be_bytes(a))
    }
    fn i64v(&mut self) -> Option<i64> {
        Some(self.u64v()? as i64)
    }
    fn s(&mut self) -> Option<String> {
        let n = self.u16v()? as usize;
        if self.left() < n {
            return None;
        }
        let s = String::from_utf8_lossy(&self.b[self.p..self.p + n]).into_owned();
        self.p += n;
        Some(s)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.left() < n {
            return None;
        }
        let r = &self.b[self.p..self.p + n];
        self.p += n;
        Some(r)
    }
}

fn wu8(v: &mut Vec<u8>, x: u8) {
    v.push(x);
}
fn wu16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn wu32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn wu64(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn wi64(v: &mut Vec<u8>, x: i64) {
    wu64(v, x as u64);
}
fn wstr(v: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    let n = b.len().min(u16::MAX as usize);
    wu16(v, n as u16);
    v.extend_from_slice(&b[..n]);
}

// ===================== упаковка сообщений =====================

pub fn pack_id_path(k: u8, id: u32, path: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + 4 + 2 + path.len());
    wu8(&mut v, k);
    wu32(&mut v, id);
    wstr(&mut v, path);
    v
}

pub fn parse_id_path(payload: &[u8], k: u8) -> Option<(u32, String)> {
    let mut r = Rd::new(payload);
    if r.u8()? != k {
        return None;
    }
    let id = r.u32v()?;
    let path = r.s()?;
    Some((id, path))
}

pub fn pack_list_resp(id: u32, status: u8, entries: &[DirEntry]) -> Vec<u8> {
    let mut v = Vec::with_capacity(16 + entries.len() * 24);
    wu8(&mut v, kind::LIST_RESP);
    wu32(&mut v, id);
    wu8(&mut v, status);
    wu32(&mut v, entries.len() as u32);
    for e in entries {
        let mut fl = 0u8;
        if e.is_dir {
            fl |= 1;
        }
        wu8(&mut v, fl);
        wu64(&mut v, e.size);
        wi64(&mut v, e.mtime);
        wstr(&mut v, &e.name);
    }
    v
}

pub fn parse_list_resp(payload: &[u8]) -> Option<(u32, u8, Vec<DirEntry>)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::LIST_RESP {
        return None;
    }
    let id = r.u32v()?;
    let st = r.u8()?;
    let n = r.u32v()? as usize;
    if n > 200_000 {
        return None;
    }
    let mut entries = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let fl = r.u8()?;
        let size = r.u64v()?;
        let mtime = r.i64v()?;
        let name = r.s()?;
        entries.push(DirEntry {
            name,
            is_dir: fl & 1 != 0,
            size,
            mtime,
        });
    }
    Some((id, st, entries))
}

pub fn pack_ok_resp(id: u32, status: u8, msg: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + msg.len());
    wu8(&mut v, kind::OK_RESP);
    wu32(&mut v, id);
    wu8(&mut v, status);
    wstr(&mut v, msg);
    v
}

pub fn parse_ok_resp(payload: &[u8]) -> Option<(u32, u8, String)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::OK_RESP {
        return None;
    }
    let id = r.u32v()?;
    let st = r.u8()?;
    let msg = r.s()?;
    Some((id, st, msg))
}

pub fn pack_rename_req(id: u32, old: &str, new: &str) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::RENAME_REQ);
    wu32(&mut v, id);
    wstr(&mut v, old);
    wstr(&mut v, new);
    v
}

pub fn parse_rename_req(payload: &[u8]) -> Option<(u32, String, String)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::RENAME_REQ {
        return None;
    }
    let id = r.u32v()?;
    let old = r.s()?;
    let new = r.s()?;
    Some((id, old, new))
}

pub fn pack_drives_resp(id: u32, drives: &[String]) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::DRIVES_RESP);
    wu32(&mut v, id);
    wu8(&mut v, drives.len().min(255) as u8);
    for d in drives.iter().take(255) {
        wstr(&mut v, d);
    }
    v
}

pub fn parse_drives_resp(payload: &[u8]) -> Option<(u32, Vec<String>)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::DRIVES_RESP {
        return None;
    }
    let id = r.u32v()?;
    let n = r.u8()? as usize;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(r.s()?);
    }
    Some((id, out))
}

pub fn pack_fsinfo_resp(id: u32, status: u8, free: u64, total: u64) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::FSINFO_RESP);
    wu32(&mut v, id);
    wu8(&mut v, status);
    wu64(&mut v, free);
    wu64(&mut v, total);
    v
}

pub fn parse_fsinfo_resp(payload: &[u8]) -> Option<(u32, u8, u64, u64)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::FSINFO_RESP {
        return None;
    }
    let id = r.u32v()?;
    let status = r.u8()?;
    let free = r.u64v()?;
    let total = r.u64v()?;
    Some((id, status, free, total))
}

pub fn pack_read_open(tid: u32, path: &str) -> Vec<u8> {
    pack_id_path(kind::READ_OPEN, tid, path)
}

pub fn pack_read_open_resp(tid: u32, status: u8, size: u64, mtime: i64) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::READ_OPEN_RESP);
    wu32(&mut v, tid);
    wu8(&mut v, status);
    wu64(&mut v, size);
    wi64(&mut v, mtime);
    v
}

pub fn parse_read_open_resp(payload: &[u8]) -> Option<(u32, u8, u64, i64)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::READ_OPEN_RESP {
        return None;
    }
    let tid = r.u32v()?;
    let st = r.u8()?;
    let size = r.u64v()?;
    let mtime = r.i64v()?;
    Some((tid, st, size, mtime))
}

pub fn pack_credit(tid: u32, n: u16) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::CREDIT);
    wu32(&mut v, tid);
    wu16(&mut v, n);
    v
}

pub fn parse_credit(payload: &[u8]) -> Option<(u32, u16)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::CREDIT {
        return None;
    }
    Some((r.u32v()?, r.u16v()?))
}

pub fn pack_data(tid: u32, offset: u64, bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(17 + bytes.len());
    wu8(&mut v, kind::DATA);
    wu32(&mut v, tid);
    wu64(&mut v, offset);
    wu32(&mut v, bytes.len() as u32);
    v.extend_from_slice(bytes);
    v
}

pub fn parse_data(payload: &[u8]) -> Option<(u32, u64, &[u8])> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::DATA {
        return None;
    }
    let tid = r.u32v()?;
    let offset = r.u64v()?;
    let len = r.u32v()? as usize;
    let bytes = r.take(len)?;
    Some((tid, offset, bytes))
}

pub fn pack_tid(k: u8, tid: u32) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, k);
    wu32(&mut v, tid);
    v
}

pub fn parse_tid(payload: &[u8], k: u8) -> Option<u32> {
    let mut r = Rd::new(payload);
    if r.u8()? != k {
        return None;
    }
    r.u32v()
}

pub fn pack_write_open(tid: u32, path: &str, size: u64, overwrite: bool) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::WRITE_OPEN);
    wu32(&mut v, tid);
    wstr(&mut v, path);
    wu64(&mut v, size);
    wu8(&mut v, u8::from(overwrite));
    v
}

pub fn parse_write_open(payload: &[u8]) -> Option<(u32, String, u64, bool)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::WRITE_OPEN {
        return None;
    }
    let tid = r.u32v()?;
    let path = r.s()?;
    let size = r.u64v()?;
    let ow = r.u8()? != 0;
    Some((tid, path, size, ow))
}

pub fn pack_write_open_resp(tid: u32, status: u8) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::WRITE_OPEN_RESP);
    wu32(&mut v, tid);
    wu8(&mut v, status);
    v
}

pub fn parse_write_open_resp(payload: &[u8]) -> Option<(u32, u8)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::WRITE_OPEN_RESP {
        return None;
    }
    Some((r.u32v()?, r.u8()?))
}

pub fn pack_write_close(tid: u32, mtime: i64) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::WRITE_CLOSE);
    wu32(&mut v, tid);
    wi64(&mut v, mtime);
    v
}

pub fn parse_write_close(payload: &[u8]) -> Option<(u32, i64)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::WRITE_CLOSE {
        return None;
    }
    Some((r.u32v()?, r.i64v()?))
}

pub fn pack_write_close_resp(tid: u32, status: u8) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::WRITE_CLOSE_RESP);
    wu32(&mut v, tid);
    wu8(&mut v, status);
    v
}

pub fn parse_write_close_resp(payload: &[u8]) -> Option<(u32, u8)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::WRITE_CLOSE_RESP {
        return None;
    }
    Some((r.u32v()?, r.u8()?))
}

pub fn pack_serv_error(tid: u32, msg: &str) -> Vec<u8> {
    let mut v = Vec::new();
    wu8(&mut v, kind::SERV_ERROR);
    wu32(&mut v, tid);
    wstr(&mut v, msg);
    v
}

pub fn parse_serv_error(payload: &[u8]) -> Option<(u32, String)> {
    let mut r = Rd::new(payload);
    if r.u8()? != kind::SERV_ERROR {
        return None;
    }
    let tid = r.u32v()?;
    let msg = r.s()?;
    Some((tid, msg))
}

// ===================== серверная сторона (агент) =====================

fn mtime_secs(t: Option<SystemTime>) -> i64 {
    t.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn io_status(e: &std::io::Error) -> (u8, String) {
    match e.kind() {
        std::io::ErrorKind::NotFound => (st::NOT_FOUND, format!("не найдено: {e}")),
        std::io::ErrorKind::PermissionDenied => (st::DENIED, format!("нет доступа: {e}")),
        std::io::ErrorKind::AlreadyExists => (st::EXISTS, format!("уже существует: {e}")),
        _ => (st::IO_ERR, format!("ошибка: {e}")),
    }
}

#[cfg(windows)]
pub fn local_drives() -> Vec<String> {
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) != 0 {
            let c = (b'A' + i as u8) as char;
            out.push(format!("{c}:\\"));
        }
    }
    out
}

#[cfg(not(windows))]
pub fn local_drives() -> Vec<String> {
    Vec::new()
}

/// Свободно/всего на томе, содержащем путь (Windows).
#[cfg(windows)]
pub fn drive_space(path: &str) -> Option<(u64, u64)> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::PCWSTR;
    fn probe(p: &str, free: &mut u64, total: &mut u64) -> bool {
        let w: Vec<u16> = p.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(w.as_ptr()),
                Some(free as *mut u64),
                Some(total as *mut u64),
                None,
            )
            .is_ok()
        }
    }
    let mut free = 0u64;
    let mut total = 0u64;
    if probe(path, &mut free, &mut total) {
        return Some((free, total));
    }
    // Путь мог исчезнуть — пробуем корень диска («C:\»).
    if path.as_bytes().get(1) == Some(&b':')
        && let Some(c) = path.chars().next()
    {
        let root = format!("{c}:\\");
        if probe(&root, &mut free, &mut total) {
            return Some((free, total));
        }
    }
    None
}

#[cfg(not(windows))]
pub fn drive_space(_path: &str) -> Option<(u64, u64)> {
    None
}

struct ReadT {
    file: std::fs::File,
    path: String,
    offset: u64,
    size: u64,
    credits: u32,
}

struct WriteT {
    file: std::fs::File,
    part: std::path::PathBuf,
    final_path: String,
    written: u64,
    declared: u64,
}

/// Серверная часть канала Files (живёт в потоке сессии агента).
#[derive(Default)]
pub struct Server {
    reads: HashMap<u32, ReadT>,
    writes: HashMap<u32, WriteT>,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Обработать входящее сообщение; кадры-ответы отдать через `emit(flags, payload)`.
    /// Возвращает строки для журнала.
    pub fn handle(&mut self, payload: &[u8], emit: &mut dyn FnMut(u8, &[u8])) -> Vec<String> {
        let mut logs = Vec::new();
        let Some(&k) = payload.first() else {
            return logs;
        };
        match k {
            kind::LIST_REQ => {
                if let Some((id, path)) = parse_id_path(payload, kind::LIST_REQ) {
                    match do_list(&path) {
                        Ok(entries) => {
                            emit(0, &pack_list_resp(id, st::OK, &entries));
                        }
                        Err((code, msg)) => {
                            emit(0, &pack_list_resp(id, code, &[]));
                            logs.push(format!("[files] список {path}: {msg}"));
                        }
                    }
                }
            }
            kind::MKDIR_REQ => {
                if let Some((id, path)) = parse_id_path(payload, kind::MKDIR_REQ) {
                    let (code, msg) = match std::fs::create_dir(&path) {
                        Ok(()) => (st::OK, String::new()),
                        Err(e) => io_status(&e),
                    };
                    emit(0, &pack_ok_resp(id, code, &msg));
                }
            }
            kind::RENAME_REQ => {
                if let Some((id, old, new)) = parse_rename_req(payload) {
                    let (code, msg) = match std::fs::rename(&old, &new) {
                        Ok(()) => (st::OK, String::new()),
                        Err(e) => io_status(&e),
                    };
                    emit(0, &pack_ok_resp(id, code, &msg));
                }
            }
            kind::DELETE_REQ => {
                if let Some((id, path)) = parse_id_path(payload, kind::DELETE_REQ) {
                    let (code, msg) = match do_delete(&path) {
                        Ok(()) => (st::OK, String::new()),
                        Err((c, m)) => (c, m),
                    };
                    emit(0, &pack_ok_resp(id, code, &msg));
                }
            }
            kind::DRIVES_REQ => {
                if let Some(id) = parse_tid(payload, kind::DRIVES_REQ) {
                    emit(0, &pack_drives_resp(id, &local_drives()));
                }
            }
            kind::FSINFO_REQ => {
                if let Some((id, path)) = parse_id_path(payload, kind::FSINFO_REQ) {
                    match drive_space(&path) {
                        Some((free, total)) => {
                            emit(0, &pack_fsinfo_resp(id, st::OK, free, total));
                        }
                        None => {
                            emit(0, &pack_fsinfo_resp(id, st::IO_ERR, 0, 0));
                        }
                    }
                }
            }
            kind::READ_OPEN => {
                if let Some((tid, path)) = parse_id_path(payload, kind::READ_OPEN) {
                    match std::fs::File::open(&path) {
                        Ok(f) => {
                            let md = f.metadata();
                            let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                            let mtime = md.as_ref().ok().and_then(|m| m.modified().ok());
                            self.reads.insert(
                                tid,
                                ReadT {
                                    file: f,
                                    path: path.clone(),
                                    offset: 0,
                                    size,
                                    credits: 0,
                                },
                            );
                            emit(
                                0,
                                &pack_read_open_resp(tid, st::OK, size, mtime_secs(mtime)),
                            );
                            logs.push(format!("[files] отправка: {path} ({size} Б)"));
                        }
                        Err(e) => {
                            let (code, _) = io_status(&e);
                            emit(0, &pack_read_open_resp(tid, code, 0, 0));
                        }
                    }
                }
            }
            kind::CREDIT => {
                if let Some((tid, n)) = parse_credit(payload) {
                    let mut done = false;
                    if let Some(t) = self.reads.get_mut(&tid) {
                        t.credits = t.credits.saturating_add(n as u32);
                        while t.credits > 0 && t.offset < t.size {
                            let want = ((t.size - t.offset).min(CHUNK as u64)) as usize;
                            let mut buf = vec![0u8; want];
                            use std::io::Read as _;
                            match t.file.read_exact(&mut buf) {
                                Ok(()) => {
                                    let frame = pack_data(tid, t.offset, &buf);
                                    emit(0, &frame);
                                    t.offset += want as u64;
                                    t.credits -= 1;
                                }
                                Err(e) => {
                                    let frame = pack_serv_error(tid, &format!("чтение: {e}"));
                                    emit(0, &frame);
                                    logs.push(format!("[files] чтение {}: {e}", t.path));
                                    done = true;
                                    break;
                                }
                            }
                        }
                        if t.offset >= t.size {
                            done = true;
                        }
                    }
                    if done {
                        self.reads.remove(&tid);
                    }
                }
            }
            kind::READ_CANCEL => {
                if let Some(tid) = parse_tid(payload, kind::READ_CANCEL) {
                    self.reads.remove(&tid);
                }
            }
            kind::WRITE_OPEN => {
                if let Some((tid, path, size, ow)) = parse_write_open(payload) {
                    let exists = std::path::Path::new(&path).exists();
                    if exists && !ow {
                        emit(0, &pack_write_open_resp(tid, st::EXISTS));
                    } else {
                        let part = format!("{path}.ratpart");
                        match std::fs::File::create(&part) {
                            Ok(f) => {
                                self.writes.insert(
                                    tid,
                                    WriteT {
                                        file: f,
                                        part: std::path::PathBuf::from(&part),
                                        final_path: path.clone(),
                                        written: 0,
                                        declared: size,
                                    },
                                );
                                emit(0, &pack_write_open_resp(tid, st::OK));
                                emit(0, &pack_credit(tid, WINDOW));
                                logs.push(format!("[files] приём: {path} ({size} Б)"));
                            }
                            Err(e) => {
                                let (code, _) = io_status(&e);
                                emit(0, &pack_write_open_resp(tid, code));
                            }
                        }
                    }
                }
            }
            kind::DATA => {
                if let Some((tid, offset, bytes)) = parse_data(payload) {
                    let mut err: Option<String> = None;
                    let mut got = false;
                    if let Some(t) = self.writes.get_mut(&tid) {
                        got = true;
                        use std::io::Write as _;
                        if offset == t.written {
                            match t.file.write_all(bytes) {
                                Ok(()) => {
                                    t.written += bytes.len() as u64;
                                    emit(0, &pack_credit(tid, 1));
                                }
                                Err(e) => {
                                    err = Some(format!("запись {}: {e}", t.final_path));
                                }
                            }
                        } else {
                            err = Some(format!(
                                "нарушен порядок данных: ждали {}, получили {offset}",
                                t.written
                            ));
                        }
                    }
                    if let Some(m) = err {
                        let frame = pack_serv_error(tid, &m);
                        emit(0, &frame);
                        if let Some(t) = self.writes.remove(&tid) {
                            drop(t.file);
                            let _ = std::fs::remove_file(&t.part);
                        }
                        logs.push(format!("[files] {m}"));
                    } else if !got {
                        emit(0, &pack_serv_error(tid, "неизвестная передача"));
                    }
                }
            }
            kind::WRITE_CLOSE => {
                if let Some((tid, mtime)) = parse_write_close(payload) {
                    match self.writes.remove(&tid) {
                        Some(t) => {
                            let fname = t.final_path.clone();
                            let (code, msg) = if t.written == t.declared {
                                match finish_write(t, mtime) {
                                    Ok(()) => (st::OK, String::new()),
                                    Err(e) => io_status(&e),
                                }
                            } else {
                                let msg = format!("получено {} из {} Б", t.written, t.declared);
                                drop(t.file);
                                let _ = std::fs::remove_file(&t.part);
                                (st::IO_ERR, msg)
                            };
                            if code == st::OK {
                                logs.push(format!("[files] файл принят: {fname}"));
                            } else {
                                logs.push(format!("[files] приём не завершён ({fname}): {msg}"));
                            }
                            emit(0, &pack_write_close_resp(tid, code));
                        }
                        None => {
                            emit(0, &pack_write_close_resp(tid, st::BAD_REQ));
                        }
                    }
                }
            }
            kind::WRITE_ABORT => {
                if let Some(tid) = parse_tid(payload, kind::WRITE_ABORT)
                    && let Some(t) = self.writes.remove(&tid)
                {
                    drop(t.file);
                    let _ = std::fs::remove_file(&t.part);
                }
            }
            _ => {
                logs.push(format!("[files] неизвестное сообщение: {k}"));
            }
        }
        logs
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Сессия завершилась — подчищаем незавершённые приёмы (.ratpart).
        for (_, t) in self.writes.drain() {
            drop(t.file);
            let _ = std::fs::remove_file(&t.part);
        }
    }
}

fn do_list(path: &str) -> Result<Vec<DirEntry>, (u8, String)> {
    let rd = std::fs::read_dir(path).map_err(|e| io_status(&e))?;
    let mut entries = Vec::new();
    for e in rd {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        if let Ok(md) = e.metadata() {
            let is_dir = md.is_dir();
            entries.push(DirEntry {
                name,
                is_dir,
                size: if is_dir { 0 } else { md.len() },
                mtime: mtime_secs(md.modified().ok()),
            });
        }
        if entries.len() >= 100_000 {
            break;
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn do_delete(path: &str) -> Result<(), (u8, String)> {
    let md = std::fs::metadata(path).map_err(|e| io_status(&e))?;
    if md.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| io_status(&e))
    } else {
        std::fs::remove_file(path).map_err(|e| io_status(&e))
    }
}

fn finish_write(t: WriteT, mtime: i64) -> Result<(), std::io::Error> {
    t.file.sync_all()?;
    drop(t.file);
    std::fs::rename(&t.part, &t.final_path)?;
    if mtime > 0 {
        use std::time::Duration as D;
        let ts = UNIX_EPOCH + D::from_secs(mtime as u64);
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&t.final_path) {
            let _ = f.set_modified(ts);
        }
    }
    Ok(())
}

// ===================== клиентская сторона (CLI/будущий UI) =====================

use crate::link;
use rat_mux::{Channel, Decoder, Encoder};

const REQ_ID: u32 = 1;
const TID: u32 = 1;

/// Команда инструмента `--files-check`.
#[derive(Debug, Clone)]
pub enum FileCmd {
    Ls(String),
    Drives,
    Get { remote: String, local: String },
    Put { local: String, remote: String },
    Mkdir(String),
    Rm(String),
    Mv { old: String, new: String },
}

/// Разобрать аргументы команды.
pub fn parse_cmd(args: &[String]) -> Result<FileCmd, String> {
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["ls", p] => Ok(FileCmd::Ls((*p).to_string())),
        ["drives"] => Ok(FileCmd::Drives),
        ["get", r, l] => Ok(FileCmd::Get {
            remote: (*r).to_string(),
            local: (*l).to_string(),
        }),
        ["put", l, r] => Ok(FileCmd::Put {
            local: (*l).to_string(),
            remote: (*r).to_string(),
        }),
        ["mkdir", p] => Ok(FileCmd::Mkdir((*p).to_string())),
        ["rm", p] => Ok(FileCmd::Rm((*p).to_string())),
        ["mv", o, n] => Ok(FileCmd::Mv {
            old: (*o).to_string(),
            new: (*n).to_string(),
        }),
        _ => Err(
            "использование: --files-check=АДРЕС ls <каталог> | drives | get <удалённый> <локальный> \
             | put <локальный> <удалённый> | mkdir <каталог> | rm <путь> | mv <откуда> <куда>"
                .to_string(),
        ),
    }
}

pub fn human_size(n: u64) -> String {
    const KB: f64 = 1024.0;
    let x = n as f64;
    if x < KB {
        format!("{n} Б")
    } else if x < KB * KB {
        format!("{:.1} КБ", x / KB).replace('.', ",")
    } else if x < KB * KB * KB {
        format!("{:.1} МБ", x / (KB * KB)).replace('.', ",")
    } else {
        format!("{:.2} ГБ", x / (KB * KB * KB)).replace('.', ",")
    }
}

fn send(link: &link::Duplex, enc: &mut Encoder, payload: &[u8]) -> Result<(), String> {
    enc.push(Channel::Files, 0, payload);
    link.tx
        .send(enc.take_bytes())
        .map_err(|_| "линк закрыт".to_string())
}

fn pump(
    link: &link::Duplex,
    dec: &mut Decoder,
    wait: Duration,
    on_frame: &mut dyn FnMut(Channel, u8, &[u8]),
) {
    match link.rx.recv_timeout(wait) {
        Ok(bytes) => dec.feed(&bytes),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
    }
    loop {
        match dec.poll_frame() {
            Ok(Some(f)) => on_frame(f.channel, f.flags, f.payload),
            Ok(None) => break,
            Err(_) => {
                dec.reset();
                break;
            }
        }
    }
}

/// Выполнить команду через свежее подключение к агенту.
pub fn client_run(addr: &str, cmd: &FileCmd) -> Result<String, String> {
    let (link, closer) =
        link::tcp_connect(addr).map_err(|e| format!("подключение к {addr}: {e}"))?;
    let mut dec = Decoder::new();
    let mut enc = Encoder::new();
    let res = match cmd {
        FileCmd::Ls(path) => cmd_ls(&link, &mut dec, &mut enc, path),
        FileCmd::Drives => cmd_drives(&link, &mut dec, &mut enc),
        FileCmd::Get { remote, local } => cmd_get(&link, &mut dec, &mut enc, remote, local),
        FileCmd::Put { local, remote } => cmd_put(&link, &mut dec, &mut enc, local, remote),
        FileCmd::Mkdir(path) => cmd_ok(
            &link,
            &mut dec,
            &mut enc,
            pack_id_path(kind::MKDIR_REQ, REQ_ID, path),
            &format!("папка создана: {path}"),
        ),
        FileCmd::Rm(path) => cmd_ok(
            &link,
            &mut dec,
            &mut enc,
            pack_id_path(kind::DELETE_REQ, REQ_ID, path),
            &format!("удалено: {path}"),
        ),
        FileCmd::Mv { old, new } => cmd_ok(
            &link,
            &mut dec,
            &mut enc,
            pack_rename_req(REQ_ID, old, new),
            &format!("переименовано: {old} → {new}"),
        ),
    };
    // Дать одиночным кадрам (CANCEL/ABORT) уйти до закрытия сокета.
    std::thread::sleep(Duration::from_millis(250));
    drop(link);
    closer.close();
    res
}

fn cmd_ok(
    link: &link::Duplex,
    dec: &mut Decoder,
    enc: &mut Encoder,
    req: Vec<u8>,
    ok_text: &str,
) -> Result<String, String> {
    send(link, enc, &req)?;
    let mut resp: Option<(u8, String)> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while resp.is_none() && Instant::now() < deadline {
        pump(link, dec, Duration::from_millis(100), &mut |ch, _f, pl| {
            if ch == Channel::Files
                && let Some((id, status, msg)) = parse_ok_resp(pl)
                && id == REQ_ID
            {
                resp = Some((status, msg));
            }
        });
    }
    match resp {
        Some((st::OK, _)) => Ok(ok_text.to_string()),
        Some((_, msg)) => Err(format!("отказ: {msg}")),
        None => Err("нет ответа агента (таймаут)".to_string()),
    }
}

fn cmd_ls(
    link: &link::Duplex,
    dec: &mut Decoder,
    enc: &mut Encoder,
    path: &str,
) -> Result<String, String> {
    send(link, enc, &pack_id_path(kind::LIST_REQ, REQ_ID, path))?;
    let mut resp: Option<(u8, Vec<DirEntry>)> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while resp.is_none() && Instant::now() < deadline {
        pump(link, dec, Duration::from_millis(100), &mut |ch, _f, pl| {
            if ch == Channel::Files
                && let Some((id, status, entries)) = parse_list_resp(pl)
                && id == REQ_ID
            {
                resp = Some((status, entries));
            }
        });
    }
    let Some((status, entries)) = resp else {
        return Err("нет ответа агента (таймаут)".to_string());
    };
    if status != st::OK {
        return Err(format!("каталог недоступен (статус {status})"));
    }
    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let files = entries.len() - dirs;
    let mut out = format!("Каталог {path}: папок {dirs}, файлов {files}\n");
    for e in entries.iter().filter(|e| e.is_dir) {
        out.push_str(&format!("[D] {}\n", e.name));
    }
    for e in entries.iter().filter(|e| !e.is_dir) {
        out.push_str(&format!("[F] {:>10} {}\n", human_size(e.size), e.name));
    }
    Ok(out)
}

fn cmd_drives(link: &link::Duplex, dec: &mut Decoder, enc: &mut Encoder) -> Result<String, String> {
    send(link, enc, &pack_tid(kind::DRIVES_REQ, REQ_ID))?;
    let mut resp: Option<Vec<String>> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while resp.is_none() && Instant::now() < deadline {
        pump(link, dec, Duration::from_millis(100), &mut |ch, _f, pl| {
            if ch == Channel::Files
                && let Some((id, drives)) = parse_drives_resp(pl)
                && id == REQ_ID
            {
                resp = Some(drives);
            }
        });
    }
    match resp {
        Some(d) => Ok(format!("Диски: {}", d.join(" "))),
        None => Err("нет ответа агента (таймаут)".to_string()),
    }
}

fn cmd_get(
    link: &link::Duplex,
    dec: &mut Decoder,
    enc: &mut Encoder,
    remote: &str,
    local: &str,
) -> Result<String, String> {
    send(link, enc, &pack_read_open(TID, remote))?;
    let mut open: Option<(u8, u64, i64)> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while open.is_none() && Instant::now() < deadline {
        pump(link, dec, Duration::from_millis(100), &mut |ch, _f, pl| {
            if ch == Channel::Files
                && let Some((tid, status, size, mtime)) = parse_read_open_resp(pl)
                && tid == TID
            {
                open = Some((status, size, mtime));
            }
        });
    }
    let Some((status, size, mtime)) = open else {
        return Err("нет ответа агента (таймаут)".to_string());
    };
    if status != st::OK {
        return Err(format!("не удалось открыть файл (статус {status})"));
    }
    let mut out = std::fs::File::create(local)
        .map_err(|e| format!("не удалось создать локальный файл: {e}"))?;
    let t0 = Instant::now();
    let mut got: u64 = 0;
    let mut err: Option<String> = None;
    let mut last_progress = Instant::now();
    use std::io::Write as _;
    if size > 0 {
        let chunks_total = size.div_ceil(CHUNK as u64);
        let initial = (chunks_total as u16).min(WINDOW);
        send(link, enc, &pack_credit(TID, initial))?;
        while got < size && err.is_none() {
            if last_progress.elapsed() > Duration::from_secs(60) {
                err = Some("таймаут передачи (60 с без данных)".to_string());
                break;
            }
            pump(link, dec, Duration::from_millis(50), &mut |ch, _f, pl| {
                if ch != Channel::Files {
                    return;
                }
                if let Some((tid, _off, bytes)) = parse_data(pl)
                    && tid == TID
                {
                    if out.write_all(bytes).is_ok() {
                        got += bytes.len() as u64;
                        last_progress = Instant::now();
                        if got < size {
                            let _ = send(link, enc, &pack_credit(TID, 1));
                        }
                    } else if err.is_none() {
                        err = Some("ошибка записи локального файла".to_string());
                        let _ = send(link, enc, &pack_tid(kind::READ_CANCEL, TID));
                    }
                } else if let Some((tid, msg)) = parse_serv_error(pl)
                    && tid == TID
                    && err.is_none()
                {
                    err = Some(msg);
                }
            });
        }
    }
    if let Some(e) = err {
        let _ = send(link, enc, &pack_tid(kind::READ_CANCEL, TID));
        return Err(e);
    }
    let _ = out.flush();
    if mtime > 0 {
        let ts = UNIX_EPOCH + Duration::from_secs(mtime as u64);
        let _ = out.set_modified(ts);
    }
    let secs = t0.elapsed().as_secs_f64().max(0.001);
    let rate = human_size((size as f64 / secs) as u64);
    Ok(format!(
        "Получен {remote} → {local}: {} за {:.2} с ({rate}/с)",
        human_size(size),
        secs
    ))
}

fn cmd_put(
    link: &link::Duplex,
    dec: &mut Decoder,
    enc: &mut Encoder,
    local: &str,
    remote: &str,
) -> Result<String, String> {
    let md = std::fs::metadata(local).map_err(|e| format!("локальный файл: {e}"))?;
    let size = md.len();
    let mtime = mtime_secs(md.modified().ok());
    send(link, enc, &pack_write_open(TID, remote, size, true))?;
    let mut open: Option<u8> = None;
    // Кредиты могут прийти одним пакетом с подтверждением открытия —
    // складываем их отдельно, чтобы не потерять.
    let mut pending: u32 = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while open.is_none() && Instant::now() < deadline {
        pump(link, dec, Duration::from_millis(100), &mut |ch, _f, pl| {
            if ch != Channel::Files {
                return;
            }
            if let Some((tid, status)) = parse_write_open_resp(pl)
                && tid == TID
            {
                open = Some(status);
            } else if let Some((tid, n)) = parse_credit(pl)
                && tid == TID
            {
                pending += n as u32;
            }
        });
    }
    let Some(status) = open else {
        return Err("нет ответа агента (таймаут)".to_string());
    };
    if status != st::OK {
        return Err(format!("не удалось начать загрузку (статус {status})"));
    }
    use std::io::Read as _;
    let mut f = std::fs::File::open(local).map_err(|e| format!("локальный файл: {e}"))?;
    let t0 = Instant::now();
    let mut sent: u64 = 0;
    let mut err: Option<String> = None;
    let mut last_progress = Instant::now();
    while sent < size && err.is_none() {
        // 1) Кредиты, накопленные к этому моменту — по чанку за итерацию.
        if pending > 0 {
            pending -= 1;
            last_progress = Instant::now();
            let want = ((size - sent).min(CHUNK as u64)) as usize;
            let mut buf = vec![0u8; want];
            if f.read_exact(&mut buf).is_err() {
                err = Some("ошибка чтения локального файла".to_string());
                break;
            }
            if let Err(e) = send(link, enc, &pack_data(TID, sent, &buf)) {
                err = Some(e);
                break;
            }
            sent += want as u64;
            continue;
        }
        // 2) Новые кредиты от сервера.
        if last_progress.elapsed() > Duration::from_secs(60) {
            err = Some("таймаут загрузки (60 с без кредитов)".to_string());
            break;
        }
        pump(link, dec, Duration::from_millis(50), &mut |ch, _f, pl| {
            if ch != Channel::Files {
                return;
            }
            if let Some((tid, n)) = parse_credit(pl)
                && tid == TID
            {
                pending += n as u32;
            } else if let Some((tid, msg)) = parse_serv_error(pl)
                && tid == TID
                && err.is_none()
            {
                err = Some(msg);
            }
        });
    }
    if let Some(e) = err {
        let _ = send(link, enc, &pack_tid(kind::WRITE_ABORT, TID));
        return Err(e);
    }
    send(link, enc, &pack_write_close(TID, mtime))?;
    let mut close: Option<u8> = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while close.is_none() && Instant::now() < deadline {
        pump(link, dec, Duration::from_millis(100), &mut |ch, _f, pl| {
            if ch == Channel::Files
                && let Some((tid, status)) = parse_write_close_resp(pl)
                && tid == TID
            {
                close = Some(status);
            }
        });
    }
    match close {
        Some(st::OK) => {
            let secs = t0.elapsed().as_secs_f64().max(0.001);
            let rate = human_size((size as f64 / secs) as u64);
            Ok(format!(
                "Загружен {local} → {remote}: {} за {:.2} с ({rate}/с)",
                human_size(size),
                secs
            ))
        }
        Some(code) => Err(format!("не завершено (статус {code})")),
        None => Err("нет подтверждения завершения (таймаут)".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_resp_roundtrip() {
        let entries = vec![
            DirEntry {
                name: "Папка".into(),
                is_dir: true,
                size: 0,
                mtime: 100,
            },
            DirEntry {
                name: "файл.txt".into(),
                is_dir: false,
                size: 12345,
                mtime: 200,
            },
        ];
        let p = pack_list_resp(7, st::OK, &entries);
        let (id, status, got) = parse_list_resp(&p).unwrap();
        assert_eq!((id, status), (7, st::OK));
        assert_eq!(got, entries);
    }

    #[test]
    fn ok_resp_roundtrip() {
        let p = pack_ok_resp(3, st::DENIED, "нет доступа");
        let (id, status, msg) = parse_ok_resp(&p).unwrap();
        assert_eq!(id, 3);
        assert_eq!(status, st::DENIED);
        assert_eq!(msg, "нет доступа");
    }

    #[test]
    fn rename_roundtrip() {
        let p = pack_rename_req(9, "C:\\a.txt", "C:\\b.txt");
        let (id, old, new) = parse_rename_req(&p).unwrap();
        assert_eq!(
            (id, old.as_str(), new.as_str()),
            (9, "C:\\a.txt", "C:\\b.txt")
        );
    }

    #[test]
    fn read_open_and_data_roundtrip() {
        let p = pack_read_open(5, "C:\\file.bin");
        let (tid, path) = parse_id_path(&p, kind::READ_OPEN).unwrap();
        assert_eq!((tid, path.as_str()), (5, "C:\\file.bin"));

        let resp = pack_read_open_resp(5, st::OK, 1000, 4242);
        let (tid, status, size, mt) = parse_read_open_resp(&resp).unwrap();
        assert_eq!((tid, status, size, mt), (5, st::OK, 1000, 4242));

        let d = pack_data(5, 4096, &[1, 2, 3, 4]);
        let (tid, off, bytes) = parse_data(&d).unwrap();
        assert_eq!((tid, off, bytes), (5, 4096, &[1u8, 2, 3, 4][..]));

        let c = pack_credit(5, 16);
        assert_eq!(parse_credit(&c).unwrap(), (5, 16));
    }

    #[test]
    fn write_flow_roundtrip() {
        let p = pack_write_open(2, "C:\\t.bin", 999, true);
        let (tid, path, size, ow) = parse_write_open(&p).unwrap();
        assert_eq!((tid, path.as_str(), size, ow), (2, "C:\\t.bin", 999, true));

        let r = pack_write_open_resp(2, st::EXISTS);
        assert_eq!(parse_write_open_resp(&r).unwrap(), (2, st::EXISTS));

        let c = pack_write_close(2, 777);
        let (tid, mt) = parse_write_close(&c).unwrap();
        assert_eq!((tid, mt), (2, 777));

        let cr = pack_write_close_resp(2, st::OK);
        assert_eq!(parse_write_close_resp(&cr).unwrap(), (2, st::OK));

        let e = pack_serv_error(2, "boom");
        assert_eq!(parse_serv_error(&e).unwrap(), (2, "boom".to_string()));

        assert_eq!(
            parse_tid(&pack_tid(kind::WRITE_ABORT, 2), kind::WRITE_ABORT),
            Some(2)
        );
        assert_eq!(
            parse_tid(&pack_tid(kind::READ_CANCEL, 1), kind::READ_CANCEL),
            Some(1)
        );
    }

    #[test]
    fn drives_roundtrip() {
        let p = pack_drives_resp(1, &["C:\\".to_string(), "D:\\".to_string()]);
        let (id, d) = parse_drives_resp(&p).unwrap();
        assert_eq!(id, 1);
        assert_eq!(d, vec!["C:\\".to_string(), "D:\\".to_string()]);
    }

    #[test]
    fn fsinfo_roundtrip() {
        let p = pack_fsinfo_resp(3, st::OK, 12_345, 67_890);
        assert_eq!(parse_fsinfo_resp(&p), Some((3, st::OK, 12_345, 67_890)));
        assert_eq!(parse_fsinfo_resp(&[kind::FSINFO_RESP]), None);
    }
}
