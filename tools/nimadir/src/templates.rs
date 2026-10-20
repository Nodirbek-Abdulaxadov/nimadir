//! What `nimadir new` writes. Every project gets its own copy of the WIT, so
//! nothing it builds against lives outside it — no checkout of this repository,
//! no package to fetch.

/// The contract this tool was built with — the one `nimadir new` vendors.
pub const WIT: &str = include_str!("../../../wit/world.wit");

pub const RUST_CARGO: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

# Android and the desktop load the app as a component; `nimadir build` makes it.
[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.60.0"

[profile.release]
opt-level = "s"
lto = true
strip = true
"#;

pub const RUST_LIB: &str = r#"//! {name} — a nimadir mini-app.
//!
//! `wit/world.wit` is everything it can ask the shell for. `nimadir build`
//! turns this crate into `{name}.component.wasm`; `nimadir run` builds it and
//! opens it.

wit_bindgen::generate!({
    world: "mini-app",
    path: "wit",
});

use std::sync::atomic::{AtomicU32, Ordering};

use nimadir::shell::host_api::{ui_button, ui_heading, ui_label};

/// App state lives here, in the app. Leaving the app ends it — keep what has
/// to outlast a visit with `storage-set` (sign the app and ask for "storage"
/// in manifest.json).
static TAPS: AtomicU32 = AtomicU32::new(0);

struct App;

impl Guest for App {
    fn init() {}

    /// Called every frame the shell draws: describe the UI, react to taps.
    fn update() {
        ui_heading("{name}", 1);
        ui_label("Edit src/lib.rs, then run: nimadir run");
        if ui_button("Tap me") {
            TAPS.fetch_add(1, Ordering::Relaxed);
        }
        ui_label(&format!("Tapped {} times.", TAPS.load(Ordering::Relaxed)));
    }
}

export!(App);
"#;

pub const CSHARP_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">

  <PropertyGroup>
    <!-- A reactor component (exports init/update, no Main). -->
    <OutputType>Library</OutputType>
    <TargetFramework>net10.0</TargetFramework>
    <RootNamespace>App</RootNamespace>
    <AssemblyName>{asm}</AssemblyName>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <RuntimeIdentifier>wasi-wasm</RuntimeIdentifier>
    <UseAppHost>false</UseAppHost>
    <PublishTrimmed>true</PublishTrimmed>
    <InvariantGlobalization>true</InvariantGlobalization>
    <SelfContained>true</SelfContained>
  </PropertyGroup>

  <ItemGroup>
    <PackageReference Include="BytecodeAlliance.Componentize.DotNet.Wasm.SDK" Version="0.8.0-preview00011" />
    <!-- The NativeAOT-LLVM cross-compiler for whatever machine builds this. -->
    <PackageReference Include="runtime.$(NETCoreSdkPortableRuntimeIdentifier).microsoft.dotnet.ilcompiler.llvm" Version="10.0.0-rc.1.26306.1" />
  </ItemGroup>

  <ItemGroup>
    <Wit Include="wit/world.wit" World="mini-app" />
  </ItemGroup>

</Project>
"#;

pub const CSHARP_NUGET: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="nuget.org" value="https://api.nuget.org/v3/index.json" />
    <!-- NativeAOT-LLVM is published here, not on nuget.org. -->
    <add key="dotnet-experimental" value="https://pkgs.dev.azure.com/dnceng/public/_packaging/dotnet-experimental/nuget/v3/index.json" />
  </packageSources>
</configuration>
"#;

pub const CSHARP_PROGRAM: &str = r#"// {name} — a nimadir mini-app in C#. `wit/world.wit` is everything it can ask
// the shell for; `nimadir build` makes {name}.component.wasm (needs .NET 10).

using MiniAppWorld;
using static MiniAppWorld.wit.Imports.nimadir.shell.v1_0_0.IHostApiImports;

public class MiniAppWorldExportsImpl : IMiniAppWorldExports
{
    private static int _taps;

    public static void Init() { }

    public static void Update()
    {
        UiHeading("{name}", 1);
        UiLabel("Edit Program.cs, then run: nimadir run");
        if (UiButton("Tap me"))
        {
            _taps++;
        }
        UiLabel($"Tapped {_taps} times.");
    }
}
"#;

pub const PYTHON_APP: &str = r#""""{name} — a nimadir mini-app in Python.

`wit/world.wit` is everything it can ask the shell for. `nimadir build` makes
{name}.component.wasm (needs `pip install componentize-py`).
"""

import wit_world
from wit_world.imports.host_api import ui_button, ui_heading, ui_label


class WitWorld(wit_world.WitWorld):
    taps = 0

    def init(self) -> None:
        pass

    def update(self) -> None:
        ui_heading("{name}", 1)
        ui_label("Edit app.py, then run: nimadir run")
        if ui_button("Tap me"):
            WitWorld.taps += 1
        ui_label(f"Tapped {WitWorld.taps} times.")
"#;

pub const MANIFEST: &str = r#"{
  "name": "{name}",
  "version": "0.1.0",
  "description": "",
  "permissions": []
}
"#;

pub const GITIGNORE: &str = "/target\n/bin\n/obj\n__pycache__/\n*.component.wasm\n";

/// Fill in a template.
pub fn fill(template: &str, name: &str) -> String {
    template
        .replace("{name}", name)
        .replace("{asm}", &name.replace('-', "_"))
}
