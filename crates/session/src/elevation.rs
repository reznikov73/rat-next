//! Проверка, запущен ли процесс с повышенными правами (UAC / elevated-токен).
//!
//! Нужна интерфейсу: без прав администратора Windows (UIPI) не пропускает
//! ввод в окна приложений, запущенных от администратора, — об этом стоит
//! предупредить пользователя.

/// `true`, если текущий процесс работает с повышенными правами.
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        #[repr(C)]
        struct TokenElevation {
            token_is_elevated: u32,
        }
        const TOKEN_QUERY: u32 = 0x0008;
        const TOKEN_ELEVATION: u32 = 20;
        unsafe {
            let mut token: *mut c_void = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TokenElevation {
                token_is_elevated: 0,
            };
            let mut returned = 0u32;
            let ok = GetTokenInformation(
                token,
                TOKEN_ELEVATION,
                &mut elevation as *mut TokenElevation as *mut c_void,
                std::mem::size_of::<TokenElevation>() as u32,
                &mut returned,
            );
            CloseHandle(token);
            ok != 0 && elevation.token_is_elevated != 0
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(
        process: *mut std::ffi::c_void,
        desired_access: u32,
        token: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn GetTokenInformation(
        token: *mut std::ffi::c_void,
        class: u32,
        info: *mut std::ffi::c_void,
        info_len: u32,
        returned_len: *mut u32,
    ) -> i32;
}
