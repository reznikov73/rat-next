//! Защищённый рабочий стол Windows («Winlogon»): экран входа, окно UAC и
//! экран блокировки.
//!
//! Обычные процессы живут на рабочем столе «Default» и защищённый стол не
//! видят. Хелпер службы работает от SYSTEM — он может определить активный
//! стол, снять его экран (GDI BitBlt) и ввести туда события (SendInput),
//! если поток переключён на этот стол через `SetThreadDesktop`.

use windows::Win32::Foundation::{BOOL, HANDLE, HWND};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DESKTOPHORZRES, DESKTOPVERTRES,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetDeviceCaps, HGDIOBJ, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_ACCESS_FLAGS, DESKTOP_CONTROL_FLAGS, GetUserObjectInformationW, HDESK,
    OpenDesktopW, OpenInputDesktop, SetThreadDesktop, UOI_NAME,
};
use windows::core::w;

/// Права на чтение имени стола (диагностика активного стола).
/// DESKTOP_READOBJECTS | DESKTOP_SWITCHDESKTOP.
fn desk_read() -> u32 {
    0x0001 | 0x0100
}

/// Права для привязки потока к столу и инъекции ввода.
/// DESKTOP_READOBJECTS | DESKTOP_SWITCHDESKTOP | DESKTOP_JOURNALPLAYBACK.
///
/// Без DESKTOP_JOURNALPLAYBACK `SendInput` на защищённом столе МОЛЧА
/// игнорируется: вызов возвращает успех, но события не применяются
/// (подтверждено практикой: блог A. Campos Magencio, retired MSFT —
/// «SendInput doesn't work but it doesn't return an error»).
fn desk_attach() -> u32 {
    0x0001 | 0x0100 | 0x0020
}

fn null_hwnd() -> HWND {
    HWND(std::ptr::null_mut())
}

/// Имя активного (входного) рабочего стола текущей сессии.
pub fn active_desktop_name() -> Option<String> {
    unsafe {
        let desk = OpenInputDesktop(
            DESKTOP_CONTROL_FLAGS(0),
            BOOL(0),
            DESKTOP_ACCESS_FLAGS(desk_read()),
        )
        .ok()?;
        let name = desk_name(desk);
        let _ = CloseDesktop(desk);
        name
    }
}

/// Активен ли защищённый стол (имя не «Default»).
pub fn active_is_secure() -> bool {
    matches!(active_desktop_name().as_deref(), Some(n) if n != "Default")
}

fn desk_name(desk: HDESK) -> Option<String> {
    let mut buf = [0u16; 128];
    let mut needed = 0u32;
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desk.0),
            UOI_NAME,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            (buf.len() * 2) as u32,
            Some(&mut needed),
        )
        .ok()?;
    }
    let len = (needed as usize / 2).saturating_sub(1).min(buf.len());
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// Поток, переключённый на защищённый стол; при уничтожении возвращает его
/// на «Default». Нужен потокам захвата и ввода.
pub struct DesktopAttach {
    desk: HDESK,
}

// SAFETY: дескриптор стола используется только тем потоком, который его
// открыл и переключился на стол; Send нужен, чтобы передать владение в этот
// поток при создании (как у `Capture`).
unsafe impl Send for DesktopAttach {}

impl DesktopAttach {
    /// Открыть стол «Winlogon» и переключить на него текущий поток.
    pub fn attach_winlogon() -> Result<Self, String> {
        unsafe {
            let desk = OpenDesktopW(
                w!("Winlogon"),
                DESKTOP_CONTROL_FLAGS(0),
                BOOL(0),
                desk_attach(),
            )
            .map_err(|e| format!("OpenDesktop(Winlogon): {e}"))?;
            if let Err(e) = SetThreadDesktop(desk) {
                let _ = CloseDesktop(desk);
                return Err(format!("SetThreadDesktop(Winlogon): {e}"));
            }
            Ok(Self { desk })
        }
    }

