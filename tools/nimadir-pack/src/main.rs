//! `nimadir-pack` — give a component its manifest, and sign it (ADR-1).
//!
//! ```text
//! nimadir-pack keygen  <key-file>                         a new publisher key
//! nimadir-pack pack    <in.wasm> <manifest.json> <out.wasm> [--key <key-file>]
//! nimadir-pack inspect <in.wasm>                          what a component claims
//! ```
//!
//! The key file holds the secret: keep it out of the repository and back it
//! up. An app's identity is its publisher plus its name, and the publisher is
//! derived from the key — lose the key and updates under that identity are
//! impossible; nothing can recover it.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use nimadir_manifest::{Key, Manifest, SignedBy};

const USAGE: &str = "usage:
  nimadir-pack keygen  <key-file>
  nimadir-pack pack    <in.wasm> <manifest.json> <out.wasm> [--key <key-file>]
  nimadir-pack inspect <in.wasm>";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["keygen", file] => keygen(Path::new(file)),
        ["pack", input, manifest, output] => pack(input, manifest, output, None),
        ["pack", input, manifest, output, "--key", key] => pack(input, manifest, output, Some(key)),
        ["inspect", input] => inspect(input),
        _ => bail!("{USAGE}"),
    }
}

fn keygen(file: &Path) -> Result<()> {
    if file.exists() {
        bail!("{} exists — a key is never overwritten", file.display());
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| anyhow!("no randomness: {e}"))?;
    let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(file, format!("{hex}\n"))
        .with_context(|| format!("write {}", file.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))?;
    }
    let key = Key::from_bytes(&seed);
    println!(
        "publisher {}",
        nimadir_manifest::publisher_id(key.verifying_key().as_bytes())
    );
    Ok(())
}

fn read_key(file: &str) -> Result<Key> {
    let text = std::fs::read_to_string(file).with_context(|| format!("read {file}"))?;
    let hex = text.trim();
    if hex.len() != 64 {
        bail!("{file} is not a key made by `nimadir-pack keygen`");
    }
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("{file} is not a key made by `nimadir-pack keygen`"))?;
    }
    Ok(Key::from_bytes(&seed))
}

fn pack(input: &str, manifest: &str, output: &str, key: Option<&str>) -> Result<()> {
    let bytes = std::fs::read(input).with_context(|| format!("read {input}"))?;
    let manifest: Manifest = serde_json::from_str(
        &std::fs::read_to_string(manifest).with_context(|| format!("read {manifest}"))?,
    )
    .with_context(|| format!("{manifest} is not a manifest"))?;

    let mut packed = nimadir_manifest::with_manifest(&bytes, &manifest).map_err(|e| anyhow!(e))?;
    if let Some(key) = key {
        packed = nimadir_manifest::sign(&packed, &read_key(key)?).map_err(|e| anyhow!(e))?;
    }
    std::fs::write(output, &packed).with_context(|| format!("write {output}"))?;
    report(output, &packed)
}

fn inspect(input: &str) -> Result<()> {
    let bytes = std::fs::read(input).with_context(|| format!("read {input}"))?;
    report(input, &bytes)
}

fn report(name: &str, bytes: &[u8]) -> Result<()> {
    let found = nimadir_manifest::inspect(bytes).map_err(|e| anyhow!("{name}: {e}"))?;
    println!("{name}: {} bytes", bytes.len());
    match &found.manifest {
        Some(m) => println!("  manifest  {} {}", m.name, m.version),
        None => println!("  manifest  none"),
    }
    match &found.signed_by {
        SignedBy::Publisher(p) => println!("  signed by {p}"),
        SignedBy::Nobody => {
            println!("  unsigned  (runs as unverified: nothing it keeps outlives a visit)")
        }
    }
    if let Some(id) = found.identity() {
        println!("  identity  {id}");
    }
    Ok(())
}
