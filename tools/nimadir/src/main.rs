//! `nimadir` — the developer's tool for mini-apps.
//!
//! ```text
//! nimadir new     <name> [--lang rust|csharp|python]   a project, with its own WIT
//! nimadir build   [dir] [--key <key-file>]              its component, manifest packed in
//! nimadir run     [dir] [-- host args…]                 build, then open it in the shell
//! nimadir check   <in.wasm>                             will this shell run it?
//! nimadir keygen  <key-file>                            a new publisher key
//! nimadir pack    <in.wasm> <manifest.json> <out.wasm> [--key <key-file>]
//! nimadir inspect <in.wasm>                             what a component claims
//! ```
//!
//! The key file holds the secret: keep it out of the repository and back it
//! up. An app's identity is its publisher plus its name, and the publisher is
//! derived from the key — lose the key and updates under that identity are
//! impossible; nothing can recover it.

mod templates;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{anyhow, bail, Context, Result};
use nimadir_manifest::{Key, Manifest, SignedBy};

const USAGE: &str = "usage:
  nimadir new     <name> [--lang rust|csharp|python]
  nimadir build   [dir] [--key <key-file>]
  nimadir run     [dir] [-- <host args>…]
  nimadir check   <in.wasm>
  nimadir keygen  <key-file>
  nimadir pack    <in.wasm> <manifest.json> <out.wasm> [--key <key-file>]
  nimadir inspect <in.wasm>";

fn main() -> Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["new", name] => new(name, "rust").map(|()| ExitCode::SUCCESS),
        ["new", name, "--lang", lang] => new(name, lang).map(|()| ExitCode::SUCCESS),
        ["build"] => build(Path::new("."), None).map(|_| ExitCode::SUCCESS),
        ["build", "--key", key] => build(Path::new("."), Some(key)).map(|_| ExitCode::SUCCESS),
        ["build", dir] => build(Path::new(dir), None).map(|_| ExitCode::SUCCESS),
        ["build", dir, "--key", key] => build(Path::new(dir), Some(key)).map(|_| ExitCode::SUCCESS),
        ["run", rest @ ..] => run(rest),
        ["check", input] => check(input),
        ["keygen", file] => keygen(Path::new(file)).map(|()| ExitCode::SUCCESS),
        ["pack", input, manifest, output] => {
            pack(input, manifest, output, None).map(|()| ExitCode::SUCCESS)
        }
        ["pack", input, manifest, output, "--key", key] => {
            pack(input, manifest, output, Some(key)).map(|()| ExitCode::SUCCESS)
        }
        ["inspect", input] => inspect(input).map(|()| ExitCode::SUCCESS),
        _ => bail!("{USAGE}"),
    }
}

/// The languages a project can be in, told apart by what is in it.
#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Rust,
    CSharp,
    Python,
}

impl Lang {
    fn of(dir: &Path) -> Result<Self> {
        if dir.join("Cargo.toml").is_file() {
            return Ok(Lang::Rust);
        }
        let has_csproj = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .any(|e| e.path().extension().is_some_and(|x| x == "csproj"))
            })
            .unwrap_or(false);
        if has_csproj {
            return Ok(Lang::CSharp);
        }
        if dir.join("app.py").is_file() {
            return Ok(Lang::Python);
        }
        bail!(
            "{} is not a mini-app project: no Cargo.toml, .csproj or app.py",
            dir.display()
        )
    }
}

/// A new project: source to start from, the manifest, and its own copy of
/// the WIT.
fn new(name: &str, lang: &str) -> Result<()> {
    let ok = name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        bail!("{name:?}: a name is a-z, 0-9 and -, starting with a letter");
    }
    let dir = Path::new(name);
    if dir.exists() {
        bail!("{name} already exists");
    }
    let files: Vec<(&str, &str)> = match lang {
        "rust" => vec![
            ("Cargo.toml", templates::RUST_CARGO),
            ("src/lib.rs", templates::RUST_LIB),
        ],
        "csharp" => vec![
            ("app.csproj", templates::CSHARP_PROJECT),
            ("nuget.config", templates::CSHARP_NUGET),
            ("Program.cs", templates::CSHARP_PROGRAM),
        ],
        "python" => vec![("app.py", templates::PYTHON_APP)],
        other => bail!("{other}: the languages are rust, csharp and python"),
    };
    let common = [
        ("manifest.json", templates::MANIFEST),
        ("wit/world.wit", templates::WIT),
        (".gitignore", templates::GITIGNORE),
    ];
    for (path, template) in files.into_iter().chain(common) {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, templates::fill(template, name))
            .with_context(|| format!("write {}", path.display()))?;
    }
    println!("made {name}/ ({lang}). Next:  cd {name} && nimadir run");
    Ok(())
}

