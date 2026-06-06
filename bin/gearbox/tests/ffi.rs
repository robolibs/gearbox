use std::ffi::CStr;

#[test]
fn c_abi_version_works() {
    let ptr = gearbox::ffi::gearbox_version();
    assert!(!ptr.is_null());
    let version = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .expect("version is utf-8")
        .to_owned();
    gearbox::ffi::gearbox_string_free(ptr);
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    assert!(gearbox::ffi::gearbox_last_error_message().is_null());
}