    /// Привязать текущий поток к активному входному столу сессии (для ввода).
    /// События SendInput должны уходить на активный стол; на защищённом
    /// столе это «Winlogon», но спрашиваем у системы текущий, чтобы не
    /// промахнуться в момент перехода.
    pub fn attach_input() -> Result<Self, String> {
        unsafe {
            let desk = OpenInputDesktop(
                DESKTOP_CONTROL_FLAGS(0),
                BOOL(0),
                DESKTOP_ACCESS_FLAGS(desk_attach()),
            )
            .map_err(|e| format!("OpenInputDesktop: {e}"))?;
            if let Err(e) = SetThreadDesktop(desk) {
                let _ = CloseDesktop(desk);
                return Err(format!("SetThreadDesktop(входной стол): {e}"));
            }
            Ok(Self { desk })
        }
    }
}

impl Drop for DesktopAttach {
    fn drop(&mut self) {
        unsafe {
            // Возвращаем поток на «Default». Пробуем разные наборы прав,
            // чтобы при любом раскладе не оставить поток на защищённом столе.
            let back = OpenDesktopW(
                w!("Default"),
                DESKTOP_CONTROL_FLAGS(0),
                BOOL(0),
                desk_attach(),
            )
            .or_else(|_| {
                OpenDesktopW(
                    w!("Default"),
                    DESKTOP_CONTROL_FLAGS(0),
                    BOOL(0),
                    desk_read(),
                )
            })
            .or_else(|_| {
                OpenInputDesktop(
                    DESKTOP_CONTROL_FLAGS(0),
                    BOOL(0),
                    DESKTOP_ACCESS_FLAGS(desk_read()),
                )
            });
            if let Ok(def) = back {
                let _ = SetThreadDesktop(def);
                let _ = CloseDesktop(def);
            }
            let _ = CloseDesktop(self.desk);
        }
    }
}

/// Захват защищённого стола через GDI (снимок экрана BitBlt).
pub struct SecureCapture {
    _attach: DesktopAttach,
    w: u32,
    h: u32,
}

// SAFETY: см. `DesktopAttach` — объект живёт и используется в одном потоке.
unsafe impl Send for SecureCapture {}

impl SecureCapture {
    /// Открыть защищённый стол и переключить на него текущий поток.
    pub fn attach() -> Result<Self, String> {
        let attach = DesktopAttach::attach_winlogon()?;
        let (w, h) = unsafe {
            let hdc = GetDC(null_hwnd());
            let w = GetDeviceCaps(hdc, DESKTOPHORZRES).max(0) as u32;
            let h = GetDeviceCaps(hdc, DESKTOPVERTRES).max(0) as u32;
            let _ = ReleaseDC(null_hwnd(), hdc);
            (w, h)
        };
        if w == 0 || h == 0 {
            return Err("не удалось определить размер защищённого стола".to_string());
        }
        Ok(Self {
            _attach: attach,
            w,
            h,
        })
    }

    /// Размер защищённого стола (физические пиксели).
    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    /// Снять кадр (BGRA сверху вниз) и отдать в замыкание.
    pub fn capture(&mut self, mut f: impl FnMut(&[u8], usize)) -> Result<bool, String> {
        unsafe {
            let screen = GetDC(null_hwnd());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, self.w as i32, self.h as i32);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let blit_ok = BitBlt(
                mem,
                0,
                0,
                self.w as i32,
                self.h as i32,
                screen,
                0,
                0,
                SRCCOPY,
            )
            .is_ok();

            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize =
                std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = self.w as i32;
            bi.bmiHeader.biHeight = -(self.h as i32);
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;

            let mut buf = vec![0u8; (self.w * self.h * 4) as usize];
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                self.h,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                &mut bi,
                DIB_RGB_COLORS,
            );

            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
            let _ = ReleaseDC(null_hwnd(), screen);

            if blit_ok && lines == self.h as i32 {
                f(&buf, (self.w as usize) * 4);
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}
