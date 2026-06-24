//! Binding tới API runtime IL2CPP của GameAssembly.dll.
//!
//! Tất cả hàm `il2cpp_*` được resolve tại runtime bằng `GetProcAddress` trên
//! export name table của module — không hardcode RVA, nên bền với mọi bản build.

use core::ffi::{c_char, c_void};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

// Con trỏ mờ (opaque) tới các struct nội bộ của runtime.
pub type Domain = *mut c_void;
pub type Assembly = *mut c_void;
pub type Image = *mut c_void;
pub type Class = *mut c_void;
/// `const MethodInfo*`. Field đầu tiên (offset 0x0) chính là `methodPointer` —
/// địa chỉ native thật sự của method, là thứ ta sẽ hook.
pub type MethodInfo = *const c_void;
pub type Iter = *mut c_void;
/// `const Il2CppType*` — mô tả kiểu của tham số / giá trị trả về.
pub type CppType = *const c_void;

// Kiểu hàm export (IL2CPP dùng quy ước gọi C = __fastcall trên x64 Windows).
type FnDomainGet = unsafe extern "C" fn() -> Domain;
type FnThreadAttach = unsafe extern "C" fn(Domain) -> *mut c_void;
type FnThreadDetach = unsafe extern "C" fn(*mut c_void);
type FnDomainGetAssemblies = unsafe extern "C" fn(Domain, *mut usize) -> *const Assembly;
type FnAssemblyGetImage = unsafe extern "C" fn(Assembly) -> Image;
type FnImageGetName = unsafe extern "C" fn(Image) -> *const c_char;
type FnImageGetClassCount = unsafe extern "C" fn(Image) -> usize;
type FnImageGetClass = unsafe extern "C" fn(Image, usize) -> Class;
type FnClassGetName = unsafe extern "C" fn(Class) -> *const c_char;
type FnClassGetNamespace = unsafe extern "C" fn(Class) -> *const c_char;
type FnClassGetMethods = unsafe extern "C" fn(Class, *mut Iter) -> MethodInfo;
type FnMethodGetName = unsafe extern "C" fn(MethodInfo) -> *const c_char;
type FnMethodGetParamCount = unsafe extern "C" fn(MethodInfo) -> u32;
type FnClassFromName = unsafe extern "C" fn(Image, *const c_char, *const c_char) -> Class;
type FnClassGetMethodFromName = unsafe extern "C" fn(Class, *const c_char, i32) -> MethodInfo;
type FnMethodGetReturnType = unsafe extern "C" fn(MethodInfo) -> CppType;
type FnMethodGetParam = unsafe extern "C" fn(MethodInfo, u32) -> CppType;
type FnTypeGetName = unsafe extern "C" fn(CppType) -> *const c_char;

/// Bảng con trỏ hàm IL2CPP đã resolve.
pub struct Il2Cpp {
    pub domain_get: FnDomainGet,
    pub thread_attach: FnThreadAttach,
    pub thread_detach: FnThreadDetach,
    pub domain_get_assemblies: FnDomainGetAssemblies,
    pub assembly_get_image: FnAssemblyGetImage,
    pub image_get_name: FnImageGetName,
    pub image_get_class_count: FnImageGetClassCount,
    pub image_get_class: FnImageGetClass,
    pub class_get_name: FnClassGetName,
    pub class_get_namespace: FnClassGetNamespace,
    pub class_get_methods: FnClassGetMethods,
    pub method_get_name: FnMethodGetName,
    pub method_get_param_count: FnMethodGetParamCount,
    pub class_from_name: FnClassFromName,
    pub class_get_method_from_name: FnClassGetMethodFromName,
    pub method_get_return_type: FnMethodGetReturnType,
    pub method_get_param: FnMethodGetParam,
    pub type_get_name: FnTypeGetName,
    /// Base thật của GameAssembly.dll sau ASLR (để tính RVA đối chiếu IDA).
    pub module_base: usize,
}

/// Lấy địa chỉ một export theo tên (PCSTR null-terminated).
unsafe fn proc(module: HMODULE, name: &[u8]) -> Option<*const c_void> {
    debug_assert_eq!(
        *name.last().unwrap(),
        0,
        "tên export phải kết thúc bằng \\0"
    );
    GetProcAddress(module, name.as_ptr()).map(|p| p as *const c_void)
}

macro_rules! load {
    ($module:expr, $name:literal) => {
        match proc($module, concat!($name, "\0").as_bytes()) {
            Some(p) => core::mem::transmute(p),
            None => return Err(concat!("thiếu export: ", $name)),
        }
    };
}

impl Il2Cpp {
    /// Resolve toàn bộ API từ module GameAssembly.dll đã nạp sẵn trong tiến trình.
    pub unsafe fn resolve() -> Result<Self, &'static str> {
        let module_name: Vec<u16> = "GameAssembly.dll\0".encode_utf16().collect();
        let module = GetModuleHandleW(module_name.as_ptr());
        if module.is_null() {
            return Err("không tìm thấy GameAssembly.dll trong tiến trình");
        }

