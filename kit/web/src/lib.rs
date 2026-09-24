//! VectorMagik in the browser: the desktop app itself, compiled to
//! WebAssembly and drawn on a canvas, so a picture is traced, edited and
//! saved on the visitor's own computer and never uploaded (the owner,
//! September 23, 2026: "fully browser ran ... it does all of the compute on
//! their computer", and then: "make both do both"). The window code is the
//! desktop's (`vector_magic_rebuild::desktop_ui`, built with the `ui`
//! feature and without the window); `app.rs` runs it one frame at a time for
//! `js/app.mjs`, which paints what it draws with WebGL.
//!
//! The interface is a handful of plain exports over the module's memory, no
//! binding generator (the offline rule): JavaScript copies bytes in
//! (`vm_alloc`), calls an export, and reads the answer back through
//! `vm_out_ptr` / `vm_out_len`. The module imports one function,
//! `env.vm_now_ms`, the page's clock (`vector_rebuild::clock`).

use std::cell::RefCell;

mod app;

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Leave `bytes` in the output buffer for the page to read.
fn set_out(bytes: Vec<u8>) {
    OUT.with(|out| *out.borrow_mut() = bytes);
}

/// # Safety
/// `ptr` and `len` must describe memory this module handed out.
unsafe fn slice<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        // SAFETY: the caller's promise.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

#[no_mangle]
pub extern "C" fn vm_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// # Safety
/// `ptr` must come from `vm_alloc(len)` and be freed once.
#[no_mangle]
pub unsafe extern "C" fn vm_dealloc(ptr: *mut u8, len: usize) {
    // SAFETY: the caller's promise.
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) });
}

#[no_mangle]
pub extern "C" fn vm_out_ptr() -> *const u8 {
    OUT.with(|out| out.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn vm_out_len() -> usize {
    OUT.with(|out| out.borrow().len())
}

#[cfg(test)]
mod tests;
