//! C ABI for gearbox.
//!
//! Conventions: fallible calls return NULL/false with the reason in the
//! thread-local gearbox_last_error_message(); owned strings returned to C
//! are freed with gearbox_string_free().
//!
//! `include/gearbox.h` is generated from this file by cbindgen.

// extern "C" fns take raw pointers from C and deref them by design.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::cell::RefCell;
use std::ffi::{CString, c_char};
use std::ptr;

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn clear_last_error() {
    LAST_ERROR.with(|s| *s.borrow_mut() = None);
}

fn set_last_error(msg: impl Into<String>) {
    let msg = msg.into().replace('\0', " ");
    LAST_ERROR.with(|s| {
        *s.borrow_mut() =
            Some(CString::new(msg).unwrap_or_else(|_| CString::new("gearbox ffi error").unwrap()));
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn gearbox_last_error_message() -> *const c_char {
    LAST_ERROR.with(|s| {
        s.borrow()
            .as_ref()
            .map(|m| m.as_ptr())
            .unwrap_or(ptr::null())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn gearbox_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: `s` must come from CString::into_raw in this ABI.
    unsafe { drop(CString::from_raw(s)) };
}

#[unsafe(no_mangle)]
pub extern "C" fn gearbox_version() -> *mut c_char {
    clear_last_error();
    match CString::new(crate::version()) {
        Ok(s) => s.into_raw(),
        Err(e) => {
            set_last_error(e.to_string());
            ptr::null_mut()
        }
    }
}
