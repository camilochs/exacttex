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
use xtex_core::review::{Resolution, prune_sidecar, resolve, resolve_sidecar};

/// One resolution event: the revision, what happened to it, and the bytes it
/// removed, which the sidecar records.
type Resolved = (Vec<u8>, Vec<(String, Resolution, Vec<u8>)>);
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
    let Ok((sources, diagnostics, coverage, bibliography)) = check_project(
        &bundle.store,
        &bundle.root,
        xtex_core::symbols::PrefixMap::default(),
    ) else {
        return result(&[]);
    };
    let mut json = String::new();
    to_json(&sources, &diagnostics, coverage, &bibliography, &mut json);
    result(json.as_bytes())
}

/// The project's identity inventory: every declaration with its class, its
/// site, and how many references demand it. One JSON array, sorted by name —
/// what an editor needs to draw the typed world and to say "3 references"
/// above an `@id` without guessing.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_inventory(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let Some(bundle) = decode_bundle(&bytes) else {
        return result(&[]);
    };
    let Ok(analysed) = xtex_core::project::analyse(&bundle.store, &bundle.root) else {
        return result(&[]);
    };
    let mut json = String::new();
    xtex_core::editor::inventory_to_json(&analysed.sources, &analysed.table, &mut json);
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

/// Answers a positional query over the bundle's project.
///
/// The three query exports share one input: `u32 target_len · target ·
/// u32 offset · bundle`, where `offset` is a byte offset into the named
/// file. The table is merged across the whole project, so a name declared in
/// an imported file answers a query made in the root — completions are
/// project-wide, and a definition can land in another file, which is why the
/// answer carries the file. A position past the end of the file, or a target
/// the project does not reach, returns the empty result; a position inside
/// an opaque region returns the empty result rather than a guess.
fn positional_query(
    bytes: &[u8],
    answer: impl Fn(&mut xtex_core::project::Analysed, &Memory, usize, usize) -> Option<String>,
) -> Vec<u8> {
    let mut at = 0usize;
    let Some(target) = take_text(bytes, &mut at) else {
        return Vec::new();
    };
    let Some(end) = at.checked_add(4) else {
        return Vec::new();
    };
    let Some(field) = bytes.get(at..end) else {
        return Vec::new();
    };
    let offset = u32::from_le_bytes([field[0], field[1], field[2], field[3]]) as usize;
    let Some(bundle) = decode_bundle(&bytes[end..]) else {
        return Vec::new();
    };
    let Ok(analysed) = xtex_core::project::analyse(&bundle.store, &bundle.root) else {
        return Vec::new();
    };
    let Some(position) = analysed.names.iter().position(|(_, name)| *name == target) else {
        return Vec::new();
    };
    let mut analysed = analysed;
    answer(&mut analysed, &bundle.store, position, offset).map_or_else(Vec::new, String::into_bytes)
}

/// Hover text for a position. See `positional_query` for the input.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_hover(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    result(&positional_query(
        &bytes,
        |analysed, _, position, offset| {
            let document = &analysed.documents[position];
            let found =
                xtex_core::editor::hover(&analysed.sources, document, &analysed.table, offset)?;
            let mut json = String::new();
            xtex_core::editor::hover_to_json(&found, &mut json);
            Some(json)
        },
    ))
}

/// Completions at a position. See `positional_query` for the input.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_completions(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    result(&positional_query(
        &bytes,
        |analysed, _, position, offset| {
            let document = &analysed.documents[position];
            let items = xtex_core::editor::completions(
                &analysed.sources,
                document,
                &analysed.table,
                offset,
            );
            if items.is_empty() {
                return None;
            }
            let mut json = String::new();
            xtex_core::editor::completions_to_json(&items, &mut json);
            Some(json)
        },
    ))
}

/// The declaration a position refers to. See `positional_query`.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_definition(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    result(&positional_query(
        &bytes,
        |analysed, store, position, offset| {
            let document_id = analysed.names[position].0;
            let document = &analysed.documents[position];
            // A construct's declaration first; failing that, a citation's — the
            // key's own line in the declared .bib, which the symbol table never
            // holds because a bib entry is not a construct.
            let site = xtex_core::editor::definition_site(
                &analysed.sources,
                document,
                &analysed.table,
                offset,
            );
            let (source, span) = if let Some(found) = site {
                found
            } else {
                let xtex_core::project::Analysed {
                    sources, documents, ..
                } = analysed;
                xtex_core::editor::citation_definition_site(
                    sources,
                    store,
                    &documents[position],
                    document_id,
                    offset,
                )?
            };
            let mut json = String::new();
            xtex_core::editor::definition_to_json(&analysed.sources, source, span, &mut json);
            Some(json)
        },
    ))
}

