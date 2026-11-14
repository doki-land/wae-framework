//! Node-API C ABI + runtime symbol table (napi-sys `dyn-symbols` model).
//!
//! Compile-time: no link against node.exe.
//! Load-time: resolve `napi_*` from the host process via libloading.

#![allow(non_camel_case_types, non_upper_case_globals, dead_code, static_mut_refs)]
#![allow(clippy::missing_safety_doc, clippy::too_many_arguments)]
#![allow(unsafe_op_in_unsafe_fn)]

use std::{ffi::c_void, os::raw::c_char, sync::OnceLock};

use libloading::Library;

pub type napi_env = *mut c_void;
pub type napi_value = *mut c_void;
pub type napi_callback_info = *mut c_void;
pub type napi_status = i32;

pub const napi_ok: napi_status = 0;
pub const napi_undefined: i32 = 0;
pub const napi_null: i32 = 1;
pub const napi_boolean: i32 = 2;
pub const napi_number: i32 = 3;
pub const napi_string: i32 = 4;
pub const napi_symbol: i32 = 5;
pub const napi_object: i32 = 6;
pub const napi_function: i32 = 7;
pub const napi_external: i32 = 8;
pub const napi_bigint: i32 = 9;

pub type napi_callback = Option<unsafe extern "C" fn(env: napi_env, info: napi_callback_info) -> napi_value>;

type FnGetCbInfo =
    unsafe extern "C" fn(napi_env, napi_callback_info, *mut usize, *mut napi_value, *mut napi_value, *mut *mut c_void) -> napi_status;
type FnCreateFunction = unsafe extern "C" fn(napi_env, *const c_char, usize, napi_callback, *mut c_void, *mut napi_value) -> napi_status;
type FnSetNamed = unsafe extern "C" fn(napi_env, napi_value, *const c_char, napi_value) -> napi_status;
type FnGetNamed = unsafe extern "C" fn(napi_env, napi_value, *const c_char, *mut napi_value) -> napi_status;
type FnTypeof = unsafe extern "C" fn(napi_env, napi_value, *mut i32) -> napi_status;
type FnGetString = unsafe extern "C" fn(napi_env, napi_value, *mut c_char, usize, *mut usize) -> napi_status;
type FnCreateString = unsafe extern "C" fn(napi_env, *const c_char, usize, *mut napi_value) -> napi_status;
type FnGetBool = unsafe extern "C" fn(napi_env, napi_value, *mut bool) -> napi_status;
type FnGetDouble = unsafe extern "C" fn(napi_env, napi_value, *mut f64) -> napi_status;
type FnGetI64 = unsafe extern "C" fn(napi_env, napi_value, *mut i64) -> napi_status;
type FnCreateDouble = unsafe extern "C" fn(napi_env, f64, *mut napi_value) -> napi_status;
type FnCreateI64 = unsafe extern "C" fn(napi_env, i64, *mut napi_value) -> napi_status;
type FnGetBoolean = unsafe extern "C" fn(napi_env, bool, *mut napi_value) -> napi_status;
type FnCreateObject = unsafe extern "C" fn(napi_env, *mut napi_value) -> napi_status;
type FnCreateArray = unsafe extern "C" fn(napi_env, usize, *mut napi_value) -> napi_status;
type FnSetElement = unsafe extern "C" fn(napi_env, napi_value, u32, napi_value) -> napi_status;
type FnThrowError = unsafe extern "C" fn(napi_env, *const c_char, *const c_char) -> napi_status;
type FnGetUndefined = unsafe extern "C" fn(napi_env, *mut napi_value) -> napi_status;
type FnGetNull = unsafe extern "C" fn(napi_env, *mut napi_value) -> napi_status;
type FnIsArray = unsafe extern "C" fn(napi_env, napi_value, *mut bool) -> napi_status;
type FnGetElement = unsafe extern "C" fn(napi_env, napi_value, u32, *mut napi_value) -> napi_status;
type FnGetPropertyNames = unsafe extern "C" fn(napi_env, napi_value, *mut napi_value) -> napi_status;

