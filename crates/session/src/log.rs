//! Простейший журнал сессионного слоя: bin-обёртка задаёт обработчик
//! (пишет в stdout и rat.log), потоки агента/вьюера шлют строки сюда.

use std::sync::OnceLock;

static LOGGER: OnceLock<fn(&str)> = OnceLock::new();

/// Задать обработчик журнала (вызывается один раз при старте приложения).
pub fn set_logger(f: fn(&str)) {
    let _ = LOGGER.set(f);
}

/// Отправить строку в журнал (если обработчик задан).
pub fn log_line(s: &str) {
    if let Some(f) = LOGGER.get() {
        f(s);
    }
}
