//! Dump metadata IL2CPP tại runtime.
//!
//! IDA không có `global-metadata.dat` nên mọi method trong .idb là `sub_xxx`.
//! Module này duyệt VM lúc chạy để lấy tên thật + chữ ký đầy đủ của assembly /
//! class / method, ghi ra file để bạn chọn target hook.

use core::ffi::c_void;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use crate::il2cpp::{cstr_to_string, Il2Cpp};

/// Duyệt toàn bộ assembly đã nạp và sinh báo cáo text.
/// Trả về (số_class, số_method, nội_dung).
pub unsafe fn dump(api: &Il2Cpp) -> (usize, usize, String) {
    let mut out = String::with_capacity(1 << 20);
    let mut total_classes = 0usize;
    let mut total_methods = 0usize;

    let domain = (api.domain_get)();
    let mut asm_count: usize = 0;
    let assemblies = (api.domain_get_assemblies)(domain, &mut asm_count);

    // Base preferred trong IDA. IDA_addr = IDA_BASE + (runtime_ptr - module_base).
    const IDA_BASE: usize = 0x180000000;

    let _ = writeln!(out, "// IL2CPP runtime dump — {} assembly", asm_count);
    let _ = writeln!(
        out,
        "// module_base (ASLR) = 0x{:x}  | IDA preferred base = 0x{:x}",
        api.module_base, IDA_BASE
    );

    for i in 0..asm_count {
        let assembly = *assemblies.add(i);
        if assembly.is_null() {
            continue;
        }
        let image = (api.assembly_get_image)(assembly);
        if image.is_null() {
            continue;
        }
        let image_name = cstr_to_string((api.image_get_name)(image));
        let class_count = (api.image_get_class_count)(image);
        let _ = writeln!(
            out,
            "\n//==================== {} ({} class) ====================",
            image_name, class_count
        );

        for ci in 0..class_count {
            let klass = (api.image_get_class)(image, ci);
            if klass.is_null() {
                continue;
            }
            total_classes += 1;
            let cname = cstr_to_string((api.class_get_name)(klass));
            let cns = cstr_to_string((api.class_get_namespace)(klass));
            let full = if cns.is_empty() {
                cname.clone()
            } else {
                format!("{}.{}", cns, cname)
            };
            let _ = writeln!(out, "\nclass {} {{", full);

            let mut iter: *mut c_void = core::ptr::null_mut();
            loop {
                let mi = (api.class_get_methods)(klass, &mut iter);
                if mi.is_null() {
                    break;
                }
                total_methods += 1;
                let mname = cstr_to_string((api.method_get_name)(mi));
                let argc = (api.method_get_param_count)(mi);

                // Chữ ký đầy đủ qua reflection: return + kiểu từng tham số.
                let ret = cstr_to_string((api.type_get_name)((api.method_get_return_type)(mi)));
                let mut params = String::new();
                for p in 0..argc {
                    if p > 0 {
                        params.push_str(", ");
                    }
                    let pty = (api.method_get_param)(mi, p);
                    if pty.is_null() {
                        params.push('?');
                    } else {
                        params.push_str(&cstr_to_string((api.type_get_name)(pty)));
                    }
                }

                let method_ptr = *(mi as *const usize);
                if method_ptr == 0 {
                    let _ = writeln!(out, "    {} {}({})  @ (no body)", ret, mname, params);
                } else {
                    let rva = method_ptr.wrapping_sub(api.module_base);
                    let ida = IDA_BASE.wrapping_add(rva);
                    let _ = writeln!(
                        out,
                        "    {} {}({})  @ 0x{:x}  (IDA 0x{:x}, RVA 0x{:x})",
                        ret, mname, params, method_ptr, ida, rva
                    );
                }
            }
            let _ = writeln!(out, "}}");
        }
    }

    let _ = writeln!(
        out,
        "\n// Tổng: {} class, {} method",
        total_classes, total_methods
    );
    (total_classes, total_methods, out)
}

/// Ghi dump cạnh executable của game (thư mục làm việc hiện tại).
pub unsafe fn dump_to_file(api: &Il2Cpp) -> std::io::Result<PathBuf> {
    let (_, _, content) = dump(api);
    let path = PathBuf::from("il2cpp_dump.txt");
    fs::write(&path, content)?;
    Ok(path)
}
