//! Rat Next — сессионный слой: линк (транспорт), управляющий канал, кадры видео,
//! метрики, захват экрана и пилотный кодек-путь (feature `ffmpeg`).
//!
//! Этот крейт — «продуктовая» версия контура, отработанного в спайке
//! `spikes/session_loopback` (P2). Агент и вьюер — тонкие обёртки в `bins/`.

pub mod control;
pub mod elevation;
pub mod files;
pub mod frame;
pub mod input;
pub mod inputcheck;
pub mod link;
pub mod log;
pub mod single_instance;
pub mod stats;

#[cfg(windows)]
pub mod capture;

#[cfg(windows)]
pub mod secure;

#[cfg(all(windows, feature = "ffmpeg"))]
pub mod agent;
#[cfg(all(windows, feature = "ffmpeg"))]
pub mod ffmpeg;
#[cfg(all(windows, feature = "ffmpeg"))]
pub mod viewer;