        Ok(Il2Cpp {
            // il2cpp_domain_get đôi khi không export; thử nó trước, fallback mono_get_root_domain.
            domain_get: match proc(module, b"il2cpp_domain_get\0")
                .or_else(|| proc(module, b"mono_get_root_domain\0"))
            {
                Some(p) => core::mem::transmute(p),
                None => return Err("thiếu il2cpp_domain_get / mono_get_root_domain"),
            },
            thread_attach: load!(module, "il2cpp_thread_attach"),
            thread_detach: match proc(module, b"il2cpp_thread_detach\0")
                .or_else(|| proc(module, b"mono_thread_detach\0"))
            {
                Some(p) => core::mem::transmute(p),
                None => return Err("thiếu il2cpp_thread_detach / mono_thread_detach"),
            },
            domain_get_assemblies: load!(module, "il2cpp_domain_get_assemblies"),
            assembly_get_image: load!(module, "il2cpp_assembly_get_image"),
            image_get_name: load!(module, "il2cpp_image_get_name"),
            image_get_class_count: load!(module, "il2cpp_image_get_class_count"),
            image_get_class: load!(module, "il2cpp_image_get_class"),
            class_get_name: load!(module, "il2cpp_class_get_name"),
            class_get_namespace: load!(module, "il2cpp_class_get_namespace"),
            class_get_methods: load!(module, "il2cpp_class_get_methods"),
            method_get_name: load!(module, "il2cpp_method_get_name"),
            method_get_param_count: load!(module, "il2cpp_method_get_param_count"),
            class_from_name: load!(module, "il2cpp_class_from_name"),
            class_get_method_from_name: load!(module, "il2cpp_class_get_method_from_name"),
            method_get_return_type: load!(module, "il2cpp_method_get_return_type"),
            method_get_param: load!(module, "il2cpp_method_get_param"),
            type_get_name: load!(module, "il2cpp_type_get_name"),
            module_base: module as usize,
        })
    }

    /// Gắn thread hiện tại vào VM IL2CPP. Trả về con trỏ Il2CppThread để DETACH sau.
    /// PHẢI detach trước khi thread kết thúc, nếu không GC sẽ quét stack của thread
    /// đã chết → crash.
    pub unsafe fn attach(&self) -> *mut c_void {
        let domain = (self.domain_get)();
        (self.thread_attach)(domain)
    }

    /// Gỡ thread khỏi VM (gọi trên đúng thread đã attach, trước khi thread thoát).
    pub unsafe fn detach(&self, thread: *mut c_void) {
        if !thread.is_null() {
            (self.thread_detach)(thread);
        }
    }

    /// Lấy con trỏ native (methodPointer) của một managed method, sẵn sàng để hook.
    pub unsafe fn method_address(
        &self,
        image: Image,
        ns: &str,
        class: &str,
        method: &str,
        argc: i32,
    ) -> Option<*mut c_void> {
        let cns = to_cstr(ns);
        let cclass = to_cstr(class);
        let cmethod = to_cstr(method);

        let klass = (self.class_from_name)(
            image,
            cns.as_ptr() as *const c_char,
            cclass.as_ptr() as *const c_char,
        );
        if klass.is_null() {
            return None;
        }
        let mi = (self.class_get_method_from_name)(klass, cmethod.as_ptr() as *const c_char, argc);
        if mi.is_null() {
            return None;
        }
        // MethodInfo->methodPointer nằm tại offset 0.
        let method_ptr = *(mi as *const *mut c_void);
        if method_ptr.is_null() {
            None
        } else {
            Some(method_ptr)
        }
    }

    /// Như `method_address` nhưng trả cả `(methodPointer, MethodInfo*)` —
    /// cần `MethodInfo*` để truyền vào tham số ẩn cuối khi GỌI method.
    pub unsafe fn resolve_method(
        &self,
        image: Image,
        ns: &str,
        class: &str,
        method: &str,
        argc: i32,
    ) -> Option<(usize, usize)> {
        let cns = to_cstr(ns);
        let cclass = to_cstr(class);
        let cmethod = to_cstr(method);

        let klass = (self.class_from_name)(
            image,
            cns.as_ptr() as *const c_char,
            cclass.as_ptr() as *const c_char,
        );
        if klass.is_null() {
            return None;
        }
        let mi = (self.class_get_method_from_name)(klass, cmethod.as_ptr() as *const c_char, argc);
        if mi.is_null() {
            return None;
        }
        let code = *(mi as *const usize);
        if code == 0 {
            return None;
        }
        Some((code, mi as usize))
    }
}

/// Chuyển &str thành buffer C null-terminated.
pub fn to_cstr(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len() + 1);
    v.extend_from_slice(s.as_bytes());
    v.push(0);
    v
}

/// Đọc C-string (UTF-8) từ con trỏ runtime trả về thành String an toàn.
pub unsafe fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
    }
    let bytes = core::slice::from_raw_parts(p as *const u8, len);
    String::from_utf8_lossy(bytes).into_owned()
}