// Typed stubs matching each function signature.
unsafe extern "C" fn stub_get_cb_info(
    _: napi_env,
    _: napi_callback_info,
    _: *mut usize,
    _: *mut napi_value,
    _: *mut napi_value,
    _: *mut *mut c_void,
) -> napi_status {
    eprintln!("Node-API symbol napi_get_cb_info has not been loaded");
    1
}
unsafe extern "C" fn stub_create_function(
    _: napi_env,
    _: *const c_char,
    _: usize,
    _: napi_callback,
    _: *mut c_void,
    _: *mut napi_value,
) -> napi_status {
    eprintln!("Node-API symbol napi_create_function has not been loaded");
    1
}
unsafe extern "C" fn stub_set_named(_: napi_env, _: napi_value, _: *const c_char, _: napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_set_named_property has not been loaded");
    1
}
unsafe extern "C" fn stub_get_named(_: napi_env, _: napi_value, _: *const c_char, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_get_named_property has not been loaded");
    1
}
unsafe extern "C" fn stub_typeof(_: napi_env, _: napi_value, _: *mut i32) -> napi_status {
    eprintln!("Node-API symbol napi_typeof has not been loaded");
    1
}
unsafe extern "C" fn stub_get_string(_: napi_env, _: napi_value, _: *mut c_char, _: usize, _: *mut usize) -> napi_status {
    eprintln!("Node-API symbol napi_get_value_string_utf8 has not been loaded");
    1
}
unsafe extern "C" fn stub_create_string(_: napi_env, _: *const c_char, _: usize, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_create_string_utf8 has not been loaded");
    1
}
unsafe extern "C" fn stub_get_bool(_: napi_env, _: napi_value, _: *mut bool) -> napi_status {
    eprintln!("Node-API symbol napi_get_value_bool has not been loaded");
    1
}
unsafe extern "C" fn stub_get_double(_: napi_env, _: napi_value, _: *mut f64) -> napi_status {
    eprintln!("Node-API symbol napi_get_value_double has not been loaded");
    1
}
unsafe extern "C" fn stub_get_i64(_: napi_env, _: napi_value, _: *mut i64) -> napi_status {
    eprintln!("Node-API symbol napi_get_value_int64 has not been loaded");
    1
}
unsafe extern "C" fn stub_create_double(_: napi_env, _: f64, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_create_double has not been loaded");
    1
}
unsafe extern "C" fn stub_create_i64(_: napi_env, _: i64, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_create_int64 has not been loaded");
    1
}
unsafe extern "C" fn stub_get_boolean(_: napi_env, _: bool, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_get_boolean has not been loaded");
    1
}
unsafe extern "C" fn stub_create_object(_: napi_env, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_create_object has not been loaded");
    1
}
unsafe extern "C" fn stub_create_array(_: napi_env, _: usize, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_create_array_with_length has not been loaded");
    1
}
unsafe extern "C" fn stub_set_element(_: napi_env, _: napi_value, _: u32, _: napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_set_element has not been loaded");
    1
}
unsafe extern "C" fn stub_throw_error(_: napi_env, _: *const c_char, _: *const c_char) -> napi_status {
    eprintln!("Node-API symbol napi_throw_error has not been loaded");
    1
}
unsafe extern "C" fn stub_get_undefined(_: napi_env, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_get_undefined has not been loaded");
    1
}
unsafe extern "C" fn stub_get_null(_: napi_env, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_get_null has not been loaded");
    1
}
unsafe extern "C" fn stub_is_array(_: napi_env, _: napi_value, _: *mut bool) -> napi_status {
    eprintln!("Node-API symbol napi_is_array has not been loaded");
    1
}
unsafe extern "C" fn stub_get_element(_: napi_env, _: napi_value, _: u32, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_get_element has not been loaded");
    1
}
unsafe extern "C" fn stub_get_property_names(_: napi_env, _: napi_value, _: *mut napi_value) -> napi_status {
    eprintln!("Node-API symbol napi_get_property_names has not been loaded");
    1
}

struct NapiTable {
    napi_get_cb_info: FnGetCbInfo,
    napi_create_function: FnCreateFunction,
    napi_set_named_property: FnSetNamed,
    napi_get_named_property: FnGetNamed,
    napi_typeof: FnTypeof,
    napi_get_value_string_utf8: FnGetString,
    napi_create_string_utf8: FnCreateString,
    napi_get_value_bool: FnGetBool,
    napi_get_value_double: FnGetDouble,
    napi_get_value_int64: FnGetI64,
    napi_create_double: FnCreateDouble,
    napi_create_int64: FnCreateI64,
    napi_get_boolean: FnGetBoolean,
    napi_create_object: FnCreateObject,
    napi_create_array_with_length: FnCreateArray,
    napi_set_element: FnSetElement,
    napi_throw_error: FnThrowError,
    napi_get_undefined: FnGetUndefined,
    napi_get_null: FnGetNull,
    napi_is_array: FnIsArray,
    napi_get_element: FnGetElement,
    napi_get_property_names: FnGetPropertyNames,
}