/// Emits the bundle's root under one revision view.
///
/// Input: `u32 view_len · view · bundle`, where `view` is `original`,
/// `final` or `marked`. The three views are the emitter's own
/// (`docs/revisions.md` §2); `marked` remains the one sanctioned exception
/// to no-injection (`decisions/0002`), and neither of the other two gains
/// injected markup by passing through this layer — the parity test compares
/// all three against the CLI's bytes.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_view(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut at = 0usize;
    let Some(view) = take_text(&bytes, &mut at) else {
        return result(&[]);
    };
    let view = match view.as_str() {
        "original" => xtex_core::RevisionView::Original,
        "final" => xtex_core::RevisionView::Final,
        "marked" => xtex_core::RevisionView::Marked,
        _ => return result(&[]),
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
    let mut out = Vec::new();
    match xtex_core::emit_view(&sources, &document, view, &mut out) {
        Ok(()) => result(&out),
        Err(_) => result(&[]),
    }
}

/// Accepts, rejects or prunes revisions in the bundle's root.
///
/// Input: `u32 action_len · action · u32 id_len · id · u32 by_len · by ·
/// u32 at_len · at · u32 sidecar_len · sidecar · bundle`. `action` is
/// `accept`, `reject`, `accept-all` or `prune`; `id` is empty except for
/// `accept` and `reject`. `by` and `at` are the reviewer and the timestamp,
/// supplied by the host because this module deliberately cannot ask a clock.
/// `sidecar` is the `.xtexrev` content, empty when none exists.
///
/// The answer is two length-prefixed fields: the rewritten root, then the
/// updated sidecar (empty when none was supplied). A sidecar updated here is
/// read by the CLI without complaint, and the reverse — the parity test
/// crosses them both ways.
///
/// # Safety
///
/// `pointer` must point at `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xtex_revise(pointer: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { input(pointer, len) };
    let mut at = 0usize;
    let (Some(action), Some(id), Some(by), Some(stamp)) = (
        take_text(&bytes, &mut at),
        take_text(&bytes, &mut at),
        take_text(&bytes, &mut at),
        take_text(&bytes, &mut at),
    ) else {
        return result(&[]);
    };
    let Some(sidecar) = take_bytes(&bytes, &mut at) else {
        return result(&[]);
    };
    let Some(bundle) = decode_bundle(&bytes[at..]) else {
        return result(&[]);
    };
    let Some(source) = bundle_file(&bundle, &bundle.root) else {
        return result(&[]);
    };

    let outcome: Result<Resolved, ()> = match action.as_str() {
        "accept" | "reject" => {
            let resolution = if action == "accept" {
                Resolution::Accept
            } else {
                Resolution::Reject
            };
            resolve(&source, &id, resolution)
                .map(|(rewritten, removed)| (rewritten, vec![(id.clone(), resolution, removed)]))
                .map_err(|_| ())
        }
        "accept-all" => {
            let mut current = source.clone();
            let mut events = Vec::new();
            loop {
                let Some(next) = xtex_core::review::revision_ids(&current).into_iter().next()
                else {
                    break Ok((current, events));
                };
                match resolve(&current, &next, Resolution::Accept) {
                    Ok((rewritten, removed)) => {
                        current = rewritten;
                        events.push((next, Resolution::Accept, removed));
                    }
                    Err(_) => break Err(()),
                }
            }
        }
        "prune" => {
            if sidecar.is_empty() {
                return result(&[]);
            }
            return match prune_sidecar(&sidecar, &source, &by, &stamp) {
                Ok(pruned) => result(&pair(&source, &pruned)),
                Err(_) => result(&[]),
            };
        }
        _ => return result(&[]),
    };
    let Ok((rewritten, events)) = outcome else {
        return result(&[]);
    };
    let updated_sidecar = if sidecar.is_empty() {
        Vec::new()
    } else {
        let mut records = sidecar;
        for (event_id, resolution, removed) in &events {
            match resolve_sidecar(&records, event_id, *resolution, &by, &stamp, removed) {
                Ok(next) => records = next,
                Err(_) => return result(&[]),
            }
        }
        records
    };
    result(&pair(&rewritten, &updated_sidecar))
}

/// Reads one length-prefixed byte field from a buffer.
fn take_bytes(bytes: &[u8], at: &mut usize) -> Option<Vec<u8>> {
    let end = at.checked_add(4)?;
    let field = bytes.get(*at..end)?;
    let field_len = u32::from_le_bytes([field[0], field[1], field[2], field[3]]) as usize;
    *at = end;
    let field_end = at.checked_add(field_len)?;
    let out = bytes.get(*at..field_end)?.to_vec();
    *at = field_end;
    Some(out)
}

/// The bundle file stored under `name`, if any.
fn bundle_file(bundle: &Bundle, name: &str) -> Option<Vec<u8>> {
    let mut sources = xtex_core::source::Sources::new();
    let id = xtex_core::io::SourceLoader::load(&bundle.store, name, None, &mut sources).ok()?;
    sources.get(id).map(|source| source.bytes().to_vec())
}

/// Two length-prefixed fields in one result.
fn pair(first: &[u8], second: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + first.len() + second.len());
    out.extend_from_slice(&u32::try_from(first.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(first);
    out.extend_from_slice(
        &u32::try_from(second.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    out.extend_from_slice(second);
    out
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
