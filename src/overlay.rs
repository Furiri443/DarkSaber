//! Overlay tương thích Wine + Windows thật.
//!
//! KHÔNG hook đồ hoạ game (không Present/SwapBuffers). Thay vào đó tạo một cửa sổ
//! layered riêng: trong suốt một phần, topmost, click-through (WS_EX_TRANSPARENT),
//! không nhận focus (WS_EX_NOACTIVATE). Vẽ menu bằng GDI; điều khiển bằng phím đọc
//! qua GetAsyncKeyState (không cần focus). Nhờ vậy chạy được dù game là
//! DX11/DX12/Vulkan/OpenGL và cả dưới Wine.
//!
//! Điều khiển:  `\` mở/đóng menu · ↑/↓ chọn dòng · ←/→ chỉnh giá trị / bật-tắt ·
//!              Enter kích hoạt power.

use core::cell::RefCell;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, TextOutW, HFONT, PAINTSTRUCT, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow,
    TranslateMessage, HWND_TOPMOST, LWA_ALPHA, MSG, SWP_NOMOVE, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
};

use crate::state;

// Phím điều khiển.
const VK_TOGGLE: i32 = 0xDC; // `\`
const VK_UP: i32 = 0x26;
const VK_DOWN: i32 = 0x28;
const VK_LEFT: i32 = 0x25;
const VK_RIGHT: i32 = 0x27;
const VK_ENTER: i32 = 0x0D;

const ITEMS: usize = 22;
const MAIN_W: i32 = 360;
const BUFF_W: i32 = 360;
const PANEL_GAP: i32 = 16;
const WIN_W: i32 = MAIN_W + PANEL_GAP + BUFF_W;
const WIN_H: i32 = 545;
const LINE_H: i32 = 22;
const TOP: i32 = 64;
const LEFT: i32 = 14;

const COL_BG: COLORREF = rgb(18, 20, 26);
const COL_SEL: COLORREF = rgb(40, 90, 60);
const COL_TEXT: COLORREF = rgb(220, 225, 230);
const COL_HEAD: COLORREF = rgb(90, 255, 140);
const COL_DIM: COLORREF = rgb(140, 145, 155);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

struct Ui {
    sel: i32,
    buff_modal: bool,
    buff_sel: i32,
    buff_mask: usize,
    p_toggle: bool,
    p_up: bool,
    p_down: bool,
    p_left: bool,
    p_right: bool,
    p_enter: bool,
    logged_first: bool,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui {
        sel: 0,
        buff_modal: false,
        buff_sel: 0,
        buff_mask: 0,
        p_toggle: false, p_up: false, p_down: false,
        p_left: false, p_right: false, p_enter: false,
        logged_first: false,
    });
}

fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

/// Edge-trigger: trả true đúng một lần khi phím vừa được nhấn.
fn edge(now: bool, prev: &mut bool) -> bool {
    let fired = now && !*prev;
    *prev = now;
    fired
}

/// Khởi chạy overlay trên thread riêng. Trả về ngay.
pub fn start() {
    std::thread::spawn(|| unsafe { run() });
}