static mut NAPI: NapiTable = NapiTable {
    napi_get_cb_info: stub_get_cb_info,
    napi_create_function: stub_create_function,
    napi_set_named_property: stub_set_named,
    napi_get_named_property: stub_get_named,
    napi_typeof: stub_typeof,
    napi_get_value_string_utf8: stub_get_string,
    napi_create_string_utf8: stub_create_string,
    napi_get_value_bool: stub_get_bool,
    napi_get_value_double: stub_get_double,
    napi_get_value_int64: stub_get_i64,
    napi_create_double: stub_create_double,
    napi_create_int64: stub_create_i64,
    napi_get_boolean: stub_get_boolean,
    napi_create_object: stub_create_object,
    napi_create_array_with_length: stub_create_array,
    napi_set_element: stub_set_element,
    napi_throw_error: stub_throw_error,
    napi_get_undefined: stub_get_undefined,
    napi_get_null: stub_get_null,
    napi_is_array: stub_is_array,
    napi_get_element: stub_get_element,
    napi_get_property_names: stub_get_property_names,
};

static HOST: OnceLock<Library> = OnceLock::new();

unsafe fn load_symbol(host: &Library, name: &'static [u8]) -> Option<*mut c_void> {
    match unsafe { host.get::<unsafe extern "C" fn()>(name) } {
        Ok(symbol) => unsafe { symbol.try_as_raw_ptr() },
        Err(_) => None,
    }
}

#[cfg(windows)]
unsafe fn test_library(
    lib: Result<libloading::os::windows::Library, libloading::Error>,
) -> Result<libloading::os::windows::Library, libloading::Error> {
    let lib = lib?;
    match unsafe { lib.get::<*mut c_void>(b"napi_create_object\0") } {
        Ok(_) => Ok(lib),
        Err(err) => Err(err),
    }
}

fn find_host() -> Result<Library, libloading::Error> {
    #[cfg(windows)]
    unsafe {
        test_library(libloading::os::windows::Library::this())
            .or_else(|_| test_library(libloading::os::windows::Library::open_already_loaded("libnode")))
            .or_else(|_| test_library(libloading::os::windows::Library::open_already_loaded("node")))
            .or_else(|_| test_library(libloading::os::windows::Library::new("node")))
            .or_else(|_| test_library(libloading::os::windows::Library::new("libnode")))
            .map(Into::into)
    }
    #[cfg(unix)]
    {
        // libloading 0.9：unix 上 `Library::this` 为安全函数。
        Ok(libloading::os::unix::Library::this().into())
    }
}

unsafe fn fill_table(host: &Library) {
    macro_rules! load_one {
        ($field:ident, $bytes:expr, $ty:ty) => {
            if let Some(ptr) = load_symbol(host, $bytes) {
                NAPI.$field = std::mem::transmute::<*mut c_void, $ty>(ptr);
            }
        };
    }

    load_one!(napi_get_cb_info, b"napi_get_cb_info\0", FnGetCbInfo);
    load_one!(napi_create_function, b"napi_create_function\0", FnCreateFunction);
    load_one!(napi_set_named_property, b"napi_set_named_property\0", FnSetNamed);
    load_one!(napi_get_named_property, b"napi_get_named_property\0", FnGetNamed);
    load_one!(napi_typeof, b"napi_typeof\0", FnTypeof);
    load_one!(napi_get_value_string_utf8, b"napi_get_value_string_utf8\0", FnGetString);
    load_one!(napi_create_string_utf8, b"napi_create_string_utf8\0", FnCreateString);
    load_one!(napi_get_value_bool, b"napi_get_value_bool\0", FnGetBool);
    load_one!(napi_get_value_double, b"napi_get_value_double\0", FnGetDouble);
    load_one!(napi_get_value_int64, b"napi_get_value_int64\0", FnGetI64);
    load_one!(napi_create_double, b"napi_create_double\0", FnCreateDouble);
    load_one!(napi_create_int64, b"napi_create_int64\0", FnCreateI64);
    load_one!(napi_get_boolean, b"napi_get_boolean\0", FnGetBoolean);
    load_one!(napi_create_object, b"napi_create_object\0", FnCreateObject);
    load_one!(napi_create_array_with_length, b"napi_create_array_with_length\0", FnCreateArray);
    load_one!(napi_set_element, b"napi_set_element\0", FnSetElement);
    load_one!(napi_throw_error, b"napi_throw_error\0", FnThrowError);
    load_one!(napi_get_undefined, b"napi_get_undefined\0", FnGetUndefined);
    load_one!(napi_get_null, b"napi_get_null\0", FnGetNull);
    load_one!(napi_is_array, b"napi_is_array\0", FnIsArray);
    load_one!(napi_get_element, b"napi_get_element\0", FnGetElement);
    load_one!(napi_get_property_names, b"napi_get_property_names\0", FnGetPropertyNames);
}

/// Resolve N-API symbols from the host. Idempotent; call from module init.
pub fn setup() {
    let _ = HOST.get_or_init(|| {
        let host = find_host().unwrap_or_else(|err| {
            panic!("failed to locate Node host for N-API symbols: {err}");
        });
        unsafe {
            fill_table(&host);
        }
        host
    });
}

