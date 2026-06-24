//! echo.dll — IL2CPP trainer + overlay cho GameAssembly.dll (Unity, x64).
//!
//! Nạp:
//!   1. First-party: gọi export `StartIl2CppProbe`.
//!   2. DllMain: tự bootstrap khi DLL được nạp (LoadLibrary/inject).
//!
//! Bootstrap: attach VM → dump metadata → cài il2cpp hook (chỉ số/quyền năng)
//! → bật overlay cửa sổ riêng (phím `\` bật/tắt — tương thích Wine + Windows).

#![allow(clippy::missing_safety_doc)]

mod dumper;
mod hooks;
mod il2cpp;
mod overlay;
mod state;

use core::ffi::c_void;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{BOOL, HMODULE};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

use il2cpp::Il2Cpp;

static STARTED: AtomicBool = AtomicBool::new(false);

/// Ghi log vào il2cpp_probe.log (cạnh executable).
pub fn log_line(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("il2cpp_probe.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}

fn bootstrap() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    log_line("[probe] bootstrap bắt đầu");

    // Chờ runtime IL2CPP sẵn sàng (~tối đa 30s).
    let api = 'wait: {
        for attempt in 0..150 {
            match unsafe { Il2Cpp::resolve() } {
                Ok(api) => break 'wait Some(api),
                Err(e) => {
                    if attempt % 20 == 0 {
                        log_line(&format!("[probe] chờ runtime ({})", e));
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
        None
    };
    let api: Il2Cpp = match api {
        Some(a) => a,
        None => {
            log_line("[probe] hết giờ chờ — không resolve được IL2CPP");
            return;
        }
    };

    unsafe {
        // Attach để resolve metadata; LẤY con trỏ thread để detach sau (tránh GC crash).
        let vm_thread = api.attach();
        log_line("[probe] đã attach thread vào VM (sẽ detach sau khi cài hook)");

        // Chờ Assembly-CSharp sẵn sàng (runtime có thể chưa init xong các class).
        let mut image = core::ptr::null_mut();
        for _ in 0..100 {
            image = hooks::find_game_image(&api, "Assembly-CSharp");
            if !image.is_null() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        if image.is_null() {
            log_line("[probe] không thấy Assembly-CSharp — không cài hook");
        } else {
            hooks::install(&api, image);
            log_line("[probe] đã cài hook");
        }

        // Dump CHỈ khi có file yêu cầu (thao tác nặng/rủi ro, không chạy mặc định).
        if std::path::Path::new("dump_request.txt").exists() {
            match dumper::dump_to_file(&api) {
                Ok(p) => log_line(&format!("[probe] dump: {}", p.display())),
                Err(e) => log_line(&format!("[probe] dump lỗi: {}", e)),
            }
        }

        // QUAN TRỌNG: detach trước khi thread bootstrap thoát → GC không quét stack chết.
        api.detach(vm_thread);
        log_line("[probe] đã detach thread bootstrap");
    }

    // Bật overlay cửa sổ riêng (tương thích Wine + Windows; không hook đồ hoạ game).
    overlay::start();
    log_line("[probe] overlay đã khởi chạy (\\ để mở menu)");

    log_line("[probe] bootstrap hoàn tất");
}

/// Export để loader xác nhận DLL đã nạp (loaderhonk gọi qua CreateRemoteThread).
///
/// QUAN TRỌNG: KHÔNG làm việc nặng ở đây. Thread do CreateRemoteThread tạo dưới
/// Wine/CrossOver có stack rất nhỏ → prologue nặng + `std::thread::spawn` gây
/// ACCESS_VIOLATION ngay tại entry (đã xác nhận qua crash.dmp: fault tại
/// StartIl2CppProbe+0x0). Bootstrap được kích hoạt an toàn bởi DllMain
/// (LoadLibrary chạy trên thread loader có stack chuẩn). Giữ hàm này là LEAF:
/// chỉ trả 1 để loader thấy "ready", không đụng stack/heap/thread.
#[no_mangle]
pub extern "system" fn StartIl2CppProbe() -> i32 {
    1
}

/// Điểm vào DLL.
#[no_mangle]
pub extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        unsafe { DisableThreadLibraryCalls(module) };
        std::thread::spawn(bootstrap);
    }
    1
}
