//! Режим «два курсора» для Windows.
//!
//! При включении системный курсор «по-человечески» (по кривой, рывками) уезжает в случайную точку
//! площадки у центра экрана и замораживается:
//! хук WH_MOUSE_LL съедает все события мыши. Сдвиг мыши считается как отклонение
//! от центра (после каждого движения курсор возвращается в центр), клики и колесо
//! уходят в интерфейс, где ими управляется красный курсор внутри окна-подсказки.

use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

static RNG: AtomicU64 = AtomicU64::new(0);

/// Случайное число от 0 до 1 (xorshift, хватает для разброса курсора).
fn rand01() -> f64 {
    let mut x = RNG.load(Ordering::Relaxed);
    if x == 0 {
        x = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(88172645463325252)
            | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    RNG.store(x, Ordering::Relaxed);
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// Площадка у центра монитора, где останавливается курсор: ±15% ширины и высоты от центра.
const PLATE: f64 = 0.15;

/// Случайная точка на площадке у центра монитора, на котором сейчас курсор.
fn random_center(from: POINT) -> (i32, i32) {
    unsafe {
        let mon = MonitorFromPoint(from, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(mon, &mut info) != 0 {
            let r = info.rcMonitor;
            let (w, h) = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
            let cx = (r.left + r.right) as f64 / 2.0;
            let cy = (r.top + r.bottom) as f64 / 2.0;
            let x = cx + (rand01() * 2.0 - 1.0) * w * PLATE;
            let y = cy + (rand01() * 2.0 - 1.0) * h * PLATE;
            return (x.round() as i32, y.round() as i32);
        }
    }
    (from.x, from.y)
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Ключевые точки (время, прогресс) «живого» движения: несколько рывков с замедлениями,
/// иногда почти остановка посередине, как у человека.
fn human_keys() -> Vec<(f64, f64)> {
    let n = 4 + (rand01() * 4.0) as usize; // 4..7 рывков
    let mut weights: Vec<f64> = (0..n).map(|_| 0.2 + rand01() * 1.6).collect();
    if n > 3 && rand01() < 0.6 {
        let i = 1 + (rand01() * (n - 2) as f64) as usize;
        weights[i] *= 0.12; // почти остановка
    }
    let total: f64 = weights.iter().sum();
    let mut keys = vec![(0.0, 0.0)];
    let mut acc = 0.0;
    for (i, w) in weights.iter().enumerate() {
        acc += w / total;
        let t = if i + 1 == n {
            1.0
        } else {
            ((i + 1) as f64 + (rand01() - 0.5) * 0.5) / n as f64
        };
        keys.push((t, if i + 1 == n { 1.0 } else { acc }));
    }
    keys
}

fn progress_at(keys: &[(f64, f64)], t: f64) -> f64 {
    for w in keys.windows(2) {
        let (t0, p0) = w[0];
        let (t1, p1) = w[1];
        if t <= t1 {
            let local = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 1.0 };
            return p0 + (p1 - p0) * smooth(local);
        }
    }
    1.0
}

fn bezier(p: [(f64, f64); 4], t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0,
        a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1,
    )
}

/// Двигает системный курсор в случайную точку площадки у центра монитора «как человек»:
/// по изогнутой траектории, рывками, с лёгкой дрожью и небольшим перелётом, и только после этого
/// «включает» отсчёт сдвигов мыши.
fn glide_to_center(epoch: i32) {
    let mut start = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut start) };
    let (tx, ty) = random_center(start);
    CX.store(tx, Ordering::SeqCst);
    CY.store(ty, Ordering::SeqCst);

    let (sx, sy) = (start.x as f64, start.y as f64);
    let (ex, ey) = (tx as f64, ty as f64);
    let (dx, dy) = (ex - sx, ey - sy);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (nx, ny) = (-dy / len, dx / len); // перпендикуляр к прямой
    let bend = |k: f64| (rand01() - 0.5) * 0.5 * len * k;
    let (o1, o2) = (bend(1.0), bend(1.0));
    let pts = [
        (sx, sy),
        (sx + dx * 0.3 + nx * o1, sy + dy * 0.3 + ny * o1),
        (sx + dx * 0.7 + nx * o2, sy + dy * 0.7 + ny * o2),
        (ex, ey),
    ];
    let keys = human_keys();
    let dur = (0.6 + len / 2200.0 + rand01() * 0.5).min(1.8);

    let began = Instant::now();
    loop {
        if EPOCH.load(Ordering::SeqCst) != epoch || !ACTIVE.load(Ordering::SeqCst) {
            return;
        }
        let t = began.elapsed().as_secs_f64() / dur;
        if t >= 1.0 {
            break;
        }
        let (x, y) = bezier(pts, progress_at(&keys, t));
        let tremor = 0.9 * (1.0 - t); // дрожь руки стихает к концу
        let x = x + (rand01() - 0.5) * 2.0 * tremor;
        let y = y + (rand01() - 0.5) * 2.0 * tremor;
        unsafe { SetCursorPos(x.round() as i32, y.round() as i32) };
        std::thread::sleep(Duration::from_millis(8));
    }

    // небольшой перелёт и возврат в точку
    let over = (tx + ((rand01() - 0.5) * 12.0) as i32, ty + ((rand01() - 0.5) * 12.0) as i32);
    unsafe { SetCursorPos(over.0, over.1) };
    std::thread::sleep(Duration::from_millis(60 + (rand01() * 60.0) as u64));
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
