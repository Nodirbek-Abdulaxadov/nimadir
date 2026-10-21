#!/usr/bin/env python3
"""Build the hosted registry: each listed app's component, and an apps.json
that names it by absolute URL and pins it by SHA-256.

    tools/publish_registry.py <base-url> [out-dir]      (default: dist/registry)

Names and descriptions come from the repository's apps.json. The store is left
out — every shell carries its own — and so is any component not yet built. The
output is a plain directory of static files: serve it from anywhere, GitHub
Pages included (.github/workflows/registry.yml).
"""

import hashlib
import json
import pathlib
import subprocess
import sys


def inspect(root: pathlib.Path, component: pathlib.Path):
    """The component's publisher, if it is signed, and its manifest's
    version, if it has one — read by nimadir. The publisher lets the registry
    require it (a copy signed by anyone else is refused); the version is
    shown on the app's card (`app-info`)."""
    pack = root / "target" / "release" / "nimadir"
    if not pack.exists():
        return None, None
    out = subprocess.run([str(pack), "inspect", str(component)], capture_output=True, text=True)
    signer, version = None, None
    for line in out.stdout.splitlines():
        line = line.strip()
        if line.startswith("signed by "):
            signer = line[len("signed by "):]
        # `manifest  <name> <version>`, or `manifest  none`.
        words = line.split()
        if len(words) == 3 and words[0] == "manifest":
            version = words[2]
    return signer, version


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    base = sys.argv[1].rstrip("/")
    out = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else "dist/registry")
    root = pathlib.Path(__file__).resolve().parent.parent

    listed = json.loads((root / "apps.json").read_text(encoding="utf-8"))["apps"]
    out.mkdir(parents=True, exist_ok=True)

    published = []
    for app in listed:
        name = app["name"]
        if name == "store":
            continue
        component = root / app["source"]
        if not component.is_file():
            print(f"skipping {name}: {app['source']} has not been built", file=sys.stderr)
            continue
        data = component.read_bytes()
        file = f"{name}.component.wasm"
        (out / file).write_bytes(data)
        entry = {
            "name": name,
            "source": f"{base}/{file}",
            "description": app.get("description", ""),
            "sha256": hashlib.sha256(data).hexdigest(),
        }
        signer, version = inspect(root, component)
        if signer:
            entry["publisher"] = signer
        if version:
            entry["version"] = version
        published.append(entry)

    catalogue = json.dumps({"apps": published}, indent=2, ensure_ascii=False)
    (out / "apps.json").write_text(catalogue + "\n", encoding="utf-8")
    print(f"{len(published)} apps -> {out / 'apps.json'}")


if __name__ == "__main__":
    main()
