"""counter-py — the same counter again, this time in Python.

The third language against one `wit/world.wit`, and the host does not know the
difference: `componentize-py` generates these bindings from the very same file
`wit-bindgen` generates the Rust ones from and `wasmtime::component::bindgen!`
generates the host ones from.

What is worth noticing here is not the counter — it is that a full CPython
interpreter is inside the component, so `datetime` below is the real standard
library running in the sandbox, not something the host handed over. The host's
only contribution is `now-millis`; formatting it is Python's own.

The price is size, and it is the point of shipping all three: the Rust counter
is ~19 KB, the C# one ~2.2 MB, this one ~20 MB. Same interface, three very
different amounts of runtime travelling with it.
"""

from datetime import datetime, timezone

import wit_world
from wit_world.imports.host_api import (
    log,
    now_millis,
    ui_button,
    ui_heading,
    ui_label,
)


class WitWorld(wit_world.WitWorld):
    """The `mini-app` world's exports. componentize-py finds this by name."""

    # On the class, not the instance: state belongs to the guest, and this is
    # the guest. It lives exactly as long as the Wasmtime `Store` does — leave
    # the app and the count is gone, the same as in the Rust and C# versions.
    count = 0

    def init(self) -> None:
        log(f"counter-py: init at host-time {now_millis()}")

    def update(self) -> None:
        ui_heading("Count (Python)", 2)
        ui_label(str(WitWorld.count))

        if ui_button("Increment"):
            WitWorld.count += 1
        if ui_button("Reset"):
            WitWorld.count = 0

        # A host capability, formatted by the interpreter that came along for
        # the ride. `now-millis` is all the host gives; `datetime` is Python's.
        stamp = datetime.fromtimestamp(now_millis() / 1000, timezone.utc)
        ui_label(f"host clock via Python datetime: {stamp:%H:%M:%S} UTC")
