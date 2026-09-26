//! Minimal C ABI: version, dimensions probe, blur-to-file helper.
//!
//! The full API lives in Rust; this module exposes the three calls C
//! hosts need for previews and smoke tests. Return codes follow the
//! `| Return | Meaning |` tables in `wiki/Ffi.md`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_float, c_int};

/// Library version string (borrowed; do not free).
#[no_mangle]
pub extern "C" fn coreimage_version() -> *const c_char {
    static VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
    VERSION.as_ptr() as *const c_char
}

/// Probe `path` dimensions. Writes `out_w`/`out_h`. Returns 1 on success.
#[no_mangle]
pub extern "C" fn coreimage_dimensions(
    path: *const c_char,
    out_w: *mut u32,
    out_h: *mut u32,
) -> c_int {
    if path.is_null() || out_w.is_null() || out_h.is_null() {
        return 0;
    }
    let p = unsafe { CStr::from_ptr(path).to_string_lossy().into_owned() };
    match crate::io::metadata(&p) {
        Ok(m) => {
            unsafe {
                *out_w = m.width;
                *out_h = m.height;
            }
            1
        }
        Err(_) => 0,
    }
}

/// Blur `input` into `output` as PNG. Returns 0 on success, negative on error.
#[no_mangle]
pub extern "C" fn coreimage_blur_to_file(
    input: *const c_char,
    output: *const c_char,
    sigma: c_float,
    err_out: *mut *mut c_char,
) -> c_int {
    if input.is_null() || output.is_null() {
        return -1;
    }
    let inp = unsafe { CStr::from_ptr(input).to_string_lossy().into_owned() };
    let outp = unsafe { CStr::from_ptr(output).to_string_lossy().into_owned() };
    let result = (|| -> Result<(), crate::ImageError> {
        let img = crate::io::load(&inp)?;
        let blurred = img.gaussian_blur(sigma);
        blurred.save(&outp, crate::io::ImageFormat::Png, 100)
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            if !err_out.is_null() {
                let msg = CString::new(e.to_string()).unwrap_or_default();
                unsafe {
                    *err_out = msg.into_raw();
                }
            }
            -2
        }
    }
}

/// Free a string produced by CoreImage.
#[no_mangle]
pub extern "C" fn coreimage_string_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}
