#!/usr/bin/env python3
"""Rewrite mini-apps/future/wit/world.wit from wit/world.wit: the same contract
one minor version on, with one function added that no host has. Run after
every change to wit/world.wit. With --check it writes nothing, and fails if
the fixture is not what it would write (the suite runs that)."""

import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
src = (root / "wit/world.wit").read_text(encoding="utf-8")

version = re.search(r"^package nimadir:shell@(\d+)\.(\d+)\.(\d+);", src, re.M)
if not version:
    raise SystemExit("wit/world.wit has no `package nimadir:shell@x.y.z;` line")
major, minor = int(version.group(1)), int(version.group(2))
newer = f"package nimadir:shell@{major}.{minor + 1}.0;"

marker = "    request-frame: func(after-ms: u32);\n"
if marker not in src:
    raise SystemExit("wit/world.wit no longer has request-frame to anchor on")
extra = marker + """
    /// Exists only in this fixture's copy of the contract: what a capability
    /// added after this host was built looks like to it.
    from-the-future: func();
"""
out = src.replace(version.group(0), newer, 1).replace(marker, extra, 1)
header = (
    "// A copy of wit/world.wit one minor version on, with one function added —\n"
    "// standing in for a contract newer than the host's. Test fixture only;\n"
    "// regenerate with tools/sync-fixture-wit.py.\n"
)
fixture = root / "mini-apps/future/wit/world.wit"
if "--check" in sys.argv[1:]:
    if fixture.read_text(encoding="utf-8") != header + out:
        raise SystemExit("mini-apps/future/wit/world.wit is out of date: run tools/sync-fixture-wit.py")
    print("in step")
else:
    fixture.write_text(header + out, encoding="utf-8")
    print(newer)