#[inline]
pub unsafe fn napi_get_cb_info(
    env: napi_env,
    cbinfo: napi_callback_info,
    argc: *mut usize,
    argv: *mut napi_value,
    this_arg: *mut napi_value,
    data: *mut *mut c_void,
) -> napi_status {
    (NAPI.napi_get_cb_info)(env, cbinfo, argc, argv, this_arg, data)
}

#[inline]
pub unsafe fn napi_create_function(
    env: napi_env,
    utf8name: *const c_char,
    length: usize,
    cb: napi_callback,
    data: *mut c_void,
    result: *mut napi_value,
) -> napi_status {
    (NAPI.napi_create_function)(env, utf8name, length, cb, data, result)
}

#[inline]
pub unsafe fn napi_set_named_property(env: napi_env, object: napi_value, utf8name: *const c_char, value: napi_value) -> napi_status {
    (NAPI.napi_set_named_property)(env, object, utf8name, value)
}

#[inline]
pub unsafe fn napi_get_named_property(env: napi_env, object: napi_value, utf8name: *const c_char, result: *mut napi_value) -> napi_status {
    (NAPI.napi_get_named_property)(env, object, utf8name, result)
}

#[inline]
pub unsafe fn napi_typeof(env: napi_env, value: napi_value, result: *mut i32) -> napi_status {
    (NAPI.napi_typeof)(env, value, result)
}

#[inline]
pub unsafe fn napi_get_value_string_utf8(
    env: napi_env,
    value: napi_value,
    buf: *mut c_char,
    bufsize: usize,
    result: *mut usize,
) -> napi_status {
    (NAPI.napi_get_value_string_utf8)(env, value, buf, bufsize, result)
}

#[inline]
pub unsafe fn napi_create_string_utf8(env: napi_env, str: *const c_char, length: usize, result: *mut napi_value) -> napi_status {
    (NAPI.napi_create_string_utf8)(env, str, length, result)
}

#[inline]
pub unsafe fn napi_get_value_bool(env: napi_env, value: napi_value, result: *mut bool) -> napi_status {
    (NAPI.napi_get_value_bool)(env, value, result)
}

#[inline]
pub unsafe fn napi_get_value_double(env: napi_env, value: napi_value, result: *mut f64) -> napi_status {
    (NAPI.napi_get_value_double)(env, value, result)
}

#[inline]
pub unsafe fn napi_get_value_int64(env: napi_env, value: napi_value, result: *mut i64) -> napi_status {
    (NAPI.napi_get_value_int64)(env, value, result)
}

#[inline]
pub unsafe fn napi_create_double(env: napi_env, value: f64, result: *mut napi_value) -> napi_status {
    (NAPI.napi_create_double)(env, value, result)
}

#[inline]
pub unsafe fn napi_create_int64(env: napi_env, value: i64, result: *mut napi_value) -> napi_status {
    (NAPI.napi_create_int64)(env, value, result)
}

#[inline]
pub unsafe fn napi_get_boolean(env: napi_env, value: bool, result: *mut napi_value) -> napi_status {
    (NAPI.napi_get_boolean)(env, value, result)
}

#[inline]
pub unsafe fn napi_create_object(env: napi_env, result: *mut napi_value) -> napi_status {
    (NAPI.napi_create_object)(env, result)
}

#[inline]
pub unsafe fn napi_create_array_with_length(env: napi_env, length: usize, result: *mut napi_value) -> napi_status {
    (NAPI.napi_create_array_with_length)(env, length, result)
}

#[inline]
pub unsafe fn napi_set_element(env: napi_env, object: napi_value, index: u32, value: napi_value) -> napi_status {
    (NAPI.napi_set_element)(env, object, index, value)
}

#[inline]
pub unsafe fn napi_throw_error(env: napi_env, code: *const c_char, msg: *const c_char) -> napi_status {
    (NAPI.napi_throw_error)(env, code, msg)
}

#[inline]
pub unsafe fn napi_get_undefined(env: napi_env, result: *mut napi_value) -> napi_status {
    (NAPI.napi_get_undefined)(env, result)
}

#[inline]
pub unsafe fn napi_get_null(env: napi_env, result: *mut napi_value) -> napi_status {
    (NAPI.napi_get_null)(env, result)
}

#[inline]
pub unsafe fn napi_is_array(env: napi_env, value: napi_value, result: *mut bool) -> napi_status {
    (NAPI.napi_is_array)(env, value, result)
}

#[inline]
pub unsafe fn napi_get_element(env: napi_env, object: napi_value, index: u32, result: *mut napi_value) -> napi_status {
    (NAPI.napi_get_element)(env, object, index, result)
}

#[inline]
pub unsafe fn napi_get_property_names(env: napi_env, object: napi_value, result: *mut napi_value) -> napi_status {
    (NAPI.napi_get_property_names)(env, object, result)
}
