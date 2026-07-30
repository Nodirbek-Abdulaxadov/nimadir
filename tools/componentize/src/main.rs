//! Core wasm module  ->  WASM component.
//!
//! `wit-bindgen` embeds the world's type information into the guest's core
//! module as custom sections. This tool reads those sections and encodes a
//! Component-Model component around the module. Because our `mini-app` world
//! imports only `nimadir:shell/host-api` (no WASI), no adapter module is needed.
//!
//! This is exactly what `wasm-tools component new` does — in ~15 lines, using
//! the `wit-component` library — so the repo needs no external CLI on PATH.
//!
//!   componentize <core-in.wasm> <component-out.wasm>

use anyhow::{Context, Result};
use wit_component::ComponentEncoder;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .context("usage: componentize <core-in.wasm> <component-out.wasm>")?;
    let output = args
        .next()
        .context("usage: componentize <core-in.wasm> <component-out.wasm>")?;

    let core = std::fs::read(&input).with_context(|| format!("read {input}"))?;

    let component = ComponentEncoder::default()
        .module(&core)
        .context("load core module (was it built with wit-bindgen?)")?
        .encode()
        .context("encode component (unresolved imports? missing adapter?)")?;

    std::fs::write(&output, &component).with_context(|| format!("write {output}"))?;
    println!(
        "componentized {input} -> {output} ({} bytes)",
        component.len()
    );
    Ok(())
}
