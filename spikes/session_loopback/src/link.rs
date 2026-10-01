//! In-memory duplex «сеть» для loopback-спайка.
//! В продукте здесь будет QUIC-транспорт; mux транспорт-агностичен.

pub struct Duplex {
    pub tx: std::sync::mpsc::Sender<Vec<u8>>,
    pub rx: std::sync::mpsc::Receiver<Vec<u8>>,
}

/// Создать пару связанных концов duplex.
pub fn duplex_pair() -> (Duplex, Duplex) {
    let (a_tx, b_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (b_tx, a_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    (
        Duplex { tx: a_tx, rx: a_rx },
        Duplex { tx: b_tx, rx: b_rx },
    )
}
