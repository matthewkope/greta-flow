# Insertion fallback (PR #14)

A recording captures one delivery route before release. A focused `AXTextField`
or `AXTextArea` with an `AXSelectedTextRange` keeps the native `AXSelectedText`
route, including element, process, selection, and setter checks. An unwritable
setter, stale target, or uncertain write never triggers typing as a second route.

If the focused element instead has no usable text target, capture records the
frontmost external application's process ID. Herdr (`dev.herdr.app`) reporting
`AXWindow` is the motivating case. An unsupported/missing selection attribute may
qualify; AX timeouts, permission failures, unknown focus, and secure fields do not.
No target is captured for Greta Flow itself. Late capture context still cannot
retarget a completed recording.

Before typing and before each Unicode event, the original process must still be
frontmost, the focused element must belong to it and must not be a secure field,
and macOS `IsSecureEventInputEnabled()` must be false. This is the SDK's name for
the secure-event-input check. Unknown focus fails closed. Unicode events are
posted to the captured PID with cleared modifiers and without keyboard-layout
lookups, clipboard changes, or Return key events. Consecutive CR/LF (including
CRLF and blank paragraphs), Unicode line/paragraph separators, and control
characters become a single space. Other spacing, list markers, and Unicode text
are preserved. History/recovery retain the original, unflattened transcript.

Carbon foreground lookup and secure-input checks run on the main thread. The
foreground query reads live process state even during the delivery task, without
waiting for AppKit's running-application properties to refresh on a run-loop turn.
Microphone start is queued first; capture and delivery are dispatched outside the
session lock. The delivery task acquires that lock on the main thread, preserving
cancel, stale-result, and one-attempt rules without waiting on the main thread
while holding the lock. Existing Recent insertion also uses this dispatch.

The fallback binds an app, not a window, terminal pane, or caret within that app.
It cannot atomically freeze focus or secure-input state between a check and event
consumption. macOS provides no consumption acknowledgement. An interrupted or
partially accepted typing attempt is therefore uncertain and must not be replayed
automatically. Inspect the destination before manually recovering. Failed results
remain subject to the existing bounded recovery and configured history policy.

## Offline validation

The backend seam tests route selection, no native-to-typing fallback, foreground
and secure-input rejection at delivery, newline/control flattening, Unicode
preservation, original recovery/history text, uncertain typing, cancellation, and
stale sessions. These tests do not send real keyboard events or validate app
compatibility. Run the three commands in `CLAUDE.md`. In a clean worktree, create
the ignored `desktop/parakeet/staging/` directory before running Cargo directly;
the normal app build populates it with the Swift helper. An empty directory is
sufficient for these Rust checks, but does not constitute a runnable app bundle.

Validation on this branch: `cargo fmt --check`, locked all-target Clippy with
`-D warnings`, and locked all-target tests pass. The application has 90 passing
tests; the three example targets have 36, 72, and 51 (including shared-module
tests). Eleven added insertion tests run offline. No installed app, microphone,
or real destination application was exercised.

## Live checks for Matt

Use disposable content; the installed app has not been replaced by these checks.

- Herdr: dictate ordinary text and a formatted multiline list. Confirm insertion
  and that every line break becomes a space with no command execution.
- cmux: confirm native insertion still works with its reported `AXTextArea` and
  that moving its native caret/selection before delivery rejects the result.
- TextEdit: confirm native insertion preserves paragraphs and selected-text
  replacement, with no typing fallback when the captured target becomes stale.
- Herdr password prompt: start normal dictation, enter a password prompt before
  delivery, and confirm no transcript is typed. Also try starting at that prompt.
- Switch applications mid-dictation: confirm nothing is typed into either app
  and the original transcript is available through recovery/history policy.
- Cancel during recognition, and after a delivery failure attempt recovery;
  confirm cancellation suppresses insertion and uncertain delivery is not replayed.

No UI wording/layout changes are required for this backend implementation. The
existing dedicated recovery-surface handoff in `FIVE_CARD_REVIEW.md` remains.
