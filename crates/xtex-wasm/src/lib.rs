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

use xtex_core::check::to_json;
use xtex_core::io::Memory;
use xtex_core::project::check_project;
use xtex_core::sourcemap::emit_with_map;
use xtex_core::{emit, parse};

/// A project, decoded from the caller's bundle.
///
/// # The bundle format
///
/// Everything little-endian, everything length-prefixed, nothing aligned —
/// readable from JavaScript with a `DataView` and nothing else:
///
/// ```text
/// u32 root_len   root_name (UTF-8)
/// u32 file_count
/// file_count × ( u32 name_len  name (UTF-8)  u32 data_len  data )
/// ```
///
/// Names are logical, `/`-separated, project-relative — the same names the
/// project's own `@import` and `\include` write. The host includes every file
/// a check may ask about; an asset that exists but is not source (a figure's
/// PDF) is listed with empty data, because existence is the only question ever
/// asked of it. See `docs/decisions/0007`.
struct Bundle {
    root: String,
    store: Memory,
}

/// Decodes a bundle, or returns `None` for one that lies about its lengths.
///
/// A malformed bundle is the caller's bug, not the author's document, so the
/// answer is no result rather than a diagnostic.
fn decode_bundle(bytes: &[u8]) -> Option<Bundle> {
    fn take_u32(bytes: &[u8], at: &mut usize) -> Option<usize> {
        let end = at.checked_add(4)?;
        let field = bytes.get(*at..end)?;
        *at = end;
        Some(u32::from_le_bytes([field[0], field[1], field[2], field[3]]) as usize)
    }
    fn take(bytes: &[u8], at: &mut usize, len: usize) -> Option<Vec<u8>> {
        let end = at.checked_add(len)?;
        let field = bytes.get(*at..end)?.to_vec();
        *at = end;
        Some(field)
    }

    let mut at = 0usize;
    let root_len = take_u32(bytes, &mut at)?;
    let root = String::from_utf8(take(bytes, &mut at, root_len)?).ok()?;
    let count = take_u32(bytes, &mut at)?;
    let mut store = Memory::new();
    for _ in 0..count {
        let name_len = take_u32(bytes, &mut at)?;
        let name = String::from_utf8(take(bytes, &mut at, name_len)?).ok()?;
        let data_len = take_u32(bytes, &mut at)?;
        let data = take(bytes, &mut at, data_len)?;
        store = store.with_input(name, data);
    }
    // Trailing bytes mean the caller and the module disagree about the
    // format, and answering anyway would answer the wrong question.
    (at == bytes.len()).then_some(Bundle { root, store })
}

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

/// Emits the bundle's root document as LaTeX.
///
/// The root alone: a host that wants every file's emission calls once per
/// file, with that file as the root. Imports and includes are transported
/// bytes in the root's own emission, exactly as the CLI writes them.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_emit(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let Some(bundle) = decode_bundle(&bytes) else {
        return result(&[]);
    };
    let mut sources = xtex_core::source::Sources::new();
    let Ok(id) = xtex_core::io::SourceLoader::load(&bundle.store, &bundle.root, None, &mut sources)
    else {
        return result(&[]);
    };
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

/// Checks the bundle's project and returns the JSON `xtex check --json` prints.
///
/// The whole pipeline: transitive `@import`, the author's own `\include` and
/// `\input` edges, the bibliography, the label inventory. One answer, equal
/// byte for byte to the CLI's for the same project on disk — the parity test
/// holds that, rather than this comment.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_check_json(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let Some(bundle) = decode_bundle(&bytes) else {
        return result(&[]);
    };
    // No `xtex.toml` reaches a browser yet; the default prefixes apply, as
    // they do for a project on disk that carries none.
    let Ok((sources, diagnostics, coverage, _)) = check_project(
        &bundle.store,
        &bundle.root,
        xtex_core::symbols::PrefixMap::default(),
    ) else {
        return result(&[]);
    };
    let mut json = String::new();
    to_json(&sources, &diagnostics, coverage, &mut json);
    result(json.as_bytes())
}

/// Emits the bundle's root and returns its source map as JSON.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_source_map(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let Some(bundle) = decode_bundle(&bytes) else {
        return result(&[]);
    };
    let mut sources = xtex_core::source::Sources::new();
    let Ok(id) = xtex_core::io::SourceLoader::load(&bundle.store, &bundle.root, None, &mut sources)
    else {
        return result(&[]);
    };
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
