//! `rat-agent-helper` — session helper: захват экрана (DXGI), инъекция ввода
//! (SendInput), clipboard, ConPTY; запускается службой в пользовательской сессии
//! (WTSQueryUserToken + CreateProcessAsUserW) или как SYSTEM-процесс для pre-login
//! задач (SAS, экран входа). См. docs/ARCHITECTURE_DESIGN_v1.1.md §3.2, §5.1–5.2.
//!
//! P0: заглушка.

fn main() {
    println!("rat-agent-helper (placeholder) — see docs/ARCHITECTURE_DESIGN_v1.1.md §3");
}