/// Build the project's component, with its manifest packed in (and signed,
/// given a key). Returns where it is.
fn build(dir: &Path, key: Option<&str>) -> Result<PathBuf> {
    let manifest_path = dir.join("manifest.json");
    let manifest: Manifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?,
    )
    .with_context(|| format!("{} is not a manifest", manifest_path.display()))?;
    let name = manifest.name.clone();
    let out = dir.join(format!("{name}.component.wasm"));

    let built = match Lang::of(dir)? {
        // The wasip2 target makes a component on its own: no other tool.
        Lang::Rust => {
            exec(
                Command::new("cargo")
                    .args(["build", "--release", "--target", "wasm32-wasip2"])
                    .current_dir(dir),
                "cargo (with `rustup target add wasm32-wasip2`)",
            )?;
            let crate_name = package_name(&dir.join("Cargo.toml"))?.replace('-', "_");
            dir.join("target/wasm32-wasip2/release")
                .join(format!("{crate_name}.wasm"))
        }
        Lang::CSharp => {
            exec(
                Command::new("dotnet")
                    .args(["build", "-c", "Release"])
                    .current_dir(dir),
                "dotnet (the .NET 10 SDK)",
            )?;
            dir.join("bin/Release/net10.0/wasi-wasm/publish")
                .join(format!("{}.wasm", name.replace('-', "_")))
        }
        Lang::Python => {
            let into = format!("{name}.component.wasm");
            exec(
                Command::new("componentize-py")
                    .args([
                        "-d",
                        "wit",
                        "-w",
                        "mini-app",
                        "componentize",
                        "app",
                        "-o",
                        &into,
                    ])
                    .current_dir(dir),
                "componentize-py (pip install componentize-py)",
            )?;
            out.clone()
        }
    };

    let bytes = std::fs::read(&built).with_context(|| format!("read {}", built.display()))?;
    let mut packed = nimadir_manifest::with_manifest(&bytes, &manifest).map_err(|e| anyhow!(e))?;
    if let Some(key) = key {
        packed = nimadir_manifest::sign(&packed, &read_key(key)?).map_err(|e| anyhow!(e))?;
    }
    std::fs::write(&out, &packed).with_context(|| format!("write {}", out.display()))?;
    report(&out.display().to_string(), &packed)?;
    Ok(out)
}

/// Build, then open the component in the shell — the window when the shell
/// has one; `-- --frames 3` and the like pass straight through to it.
fn run(rest: &[&str]) -> Result<ExitCode> {
    let (dir, host_args) = match rest {
        ["--", args @ ..] => (".", args),
        [dir, "--", args @ ..] => (*dir, args),
        [dir] => (*dir, &[][..]),
        [] => (".", &[][..]),
        _ => bail!("{USAGE}"),
    };
    let component = build(Path::new(dir), None)?;
    let status = Command::new(host())
        .arg(&component)
        .args(host_args)
        .status()
        .with_context(|| format!("run {} (set NIMADIR_HOST to the shell)", host().display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

/// Whether this shell will run the component: what it claims, then the shell
/// itself loading it — which is what says "needs a newer nimadir", or why else
/// not.
fn check(input: &str) -> Result<ExitCode> {
    inspect(input)?;
    let out = Command::new(host())
        .args([input, "--headless", "--frames", "0"])
        // Nothing to look up: a check of one file needs no registry.
        .env("NIMADIR_HOSTED_REGISTRY", "")
        .output()
        .with_context(|| format!("run {} (set NIMADIR_HOST to the shell)", host().display()))?;
    if out.status.success() {
        println!("  runs on this shell");
        Ok(ExitCode::SUCCESS)
    } else {
        let why = String::from_utf8_lossy(&out.stderr);
        println!(
            "  does not run on this shell: {}",
            why.trim().trim_start_matches("Error: ")
        );
        Ok(ExitCode::FAILURE)
    }
}

/// The shell to run: `NIMADIR_HOST`, else the `nimadir-shell` built beside
/// this tool, else whatever `nimadir-shell` is on the PATH.
fn host() -> PathBuf {
    if let Some(path) = std::env::var_os("NIMADIR_HOST") {
        return path.into();
    }
    let beside = std::env::current_exe()
        .ok()
        .map(|me| me.with_file_name(format!("nimadir-shell{}", std::env::consts::EXE_SUFFIX)));
    beside
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("nimadir-shell"))
}

fn exec(command: &mut Command, what: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not start {what}"))?;
    if !status.success() {
        bail!("the build failed ({what})");
    }
    Ok(())
}

/// `[package] name = "…"`, without a TOML parser for one line.
fn package_name(cargo_toml: &Path) -> Result<String> {
    let text = std::fs::read_to_string(cargo_toml)?;
    text.lines()
        .map(str::trim)
        .find_map(|l| {
            l.strip_prefix("name")
                .map(str::trim_start)
                .and_then(|l| l.strip_prefix('='))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .ok_or_else(|| anyhow!("{} has no package name", cargo_toml.display()))
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
        bail!("{file} is not a key made by `nimadir keygen`");
    }
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("{file} is not a key made by `nimadir keygen`"))?;
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
