# winit 0.30.13, patched for nimadir

The crate as published on crates.io, with one change, so that text typed on
an Android phone's own keyboard reaches the app (`docs/ROADMAP.md`, S9-2).

GameActivity's soft keyboard edits a text of its own and sends all of it on
each change (`android_activity::input::InputEvent::TextEvent`). Upstream
0.30 logs it as unknown and drops it, so only the ASCII a keyboard also sends
as key events arrived: no predictions, no Cyrillic, no `oʻ`.

`src/platform_impl/android/mod.rs`, in `handle_input_event`: each
`TextEvent` is compared with the last (`ime_changes`, `ImeSeen`). Committed
text the keyboard deleted becomes Backspace presses, committed text it added
`WindowEvent::Ime(Ime::Commit(..))`, and the word it is composing
`Ime::Preedit(..)`. Spans count UTF-16 units, as Java does (`ime_split`,
`utf16_to_byte`, at the end of the file). The keyboard's action key (✓, Go,
Search) comes as a `TextAction`, also dropped upstream; it is handed on as an
Enter press.

The keyboard's text is cleared (`ime_forget`) only when a field is left or
another taken — `Window::set_ime_allowed`, which egui turns off and on again
when focus moves — and after Enter, never while typing. GameTextInput restarts
the keyboard whenever its text is set, which loses its predictions, and the
reset is posted to the UI thread, so a key committed before it lands would be
lost. A change that still shows the text being cleared is ignored until the
reset lands. Every line added is marked `nimadir:`.

Drop the patch when eframe moves to a winit that hands this on itself.
