//! The ExactTeX compiler as a WebAssembly module.
//!
//! # Why the interface looks like this
//!
//! There is no `wasm-bindgen` here, for the reason in `docs/decisions/0005`:
//! the compiler carries no dependencies, and the boundary is small enough to
//! write. What that buys is a module with no generated glue and no runtime —
//! it is a `.wasm` file with a handful of exports and a linear memory.
//!
//! Every call takes bytes the caller put in our memory and returns a pointer
//! to bytes we put there. Nothing here opens a file, reads an environment
//! variable, or knows what a current directory is; the browser has none of
//! those, and neither does this.
//!
//! # The calling convention
//!
//! 1. `xtex_alloc(len)` returns a pointer to `len` writable bytes.
//! 2. The caller copies input there.
//! 3. An operation is called with `(ptr, len)` and returns a **result
//!    pointer**: four little-endian bytes of length, then that many bytes.
//! 4. The caller reads them, then calls `xtex_free_result(result)`.
//!
//! The length prefix is there so a caller never needs a second call to learn
//! how much to read, and so a result containing a zero byte — emitted LaTeX
//! can — is not truncated by a C-string convention.

use xtex_core::bibliography::{Bibliography, Unavailable};
use xtex_core::check::{check, to_json};
use xtex_core::source::Sources;
use xtex_core::sourcemap::emit_with_map;
use xtex_core::symbols::SymbolTable;
use xtex_core::{emit, parse};

/// The name every document is given inside the module.
///
/// A browser has no paths. Diagnostics still need a file name, and inventing a
/// stable one is honest where inventing a path would not be.
const DOCUMENT: &str = "document.xtex";

/// Reserves `len` bytes for the caller to write into.
///
/// # Panics
///
/// Never for a length the module can serve; an allocation failure aborts, as
/// it does anywhere else in Rust.
#[unsafe(no_mangle)]
pub extern "C" fn xtex_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Releases a buffer obtained from [`xtex_alloc`].
///
/// # Safety
///
/// `pointer` and `len` must be exactly what a previous `xtex_alloc` returned
/// and was asked for.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_free(pointer: *mut u8, len: usize) {
    if !pointer.is_null() {
        drop(unsafe { Vec::from_raw_parts(pointer, 0, len) });
    }
}

/// Releases a result returned by an operation.
///
/// # Safety
///
/// `result` must be a pointer an operation in this module returned, and must
/// not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_free_result(result: *mut u8) {
    if result.is_null() {
        return;
    }
    let len = unsafe { read_length(result) } + 4;
    // Length and capacity are the same number because `result` built the
    // buffer with `with_capacity(len)` and filled it exactly. Clippy warns
    // because that is usually a mistake; here it is the shape of the
    // allocation, and passing anything else would be the bug.
    #[allow(clippy::same_length_and_capacity)]
    drop(unsafe { Vec::from_raw_parts(result, len, len) });
}

/// Emits the document as LaTeX.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_emit(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut sources = Sources::new();
    let id = sources.add(DOCUMENT, bytes);
    let document = parse(&sources, id);
    let mut out = Vec::new();
    match emit(&sources, &document, &mut out) {
        Ok(()) => result(&out),
        // An emit failure is a broken internal invariant rather than a
        // document problem, so it is reported as no bytes rather than as a
        // diagnostic that would look like the author's fault.
        Err(_) => result(&[]),
    }
}

/// Checks the document and returns the JSON `xtex check --json` prints.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_check_json(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut sources = Sources::new();
    let id = sources.add(DOCUMENT, bytes);
    let document = parse(&sources, id);
    let mut table = SymbolTable::new();
    table.merge(&sources, &document);
    // No bibliography can be read here: the module is handed a document, not a
    // project. `Unavailable` is the state that keeps every citation silent.
    let diagnostics = check(
        &table,
        &Bibliography::Unavailable(Unavailable::NoneDeclared),
    );

    let mut json = String::new();
    to_json(&sources, &diagnostics, document.coverage(), &mut json);
    result(json.as_bytes())
}

/// Emits the document and returns its source map as JSON.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_source_map(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut sources = Sources::new();
    let id = sources.add(DOCUMENT, bytes);
    let document = parse(&sources, id);
    match emit_with_map(&sources, &document) {
        Ok(emission) => result(&emission.map.to_json()),
        Err(_) => result(&[]),
    }
}

/// Copies `len` bytes out of the caller's buffer.
unsafe fn input(pointer: *const u8, len: usize) -> Vec<u8> {
    if pointer.is_null() || len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(pointer, len) }.to_vec()
}

/// Wraps `bytes` as a length-prefixed result the caller owns.
fn result(bytes: &[u8]) -> *mut u8 {
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(bytes.len() + 4);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    let pointer = out.as_mut_ptr();
    std::mem::forget(out);
    pointer
}

/// The length a result pointer carries.
unsafe fn read_length(result: *const u8) -> usize {
    let header = unsafe { std::slice::from_raw_parts(result, 4) };
    u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize
}
