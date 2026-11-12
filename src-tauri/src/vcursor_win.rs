//! Режим «два курсора» для Windows.
//!
//! Когда режим включён, системная мышь «замораживается» (хук WH_MOUSE_LL съедает все
//! события мыши), а движения и клики читаются через Raw Input и отправляются
//! в интерфейс, где ими управляется красный курсор внутри окна-подсказки.

use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::OnceLock;

use serde::Serialize;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow,
    GetMessageW, RegisterClassW, SetForegroundWindow, SetTimer, SetWindowsHookExW,
    TranslateMessage, HWND_MESSAGE, MSG, WH_MOUSE_LL, WM_INPUT, WM_TIMER, WNDCLASSW,
};

/// Строка в UTF-16 с завершающим нулём.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(Serialize, Clone, Debug)]
pub struct VEvent {
    /// "move" | "down" | "up" | "wheel"
    pub kind: &'static str,
    pub dx: i32,
    pub dy: i32,
    /// 0 — левая, 2 — правая
    pub button: u8,
    pub delta: i32,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static DX: AtomicI32 = AtomicI32::new(0);
static DY: AtomicI32 = AtomicI32::new(0);
static SAVED_FG: AtomicIsize = AtomicIsize::new(0);
static EMIT: OnceLock<Box<dyn Fn(VEvent) + Send + Sync>> = OnceLock::new();

fn emit(ev: VEvent) {
    if let Some(f) = EMIT.get() {
        f(ev);
    }
}

/// Запускает фоновый поток с хуком и приёмом Raw Input. Вызывать один раз.
pub fn init(emit_fn: Box<dyn Fn(VEvent) + Send + Sync>) {
    if EMIT.set(emit_fn).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe { run_message_loop() });
}

/// Включает или выключает режим. `own_hwnd` — окно приложения (чтобы не «возвращать» фокус самому себе).
pub fn set_active(active: bool, own_hwnd: isize) {
    if active {
        let fg = unsafe { GetForegroundWindow() } as isize;
        SAVED_FG.store(if fg == own_hwnd { 0 } else { fg }, Ordering::SeqCst);
        DX.store(0, Ordering::SeqCst);
        DY.store(0, Ordering::SeqCst);
        ACTIVE.store(true, Ordering::SeqCst);
    } else {
        ACTIVE.store(false, Ordering::SeqCst);
        let fg = SAVED_FG.swap(0, Ordering::SeqCst);
        if fg != 0 {
            // Возвращаем фокус программе, из которой пришли (Zoom, презентация и т.д.).
            unsafe {
                SetForegroundWindow(fg as HWND);
            }
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && ACTIVE.load(Ordering::Relaxed) {
        // Съедаем событие: системный курсор остаётся на месте, клики не уходят в другие окна.
        return 1;
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

fn flush_move() {
    let dx = DX.swap(0, Ordering::Relaxed);
    let dy = DY.swap(0, Ordering::Relaxed);
    if (dx != 0 || dy != 0) && ACTIVE.load(Ordering::Relaxed) {
        emit(VEvent { kind: "move", dx, dy, button: 0, delta: 0 });
    }
}

unsafe fn handle_raw_input(lparam: LPARAM) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let handle = lparam as HRAWINPUT;
    let header_size = size_of::<RAWINPUTHEADER>() as u32;

    let mut size = 0u32;
    unsafe { GetRawInputData(handle, RID_INPUT, null_mut(), &mut size, header_size) };
    if size == 0 {
        return;
    }
    let mut buf = vec![0u8; size as usize];
    let got = unsafe {
        GetRawInputData(handle, RID_INPUT, buf.as_mut_ptr() as *mut _, &mut size, header_size)
    };
    if got == u32::MAX || (got as usize) < size_of::<RAWINPUTHEADER>() {
        return;
    }
    let raw: RAWINPUT = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const RAWINPUT) };
    if raw.header.dwType != RIM_TYPEMOUSE {
        return;
    }
    let m = unsafe { raw.data.mouse };
    // 0x01 = MOUSE_MOVE_ABSOLUTE (планшеты, удалённый рабочий стол): такие события пропускаем.
    if m.usFlags & 0x01 == 0 {
        DX.fetch_add(m.lLastX, Ordering::Relaxed);
        DY.fetch_add(m.lLastY, Ordering::Relaxed);
    }

    let (flags, data) = unsafe { (m.Anonymous.Anonymous.usButtonFlags, m.Anonymous.Anonymous.usButtonData) };
    let button_events: [(u16, &'static str, u8); 4] = [
        (0x0001, "down", 0),
        (0x0002, "up", 0),
        (0x0004, "down", 2),
        (0x0008, "up", 2),
    ];
    for (mask, kind, button) in button_events {
        if flags & mask != 0 {
            flush_move();
            emit(VEvent { kind, dx: 0, dy: 0, button, delta: 0 });
        }
    }
    if flags & 0x0400 != 0 {
        // RI_MOUSE_WHEEL: знаковое значение, 120 на «щелчок»
        emit(VEvent { kind: "wheel", dx: 0, dy: 0, button: 0, delta: data as i16 as i32 });
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_INPUT => {
            unsafe { handle_raw_input(lparam) };
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_TIMER => {
            flush_move();
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn run_message_loop() {
    let hinst = unsafe { GetModuleHandleW(null()) };
    let class_name = wide("SbsVirtualCursor");

    let mut class: WNDCLASSW = unsafe { std::mem::zeroed() };
    class.lpfnWndProc = Some(wnd_proc);
    class.hInstance = hinst;
    class.lpszClassName = class_name.as_ptr();
    unsafe { RegisterClassW(&class) };

    let title = wide("");
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            hinst,
            null(),
        )
    };
    if hwnd.is_null() {
        return;
    }

    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // Generic Desktop
        usUsage: 0x02,     // Mouse
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    unsafe { RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) };

    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), hinst, 0) };

    // ~60 раз в секунду отправляем накопленное движение
    unsafe { SetTimer(hwnd, 1, 16, None) };

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