unsafe fn run() {
    let hinst = GetModuleHandleW(core::ptr::null());
    let class_name: Vec<u16> = "DarkSaberOverlay\0".encode_utf16().collect();

    let wc = WNDCLASSEXW {
        cbSize: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: core::ptr::null_mut(),
        hCursor: core::ptr::null_mut(),
        hbrBackground: core::ptr::null_mut(),
        lpszMenuName: core::ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: core::ptr::null_mut(),
    };
    RegisterClassExW(&wc);

    let title: Vec<u16> = "DarkSaber Overlay\0".encode_utf16().collect();
    let ex_style =
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let hwnd = CreateWindowExW(
        ex_style,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_POPUP | WS_VISIBLE,
        24,
        24,
        MAIN_W,
        WIN_H,
        core::ptr::null_mut(),
        core::ptr::null_mut(),
        hinst,
        core::ptr::null(),
    );
    if hwnd.is_null() {
        crate::log_line("[overlay] tạo cửa sổ thất bại");
        return;
    }

    // Trong suốt toàn cửa sổ với alpha (panel mờ ~ 90%).
    SetLayeredWindowAttributes(hwnd, 0, 230, LWA_ALPHA);
    // Nhịp ~33ms để poll phím + vẽ lại.
    SetTimer(hwnd, 1, 33, None);
    crate::log_line("[overlay] cửa sổ layered đã tạo (Wine/Win compatible)");

    let mut msg: MSG = core::mem::zeroed();
    while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            tick(hwnd);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Poll phím, cập nhật state, quyết định ẩn/hiện, yêu cầu vẽ lại.
unsafe fn tick(hwnd: HWND) {
    UI.with(|ui| {
        let mut ui = ui.borrow_mut();
        let buffs = state::known_buffs_snapshot();
        let modal_items = buffs.len() as i32 + 2;

        if !ui.logged_first {
            ui.logged_first = true;
            crate::log_line("[overlay] tick đầu tiên — overlay đang chạy");
        }

        // `\` bật/tắt menu.
        if edge(key_down(VK_TOGGLE), &mut ui.p_toggle) {
            if ui.buff_modal {
                ui.buff_modal = false;
            } else {
                let v = state::MENU_VISIBLE.load(core::sync::atomic::Ordering::Relaxed);
                state::MENU_VISIBLE.store(!v, core::sync::atomic::Ordering::Relaxed);
            }
        }

        let menu = state::MENU_VISIBLE.load(core::sync::atomic::Ordering::Relaxed);

        if menu {
            if ui.buff_modal {
                if edge(key_down(VK_UP), &mut ui.p_up) {
                    ui.buff_sel = (ui.buff_sel - 1 + modal_items) % modal_items;
                }
                if edge(key_down(VK_DOWN), &mut ui.p_down) {
                    ui.buff_sel = (ui.buff_sel + 1) % modal_items;
                }
                ui.p_left = key_down(VK_LEFT);
                ui.p_right = key_down(VK_RIGHT);
                if edge(key_down(VK_ENTER), &mut ui.p_enter) {
                    buff_modal_enter(&mut ui, &buffs);
                }
            } else {
                if edge(key_down(VK_UP), &mut ui.p_up) {
                    ui.sel = (ui.sel - 1 + ITEMS as i32) % ITEMS as i32;
                }
                if edge(key_down(VK_DOWN), &mut ui.p_down) {
                    ui.sel = (ui.sel + 1) % ITEMS as i32;
                }
                if edge(key_down(VK_LEFT), &mut ui.p_left) {
                    act(ui.sel, -1);
                }
                if edge(key_down(VK_RIGHT), &mut ui.p_right) {
                    act(ui.sel, 1);
                }
                if edge(key_down(VK_ENTER), &mut ui.p_enter) {
                    let sel = ui.sel;
                    act_enter(&mut ui, sel);
                }
            }
        } else {
            // vẫn cập nhật prev để không "nuốt" phím khi mở lại
            ui.p_up = key_down(VK_UP);
            ui.p_down = key_down(VK_DOWN);
            ui.p_left = key_down(VK_LEFT);
            ui.p_right = key_down(VK_RIGHT);
            ui.p_enter = key_down(VK_ENTER);
        }

        let visible = menu;
        ShowWindow(hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        if visible {
            let width = if ui.buff_modal { WIN_W } else { MAIN_W };
            // giữ topmost mà không cướp focus
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, width, WIN_H, SWP_NOMOVE);
            InvalidateRect(hwnd, core::ptr::null(), 1);
        }
    });
}

// ───────────────────────── thao tác menu ─────────────────────────
use core::sync::atomic::Ordering::Relaxed;

fn flip(flag: &core::sync::atomic::AtomicBool, dir: i32) {
    match dir {
        1 => flag.store(true, Relaxed),
        -1 => flag.store(false, Relaxed),
        _ => flag.store(!flag.load(Relaxed), Relaxed),
    }
}

fn buff_modal_enter(ui: &mut Ui, buffs: &[state::BuffOption]) {
    let buff_count = buffs.len() as i32;
    if ui.buff_sel < buff_count {
        let bit = 1usize << (ui.buff_sel as usize);
        ui.buff_mask ^= bit;
        return;
    }
    if ui.buff_sel == buff_count {
        state::BUFF_SELECTED_MASK.store(ui.buff_mask, Relaxed);
        state::REQ_APPLY_SELECTED_BUFFS.store(true, Relaxed);
        ui.buff_modal = false;
        return;
    }
    ui.buff_modal = false;
}

/// ←/→ trên một dòng (dir = -1 hoặc +1).
fn act(sel: i32, dir: i32) {
    let d = dir as f32;
    match sel {
        0 => flip(&state::GOD_MODE, dir),
        1 => flip(&state::SPEED_HACK, dir),
        2 => state::set_speed_value((state::speed_value() + d * 1.0).clamp(0.0, 40.0)),
        3 => flip(&state::RAPID_FIRE, dir),
        4 => state::set_fire_rate_value((state::fire_rate_value() + d * 0.01).clamp(0.01, 5.0)),
        5 => flip(&state::FAST_BULLETS, dir),
        6 => {
            state::set_bullet_speed_value((state::bullet_speed_value() + d * 2.0).clamp(0.0, 120.0))
        }
        7 => flip(&state::MAXHP_OVERRIDE, dir),
        8 => state::MAXHP_VALUE.store(
            (state::MAXHP_VALUE.load(Relaxed) + dir * 100).max(1),
            Relaxed,
        ),
        9 => flip(&state::SHIELD_LOCK, dir),
        10 => state::SHIELD_VALUE.store(
            (state::SHIELD_VALUE.load(Relaxed) + dir * 100).max(1),
            Relaxed,
        ),
        11 => flip(&state::ONE_HIT_KILL, dir),
        14 => state::GOLD_AMOUNT.store(
            (state::GOLD_AMOUNT.load(Relaxed) + dir * 100).max(0),
            Relaxed,
        ),
        16 => state::EXP_AMOUNT.store(
            (state::EXP_AMOUNT.load(Relaxed) + dir * 100).max(0),
            Relaxed,
        ),
        17 => {
            let max = state::item_preset_count().saturating_sub(1);
            state::ITEM_PRESET_INDEX.store(
                (state::ITEM_PRESET_INDEX.load(Relaxed) + dir).clamp(0, max),
                Relaxed,
            );
        }
        _ => {}
    }
}

/// Enter trên một dòng.
fn act_enter(ui: &mut Ui, sel: i32) {
    match sel {
        0 => flip(&state::GOD_MODE, 0),
        1 => flip(&state::SPEED_HACK, 0),
        3 => flip(&state::RAPID_FIRE, 0),
        5 => flip(&state::FAST_BULLETS, 0),
        7 => flip(&state::MAXHP_OVERRIDE, 0),
        9 => flip(&state::SHIELD_LOCK, 0),
        11 => flip(&state::ONE_HIT_KILL, 0),
        12 => state::REQ_REVIVE.store(true, Relaxed),
        13 => state::REQ_ADD_GOLD.store(true, Relaxed),
        15 => state::REQ_ADD_EXP.store(true, Relaxed),
        17 => state::REQ_ADD_ITEM_PRESET.store(true, Relaxed),
        18 => {
            ui.buff_modal = true;
            ui.buff_sel = 0;
        }
        19 => state::REQ_ADD_KNOWN_ITEMS.store(true, Relaxed),
        20 => state::REQ_SHIELD_ON.store(true, Relaxed),
        21 => state::REQ_SHIELD_OFF.store(true, Relaxed),
        _ => {}
    }
}

fn mark(flag: &core::sync::atomic::AtomicBool) -> char {
    if flag.load(Relaxed) {
        'x'
    } else {
        ' '
    }
}

fn label(i: i32) -> String {
    match i {
        0 => format!("[{}] God Mode (bat tu)", mark(&state::GOD_MODE)),
        1 => format!("[{}] Speed Hack", mark(&state::SPEED_HACK)),
        2 => format!("    Move Speed: {:.1}", state::speed_value()),
        3 => format!("[{}] Rapid Fire", mark(&state::RAPID_FIRE)),
        4 => format!("    Cooldown: {:.2}s", state::fire_rate_value()),
        5 => format!("[{}] Fast Bullets", mark(&state::FAST_BULLETS)),
        6 => format!("    Bullet Speed: {:.1}", state::bullet_speed_value()),
        7 => format!("[{}] Max/Lock HP", mark(&state::MAXHP_OVERRIDE)),
        8 => format!("    HP: {}", state::MAXHP_VALUE.load(Relaxed)),
        9 => format!("[{}] Shield Lock", mark(&state::SHIELD_LOCK)),
        10 => format!("    Shield HP: {}", state::SHIELD_VALUE.load(Relaxed)),
        11 => format!("[{}] One-Hit Kill", mark(&state::ONE_HIT_KILL)),
        12 => "> Revive (Hoi sinh)  [Enter]".to_string(),
        13 => format!("> Add Gold: {}  [Enter]", state::GOLD_AMOUNT.load(Relaxed)),
        14 => format!("    Gold amount: {}", state::GOLD_AMOUNT.load(Relaxed)),
        15 => format!("> Add EXP: {}  [Enter]", state::EXP_AMOUNT.load(Relaxed)),
        16 => format!("    EXP amount: {}", state::EXP_AMOUNT.load(Relaxed)),
        17 => {
            let item = state::item_preset(state::ITEM_PRESET_INDEX.load(Relaxed));
            format!("> Add item: {} ({})  [Enter]", item.name, item.id)
        }
        18 => "> Get Buffs...  [Enter]".to_string(),
        19 => "> Add all known items  [Enter]".to_string(),
        20 => "> Shield ON  [Enter]".to_string(),
        21 => "> Shield OFF  [Enter]".to_string(),
        _ => String::new(),
    }
}

// ───────────────────────── vẽ GDI ─────────────────────────
unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = core::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let show_buff_panel = UI.with(|u| u.borrow().buff_modal);

    // nền cửa sổ tổng.
    let full = RECT {
        left: 0,
        top: 0,
        right: if show_buff_panel { WIN_W } else { MAIN_W },
        bottom: WIN_H,
    };
    let bg = CreateSolidBrush(COL_BG);
    FillRect(hdc, &full, bg);
    DeleteObject(bg as _);

    if show_buff_panel {
        let gap = RECT {
            left: MAIN_W,
            top: 0,
            right: MAIN_W + PANEL_GAP,
            bottom: WIN_H,
        };
        let gap_bg = CreateSolidBrush(rgb(22, 26, 34));
        FillRect(hdc, &gap, gap_bg);
        DeleteObject(gap_bg as _);
    }

    // font
    let face: Vec<u16> = "Consolas\0".encode_utf16().collect();
    let font: HFONT = CreateFontW(-16, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, face.as_ptr());
    let old = SelectObject(hdc, font as _);
    SetBkMode(hdc, TRANSPARENT as i32);

    // tiêu đề
    SetTextColor(hdc, COL_HEAD);
    draw(hdc, LEFT, 10, "DarkSaber Trainer");
    SetTextColor(hdc, COL_DIM);
    draw(hdc, LEFT, 32, "\\ mo/dong  -  arrows chinh  -  Enter dung");

    let sel = UI.with(|u| u.borrow().sel);
    for i in 0..ITEMS as i32 {
        let y = TOP + i * LINE_H;
        if i == sel {
            let r = RECT {
                left: 6,
                top: y - 2,
                right: MAIN_W - 6,
                bottom: y + LINE_H - 4,
            };
            let hb = CreateSolidBrush(COL_SEL);
            FillRect(hdc, &r, hb);
            DeleteObject(hb as _);
            SetTextColor(hdc, COL_HEAD);
        } else {
            SetTextColor(hdc, COL_TEXT);
        }
        draw(hdc, LEFT, y, &label(i));
    }

    UI.with(|u| {
        let ui = u.borrow();
        if !ui.buff_modal {
            return;
        }

        let buffs = state::known_buffs_snapshot();
        let box_top = 18;
        let box_h = (92 + (buffs.len() as i32 + 2) * LINE_H).clamp(180, WIN_H - 36);
        let box_left = MAIN_W + PANEL_GAP;
        let box_right = WIN_W - 12;
        let modal = RECT {
            left: box_left,
            top: box_top,
            right: box_right,
            bottom: box_top + box_h,
        };
        let modal_bg = CreateSolidBrush(rgb(10, 14, 18));
        FillRect(hdc, &modal, modal_bg);
        DeleteObject(modal_bg as _);

        SetTextColor(hdc, COL_HEAD);
        draw(hdc, box_left + 14, box_top + 10, "Select Buffs");
        SetTextColor(hdc, COL_DIM);
        draw(
            hdc,
            box_left + 14,
            box_top + 34,
            "Catalog buff tu RE. Buff spawn se cap nhat value/cost.",
        );

        let mut row = 0i32;
        for (idx, buff) in buffs.iter().enumerate() {
            let y = box_top + 62 + row * LINE_H;
            if ui.buff_sel == row {
                let r = RECT {
                    left: box_left + 6,
                    top: y - 2,
                    right: box_right - 6,
                    bottom: y + LINE_H - 4,
                };
                let hb = CreateSolidBrush(COL_SEL);
                FillRect(hdc, &r, hb);
                DeleteObject(hb as _);
                SetTextColor(hdc, COL_HEAD);
            } else {
                SetTextColor(hdc, COL_TEXT);
            }
            let checked = if (ui.buff_mask & (1usize << idx)) != 0 {
                'x'
            } else {
                ' '
            };
            let value = if buff.is_percent {
                format!("{}%", buff.value)
            } else {
                buff.value.to_string()
            };
            let cost = if buff.cost > 0 {
                format!("  cost={}", buff.cost)
            } else {
                String::new()
            };
            let text = format!(
                "[{}] {}  val={}{}",
                checked,
                state::buff_kind_name(buff.stat_type, buff.is_percent),
                value,
                cost
            );
            draw(hdc, box_left + 14, y, &text);
            row += 1;
        }

        for extra in 0..2 {
            let y = box_top + 62 + row * LINE_H;
            if ui.buff_sel == row {
                let r = RECT {
                    left: box_left + 6,
                    top: y - 2,
                    right: box_right - 6,
                    bottom: y + LINE_H - 4,
                };
                let hb = CreateSolidBrush(COL_SEL);
                FillRect(hdc, &r, hb);
                DeleteObject(hb as _);
                SetTextColor(hdc, COL_HEAD);
            } else {
                SetTextColor(hdc, COL_TEXT);
            }
            if extra == 0 {
                draw(hdc, box_left + 14, y, "> Apply selected buffs");
            } else {
                draw(hdc, box_left + 14, y, "> Close");
            }
            row += 1;
        }
    });

    SelectObject(hdc, old);
    DeleteObject(font as _);
    EndPaint(hwnd, &ps);
}

unsafe fn draw(hdc: windows_sys::Win32::Graphics::Gdi::HDC, x: i32, y: i32, s: &str) {
    let w: Vec<u16> = s.encode_utf16().collect();
    TextOutW(hdc, x, y, w.as_ptr(), w.len() as i32);
}
