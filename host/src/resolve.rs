//! Address classification — the router that sits in front of navigation.
//!
//! The shell now has two kinds of destination: a **WASM component** (the
//! sandboxed native path, rendered by egui) and an **old-style web page** (the
//! webview). Something has to decide which one an address is *before*
//! `Shell` can navigate to it, and that decision lives here and nowhere else.
//!
//! The rule, in order:
//!
//!   1. a local path        -> classified by extension, with no network at all
//!   2. a URL ending `.wasm` -> a component; the address already said so
//!   3. any other URL        -> fetched once, classified by `Content-Type`
//!
//! Case 3 fetches instead of sending a `HEAD` on purpose. The request a `HEAD`
//! would save is the request the WASM path has to make anyway, so probing with
//! `HEAD` first would double the round trips on the *common* case to save one on
//! the rarer one. The body comes back attached to the classification instead, so
//! a component is never downloaded twice.

use std::io::Read;

use anyhow::{bail, Context, Result};

/// What lives at an address — and, for a component, the bytes to run.
pub enum Target {
    /// A WASM component, already fetched. Rendered natively (egui + Wasmtime).
    WasmApp(Vec<u8>),
    /// An old-style web page. Belongs to the webview module; until that exists,
    /// the shell routes here and draws a placeholder.
    WebPage,
}

/// The four bytes every WASM module and component starts with.
const WASM_MAGIC: &[u8] = b"\0asm";

/// Decide what `src` is, fetching it if that is the only way to know.
pub fn classify(src: &str) -> Result<Target> {
    if is_remote(src) {
        remote(src)
    } else {
        local(src)
    }
}

/// Whether an address is something we fetch over the network rather than read
/// off disk. Deliberately strict: a bare `example.com` is *not* a URL here, so
/// it falls through to the local branch and gets a message saying so.
pub fn is_remote(src: &str) -> bool {
    src.starts_with("http://") || src.starts_with("https://")
}

/// Local paths are classified by extension only — no I/O beyond the read the
/// component itself needs.
fn local(path: &str) -> Result<Target> {
    match extension(path).as_deref() {
        Some("wasm") => {
            let bytes = std::fs::read(path).with_context(|| format!("read file {path}"))?;
            Ok(Target::WasmApp(bytes))
        }
        Some("html") | Some("htm") => Ok(Target::WebPage),
        _ => bail!(
            "cannot tell what {path} is — expected a .wasm component or an .html page. \
             (A web address needs its http:// or https:// prefix.)"
        ),
    }
}

/// Remote addresses are fetched once; the response says what they are.
fn remote(url: &str) -> Result<Target> {
    // Checked before the request so a `.wasm` address is trusted even when the
    // server labels it badly — see the octet-stream note below.
    let named_wasm = extension(url_path(url)).as_deref() == Some("wasm");

    let resp = ureq::get(url).call().with_context(|| format!("GET {url}"))?;

    // Headers must be read before `into_reader` consumes the response. Only the
    // mime type matters; `; charset=utf-8` and friends are dropped.
    let declared = resp
        .header("Content-Type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    if declared == "text/html" || declared == "application/xhtml+xml" {
        // The page is the webview's to fetch; drop this body on the floor.
        return Ok(Target::WebPage);
    }

    let mut body = Vec::new();
    resp.into_reader()
        .read_to_end(&mut body)
        .with_context(|| format!("read body of {url}"))?;

    // `application/wasm` is the registered type, but plenty of static hosts
    // serve .wasm as `application/octet-stream` or with no type at all. The
    // bytes are already here and they cannot lie, so they settle it.
    if declared == "application/wasm" || named_wasm || body.starts_with(WASM_MAGIC) {
        return Ok(Target::WasmApp(body));
    }

    bail!(
        "{url} is neither a WASM component nor a web page (Content-Type: {})",
        if declared.is_empty() {
            "none"
        } else {
            &declared
        }
    );
}

/// The lowercase extension of the last path segment, if it has one.
/// `app.component.wasm` -> `wasm`.
fn extension(path: &str) -> Option<String> {
    let last = path.rsplit(['/', '\\']).next()?;
    let (_, ext) = last.rsplit_once('.')?;
    (!ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// The path part of a URL, so `…/app.wasm?v=2#top` still ends in `.wasm`.
fn url_path(url: &str) -> &str {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let end = after_scheme.find(['?', '#']).unwrap_or(after_scheme.len());
    &after_scheme[..end]
}
