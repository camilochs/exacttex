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

/// Plans a rename across the bundle's project, honesty included.
///
/// Input: `u32 from_len · from · u32 to_len · to · bundle`. The answer is
/// JSON with two lists: `edits`, each with file, position and replacement;
/// and `untouched` — every occurrence left alone because it sits in opaque
/// text. An editor that silently renames 12 of 14 places is worse than one
/// that renames none, so the untouched places are part of the answer.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_rename_plan(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut at = 0usize;
    let Some(from) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(to) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(bundle) = decode_bundle(&bytes[at..]) else {
        return result(&[]);
    };
    let Ok((sources, documents, _)) = xtex_core::project::load_imports(&bundle.store, &bundle.root)
    else {
        return result(&[]);
    };
    let plan = xtex_core::rename::plan(&sources, &documents, &from, &to);
    let mut json = String::new();
    xtex_core::rename::to_json(&sources, &plan, &mut json);
    result(json.as_bytes())
}

/// Applies a rename to one file of the bundle and returns its new bytes.
///
/// Input: `u32 from_len · from · u32 to_len · to · u32 target_len · target ·
/// bundle`. The plan is computed over the whole project — an edit's offsets
/// are only meaningful against every reachable file — and applied to the one
/// named file. A host rewrites its files one call at a time, the way
/// `xtex rename` writes them one at a time. A rename that touches nothing
/// returns the file unchanged, and a target the project does not reach
/// returns the empty result.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_rename_apply(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut at = 0usize;
    let Some(from) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(to) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(target) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(bundle) = decode_bundle(&bytes[at..]) else {
        return result(&[]);
    };
    let Ok((sources, documents, names)) =
        xtex_core::project::load_imports(&bundle.store, &bundle.root)
    else {
        return result(&[]);
    };
    let Some((id, _)) = names.iter().find(|(_, name)| *name == target) else {
        return result(&[]);
    };
    let plan = xtex_core::rename::plan(&sources, &documents, &from, &to);
    let Some(original) = sources.get(*id).map(|source| source.bytes().to_vec()) else {
        return result(&[]);
    };
    result(&xtex_core::rename::apply(&original, *id, &plan))
}

/// Reads one length-prefixed UTF-8 text from a buffer.
fn take_text(bytes: &[u8], at: &mut usize) -> Option<String> {
    let end = at.checked_add(4)?;
    let field = bytes.get(*at..end)?;
    let text_len = u32::from_le_bytes([field[0], field[1], field[2], field[3]]) as usize;
    *at = end;
    let text_end = at.checked_add(text_len)?;
    let text = String::from_utf8(bytes.get(*at..text_end)?.to_vec()).ok()?;
    *at = text_end;
    Some(text)
}

/// Translates a TeX log against the bundle's root, with blame resolved.
///
/// The input is two length-prefixed texts followed by a bundle:
///
/// ```text
/// u32 stderr_len   stderr (UTF-8; the engine's console output, may be empty)
/// u32 log_len      log (UTF-8; the .log file's content, may be empty)
/// bundle           as for every other operation
/// ```
///
/// The answer is JSON: the engine's own words unchanged, the emitted line,
/// the author's position where a map segment supports one, the entity where
/// a declaration supplies the evidence, and `"unresolved"` blame otherwise —
/// never a guess. A browser is exactly where a confident wrong attribution
/// does the most damage, because the user cannot check it against a terminal.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_blame(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut at = 0usize;
    let Some(stderr) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(log) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(bundle) = decode_bundle(&bytes[at..]) else {
        return result(&[]);
    };

    let mut sources = xtex_core::source::Sources::new();
    let Ok(id) = xtex_core::io::SourceLoader::load(&bundle.store, &bundle.root, None, &mut sources)
    else {
        return result(&[]);
    };
    let document = parse(&sources, id);
    let mut table = xtex_core::symbols::SymbolTable::new();
    table.merge(&sources, &document);
    let Ok(emission) = emit_with_map(&sources, &document) else {
        return result(&[]);
    };

    // The emitted name the engine reports against: the root with `.tex`, as
    // the CLI writes it under `build/`.
    let emitted_name = bundle
        .root
        .rsplit('/')
        .next()
        .unwrap_or(&bundle.root)
        .replace(".xtex", ".tex");
    let stderr = (!stderr.is_empty()).then_some(stderr);
    let log = (!log.is_empty()).then_some(log);
    let records = xtex_core::blame::merge_records(stderr.as_deref(), log.as_deref(), &emitted_name);
    let translated = xtex_core::blame::translate(&records, &emission, &sources, &document, &table);
    let mut json = String::new();
    xtex_core::blame::to_json(&translated, &mut json);
    result(json.as_bytes())
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
