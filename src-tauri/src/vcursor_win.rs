//! Режим «два курсора» для Windows.
//!
//! При включении системный курсор плавно уезжает в центр экрана и замораживается:
//! хук WH_MOUSE_LL съедает все события мыши. Сдвиг мыши считается как отклонение
//! от центра (после каждого движения курсор возвращается в центр), клики и колесо
//! уходят в интерфейс, где ими управляется красный курсор внутри окна-подсказки.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    SetCursorPos, SetForegroundWindow, SetWindowsHookExW, TranslateMessage, MSG,
    MSLLHOOKSTRUCT, WH_MOUSE_LL,
};

const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MOUSEWHEEL: u32 = 0x020A;

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

/// Режим включён: мышь перехватывается.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Курсор доехал до центра, можно считать сдвиги.
static READY: AtomicBool = AtomicBool::new(false);
/// Номер включения: старая анимация прекращается, если режим переключили заново.
static EPOCH: AtomicI32 = AtomicI32::new(0);
static CX: AtomicI32 = AtomicI32::new(0);
static CY: AtomicI32 = AtomicI32::new(0);
static DX: AtomicI32 = AtomicI32::new(0);
static DY: AtomicI32 = AtomicI32::new(0);
static SAVED_FG: AtomicIsize = AtomicIsize::new(0);
static EMIT: OnceLock<Box<dyn Fn(VEvent) + Send + Sync>> = OnceLock::new();

fn emit(ev: VEvent) {
    if let Some(f) = EMIT.get() {
        f(ev);
    }
}

/// Запускает фоновые потоки: хук мыши и отправку накопленного движения. Вызывать один раз.
pub fn init(emit_fn: Box<dyn Fn(VEvent) + Send + Sync>) {
    if EMIT.set(emit_fn).is_err() {
        return;
    }
    std::thread::spawn(|| unsafe { run_hook_loop() });
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(16));
        flush_move();
    });
}

/// Центр монитора, на котором сейчас находится курсор.
fn monitor_center(from: POINT) -> (i32, i32) {
    unsafe {
        let mon = MonitorFromPoint(from, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut info) != 0 {
            let r = info.rcMonitor;
            return ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        }
    }
    (from.x, from.y)
}

/// Плавно двигает системный курсор в центр монитора и после этого «включает» отсчёт сдвигов.
fn glide_to_center(epoch: i32) {
    let mut start = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut start) };
    let (tx, ty) = monitor_center(start);
    CX.store(tx, Ordering::SeqCst);
    CY.store(ty, Ordering::SeqCst);

    const STEPS: i32 = 28;
    for i in 1..=STEPS {
        if EPOCH.load(Ordering::SeqCst) != epoch || !ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        let t = i as f64 / STEPS as f64;
        let e = 1.0 - (1.0 - t).powi(3); // плавное замедление в конце
        let x = start.x as f64 + (tx - start.x) as f64 * e;
        let y = start.y as f64 + (ty - start.y) as f64 * e;
        unsafe { SetCursorPos(x.round() as i32, y.round() as i32) };
        std::thread::sleep(Duration::from_millis(14));
    }
    unsafe { SetCursorPos(tx, ty) };
    if EPOCH.load(Ordering::SeqCst) == epoch && ACTIVE.load(Ordering::SeqCst) {
        DX.store(0, Ordering::SeqCst);
        DY.store(0, Ordering::SeqCst);
        READY.store(true, Ordering::SeqCst);
    }
}

/// Включает или выключает режим. `own_hwnd` — окно приложения (чтобы не «возвращать» фокус самому себе).
pub fn set_active(active: bool, own_hwnd: isize) {
    if active {
        if ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        let fg = unsafe { GetForegroundWindow() } as isize;
        SAVED_FG.store(if fg == own_hwnd { 0 } else { fg }, Ordering::SeqCst);
        READY.store(false, Ordering::SeqCst);
        let epoch = EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
        ACTIVE.store(true, Ordering::SeqCst);
        std::thread::spawn(move || glide_to_center(epoch));
    } else {
        ACTIVE.store(false, Ordering::SeqCst);
        READY.store(false, Ordering::SeqCst);
        EPOCH.fetch_add(1, Ordering::SeqCst);
        DX.store(0, Ordering::SeqCst);
        DY.store(0, Ordering::SeqCst);
        let fg = SAVED_FG.swap(0, Ordering::SeqCst);
        if fg != 0 {
            // Возвращаем фокус программе, из которой пришли (Zoom, презентация и т.д.).
            unsafe { SetForegroundWindow(fg as HWND) };
        }
    }
}

fn flush_move() {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let dx = DX.swap(0, Ordering::Relaxed);
    let dy = DY.swap(0, Ordering::Relaxed);
    if dx != 0 || dy != 0 {
        emit(VEvent { kind: "move", dx, dy, button: 0, delta: 0 });
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 || !ACTIVE.load(Ordering::Relaxed) {
        return unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) };
    }
    let info = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
    let ready = READY.load(Ordering::Relaxed);

    match wparam as u32 {
        WM_MOUSEMOVE => {
            if ready {
                let cx = CX.load(Ordering::Relaxed);
                let cy = CY.load(Ordering::Relaxed);
                DX.fetch_add(info.pt.x - cx, Ordering::Relaxed);
                DY.fetch_add(info.pt.y - cy, Ordering::Relaxed);
                // Возвращаем курсор в центр: так сдвиг всегда считается от одной точки.
                unsafe { SetCursorPos(cx, cy) };
            }
        }
        WM_LBUTTONDOWN if ready => {
            flush_move();
            emit(VEvent { kind: "down", dx: 0, dy: 0, button: 0, delta: 0 });
        }
        WM_LBUTTONUP if ready => {
            flush_move();
            emit(VEvent { kind: "up", dx: 0, dy: 0, button: 0, delta: 0 });
        }
        WM_RBUTTONDOWN if ready => {
            emit(VEvent { kind: "down", dx: 0, dy: 0, button: 2, delta: 0 });
        }
        WM_RBUTTONUP if ready => {
            emit(VEvent { kind: "up", dx: 0, dy: 0, button: 2, delta: 0 });
        }
        WM_MOUSEWHEEL if ready => {
            let delta = (info.mouseData >> 16) as u16 as i16 as i32;
            emit(VEvent { kind: "wheel", dx: 0, dy: 0, button: 0, delta });
        }
        _ => {}
    }
    // Съедаем событие: системный курсор остаётся в центре, клики не уходят в другие окна.
    1
}

unsafe fn run_hook_loop() {
    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(hook_proc), null_mut(), 0) };
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
