//! Safe loader for Echo.exe + echo.dll.
//!
//! The previous loader calls `StartIl2CppProbe` with `CreateRemoteThread`.
//! Under Wine/CrossOver that remote export address was truncated to 32 bits
//! (e.g. `0x6FFFF8ADC570` -> `0xF8ADC570`), so the process jumped outside any
//! module and crashed immediately. This loader only injects with `LoadLibraryA`;
//! `echo.dll` bootstraps itself from DllMain.

use std::ffi::CString;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};
use windows_sys::Win32::System::Threading::{
    CreateProcessA, CreateRemoteThread, ResumeThread, WaitForSingleObject, CREATE_SUSPENDED,
    INFINITE, PROCESS_INFORMATION, STARTUPINFOA,
};

fn last_error(context: &str) -> String {
    format!("{context} failed: GetLastError={}", unsafe {
        GetLastError()
    })
}

fn cstring(s: impl AsRef<str>) -> Result<CString, String> {
    CString::new(s.as_ref()).map_err(|_| format!("string contains NUL: {}", s.as_ref()))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let exe = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Echo.exe".to_string());
    let dll = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "echo.dll".to_string());

    let current_dir = std::env::current_dir().map_err(|e| e.to_string())?;
    let dll_path = current_dir.join(&dll);
    let dll_path = dll_path
        .to_str()
        .ok_or_else(|| "DLL path is not valid UTF-8".to_string())?
        .to_string();

    println!("Target EXE: {exe}");
    println!("DLL: {dll_path}");

    let mut cmd = cstring(exe.clone())?.into_bytes_with_nul();
    let cwd = cstring(current_dir.to_string_lossy())?;

    let mut si = STARTUPINFOA {
        cb: std::mem::size_of::<STARTUPINFOA>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let ok = unsafe {
        CreateProcessA(
            null(),
            cmd.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_SUSPENDED,
            null(),
            cwd.as_ptr() as _,
            &mut si,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(last_error("CreateProcessA"));
    }

    let result = unsafe { inject(pi.hProcess, &dll_path) };

    unsafe {
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }

    result?;
    println!("Injection complete. {exe} resumed.");
    Ok(())
}

unsafe fn inject(process: *mut core::ffi::c_void, dll_path: &str) -> Result<(), String> {
    let dll = cstring(dll_path)?;
    let remote_mem = VirtualAllocEx(
        process,
        null(),
        dll.as_bytes_with_nul().len(),
        MEM_COMMIT | MEM_RESERVE,
        PAGE_READWRITE,
    );
    if remote_mem.is_null() {
        return Err(last_error("VirtualAllocEx"));
    }

    let mut written = 0usize;
    let ok = WriteProcessMemory(
        process,
        remote_mem,
        dll.as_ptr() as _,
        dll.as_bytes_with_nul().len(),
        &mut written,
    );
    if ok == 0 || written != dll.as_bytes_with_nul().len() {
        return Err(last_error("WriteProcessMemory"));
    }

    let kernel32 = cstring("kernel32.dll")?;
    let load_library = cstring("LoadLibraryA")?;
    let kernel32 = GetModuleHandleA(kernel32.as_ptr() as _);
    if kernel32.is_null() {
        return Err(last_error("GetModuleHandleA(kernel32.dll)"));
    }
    let proc = GetProcAddress(kernel32, load_library.as_ptr() as _)
        .ok_or_else(|| last_error("GetProcAddress(LoadLibraryA)"))?;

    let start: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32 = std::mem::transmute(proc);
    let thread = CreateRemoteThread(process, null(), 0, Some(start), remote_mem, 0, null_mut());
    if thread.is_null() {
        return Err(last_error("CreateRemoteThread(LoadLibraryA)"));
    }

    let wait = WaitForSingleObject(thread, INFINITE);
    CloseHandle(thread);
    if wait != WAIT_OBJECT_0 {
        return Err(format!("WaitForSingleObject returned 0x{wait:X}"));
    }

    Ok(())
}
