//! Target-bound Accessibility insertion, with capture-time keystroke fallback for
//! focused elements without a usable text target. Never falls back after an AX
//! failure or stale target. Keystrokes require the original foreground process and
//! non-secure input; line breaks become spaces. Never sends Return or uses paste.
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::event::{CGEvent, CGEventFlags};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use std::ffi::c_void;

#[repr(C)]
#[derive(Default)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}
#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetFrontProcess(process: *mut ProcessSerialNumber) -> i16;
    fn GetProcessPID(process: *const ProcessSerialNumber, pid: *mut i32) -> i32;
    fn IsSecureEventInputEnabled() -> u8;
}

type AX = *const c_void;
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AX;
    fn AXUIElementCopyAttributeValue(e: AX, key: CFStringRef, out: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(e: AX, key: CFStringRef, value: CFTypeRef) -> i32;
    fn AXUIElementIsAttributeSettable(e: AX, key: CFStringRef, out: *mut u8) -> i32;
    fn AXUIElementGetPid(e: AX, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(e: AX, seconds: f32) -> i32;
}

#[derive(Debug)]
struct Object(CFTypeRef);
// AX objects are immutable CF proxy references; all operations here are synchronous.
unsafe impl Send for Object {}
unsafe impl Sync for Object {}
impl Clone for Object {
    fn clone(&self) -> Self {
        unsafe { Self(CFRetain(self.0)) }
    }
}
impl Drop for Object {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        unsafe { CFEqual(self.0, other.0) != 0 }
    }
}
impl Object {
    fn get(&self, key: &str) -> Result<Self, String> {
        let mut value = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(
                self.0,
                CFString::new(key).as_concrete_TypeRef(),
                &mut value,
            )
        };
        if status == 0 && !value.is_null() {
            Ok(Self(value))
        } else {
            Err(format!(
                "Accessibility attribute unavailable ({key}, {status})"
            ))
        }
    }
}

// A small seam keeps capture/delivery decisions testable without querying macOS
// or sending events. Missing attributes differ from an AX timeout/permission error.
trait Backend {
    type Native: Clone + std::fmt::Debug;
    fn frontmost_pid(&self) -> Option<i32>;
    fn own_pid(&self) -> i32;
    fn secure_input(&self) -> bool;
    fn focus(&self) -> Result<Focus<Self::Native>, String>;
    fn validate_native(&self, native: &Self::Native) -> Result<(), String>;
    fn write_native(&self, native: &Self::Native, text: &str) -> Result<(), String>;
    fn type_text(&self, pid: i32, text: &str) -> Result<(), String>;
}

struct Focus<N> {
    pid: i32,
    secure: bool,
    native: Option<N>,
}

#[derive(Clone, Debug)]
enum Destination<N> {
    Accessibility(N),
    Keystrokes(i32),
}

impl<N: Clone + std::fmt::Debug> Destination<N> {
    fn capture(backend: &impl Backend<Native = N>) -> Result<(Self, i32), String> {
        let frontmost = backend.frontmost_pid();
        let focus = backend.focus()?;
        let pid = focus.pid;
        if pid <= 0 || pid == backend.own_pid() {
            return Err("No external destination; use recovery text".into());
        }
        if focus.secure {
            return Err("Focus an editable, non-password text field".into());
        }
        let destination = match focus.native {
            Some(native) => Self::Accessibility(native),
            None => {
                if frontmost != Some(pid) || external_frontmost(backend)? != pid {
                    return Err("Destination changed during capture".into());
                }
                if backend.secure_input() {
                    return Err("Secure input is active; transcript retained for recovery".into());
                }
                Self::Keystrokes(pid)
            }
        };
        Ok((destination, pid))
    }

    fn validate(&self, backend: &impl Backend<Native = N>) -> Result<(), String> {
        match self {
            Self::Accessibility(native) => backend.validate_native(native),
            Self::Keystrokes(pid) => validate_keystrokes(backend, *pid),
        }
    }

    fn insert(&self, backend: &impl Backend<Native = N>, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        self.validate(backend)?;
        match self {
            Self::Accessibility(native) => backend.write_native(native, text),
            Self::Keystrokes(pid) => backend.type_text(*pid, &flatten_newlines(text)),
        }
    }
}

fn external_frontmost(backend: &impl Backend) -> Result<i32, String> {
    backend
        .frontmost_pid()
        .filter(|pid| *pid > 0 && *pid != backend.own_pid())
        .ok_or_else(|| "No external foreground destination; use recovery text".into())
}

fn validate_keystrokes(backend: &impl Backend, pid: i32) -> Result<(), String> {
    if external_frontmost(backend)? != pid {
        return Err("Destination changed; transcript retained for recovery".into());
    }
    let focus = backend.focus()?;
    if focus.pid != pid || focus.secure || backend.secure_input() {
        return Err("Destination unavailable or secure; transcript retained for recovery".into());
    }
    // AX queries can block; recheck the foreground process after querying focus.
    if external_frontmost(backend)? != pid {
        return Err("Destination changed; transcript retained for recovery".into());
    }
    Ok(())
}

fn flatten_newlines(text: &str) -> String {
    // Collapse each run of line separators (including CRLF/blank paragraphs) to
    // one space. Other spacing and Unicode remain intact. Control characters are
    // also neutralized so a literal transcript cannot send terminal controls.
    let mut flattened = String::with_capacity(text.len());
    let mut separator = false;
    for ch in text.chars() {
        if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') {
            if !separator {
                flattened.push(' ');
            }
            separator = true;
        } else {
            flattened.push(ch);
            separator = false;
        }
    }
    flattened
}

// CGEvent Unicode text is limited to short strings. Count UTF-16 code units,
// not UTF-8 bytes or scalar values, so supplementary characters remain intact.
fn unicode_chunks(mut text: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let mut units = 0;
        let mut end = 0;
        for (index, ch) in text.char_indices() {
            units += ch.len_utf16();
            if units > 20 {
                break;
            }
            end = index + ch.len_utf8();
        }
        let (chunk, rest) = text.split_at(end);
        text = rest;
        Some(chunk)
    })
}

#[derive(Clone, Debug)]
pub struct Target {
    destination: Destination<NativeTarget>,
    pub pid: i32,
}
impl Target {
    pub fn capture() -> Result<Self, String> {
        require_main_thread()?;
        let (destination, pid) = Destination::capture(&Mac)?;
        Ok(Self { destination, pid })
    }
    pub fn validate(&self) -> Result<(), String> {
        require_main_thread()?;
        self.destination.validate(&Mac)
    }
    pub fn insert(&self, text: &str) -> Result<(), String> {
        require_main_thread()?;
        self.destination.insert(&Mac, text)
    }
}

#[derive(Clone, Debug)]
struct NativeTarget {
    element: Object,
    selection: Object,
    pid: i32,
}
impl NativeTarget {
    fn validate(&self) -> Result<(), String> {
        let focus = Mac.focus()?;
        let now = focus
            .native
            .filter(|_| !focus.secure)
            .ok_or("Destination is no longer a non-password text field")?;
        if self.pid != now.pid || self.element != now.element || self.selection != now.selection {
            return Err("Destination or cursor changed; transcript retained for recovery".into());
        }
        Ok(())
    }
    pub fn insert(&self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        self.validate()?;
        let key = CFString::new("AXSelectedText");
        let mut writable = 0;
        if unsafe {
            AXUIElementIsAttributeSettable(self.element.0, key.as_concrete_TypeRef(), &mut writable)
        } != 0
            || writable == 0
        {
            return Err(
                "Destination does not support safe native insertion; use recovery text".into(),
            );
        }
        // Write directly to the captured element, not whichever app receives a key event.
        let status = unsafe {
            AXUIElementSetAttributeValue(
                self.element.0,
                key.as_concrete_TypeRef(),
                CFString::new(text).as_CFTypeRef(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(format!(
                "Native insertion failed ({status}); inspect destination before recovery"
            ))
        }
    }
}

struct Mac;
impl Backend for Mac {
    type Native = NativeTarget;

    fn frontmost_pid(&self) -> Option<i32> {
        // Query live process state even during a delivery task: AppKit's
        // running-application properties may refresh only on run-loop turns.
        let mut process = ProcessSerialNumber::default();
        let mut pid = 0;
        (unsafe { GetFrontProcess(&mut process) } == 0
            && unsafe { GetProcessPID(&process, &mut pid) } == 0)
            .then_some(pid)
    }
    fn own_pid(&self) -> i32 {
        std::process::id() as i32
    }
    fn secure_input(&self) -> bool {
        unsafe { IsSecureEventInputEnabled() != 0 }
    }
    fn focus(&self) -> Result<Focus<NativeTarget>, String> {
        let system = Object(unsafe { AXUIElementCreateSystemWide() });
        unsafe {
            AXUIElementSetMessagingTimeout(system.0, 0.2);
        }
        let element = system.get("AXFocusedUIElement")?;
        unsafe {
            AXUIElementSetMessagingTimeout(element.0, 0.2);
        }
        let role = element.get("AXRole")?;
        let secure = equals(&role, "AXSecureTextField")
            || element
                .optional("AXSubrole")?
                .is_some_and(|r| equals(&r, "AXSecureTextField"));
        let mut pid = 0;
        if unsafe { AXUIElementGetPid(element.0, &mut pid) } != 0 {
            return Err("Cannot identify destination".into());
        }
        let native = if !secure
            && ["AXTextField", "AXTextArea"]
                .iter()
                .any(|r| equals(&role, r))
        {
            element
                .optional("AXSelectedTextRange")?
                .map(|selection| NativeTarget {
                    element,
                    selection,
                    pid,
                })
        } else {
            None
        };
        Ok(Focus {
            pid,
            secure,
            native,
        })
    }
    fn validate_native(&self, native: &NativeTarget) -> Result<(), String> {
        native.validate()
    }
    fn write_native(&self, native: &NativeTarget, text: &str) -> Result<(), String> {
        native.insert(text)
    }
    fn type_text(&self, pid: i32, text: &str) -> Result<(), String> {
        let source = CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|_| "Cannot create typing event source")?;
        // Stay within CGEvent's short Unicode string limit without splitting a
        // surrogate pair. Virtual key 0 is not Return. Posting to the captured
        // pid also avoids redirecting queued events to another app.
        for chunk in unicode_chunks(text) {
            let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
                .map_err(|_| "Cannot create typing event; inspect destination before recovery")?;
            event.set_string(chunk);
            event.set_flags(CGEventFlags::CGEventFlagNull);
            validate_keystrokes(self, pid)?;
            event.post_to_pid(pid);
        }
        Ok(())
    }
}

fn equals(value: &Object, text: &str) -> bool {
    unsafe { CFEqual(value.0, CFString::new(text).as_CFTypeRef()) != 0 }
}

impl Object {
    fn optional(&self, key: &str) -> Result<Option<Self>, String> {
        let mut value = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(
                self.0,
                CFString::new(key).as_concrete_TypeRef(),
                &mut value,
            )
        };
        match status {
            0 if !value.is_null() => Ok(Some(Self(value))),
            // kAXErrorAttributeUnsupported / kAXErrorNoValue: not a usable text
            // target. Permission errors, timeouts, and invalid elements fail closed.
            -25205 | -25212 => Ok(None),
            _ => Err(format!(
                "Accessibility attribute unavailable ({key}, {status})"
            )),
        }
    }
}

fn require_main_thread() -> Result<(), String> {
    objc2::MainThreadMarker::new()
        .map(|_| ())
        .ok_or_else(|| "Insertion requires the main thread".into())
}

/// Dispatch before acquiring the session lock. Carbon foreground/secure-input
/// checks run on the main thread; waiting while holding that lock would deadlock
/// against a main-thread hotkey/cancel callback. No keyboard-layout calls occur.
pub(crate) fn on_main_thread<T: Send + 'static>(
    app: &tauri::AppHandle,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    if objc2::MainThreadMarker::new().is_some() {
        return task();
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(task());
    })
    .map_err(|e| format!("Cannot schedule insertion: {e}"))?;
    rx.recv().map_err(|_| "Insertion task unavailable")?
}

pub fn type_text(app: &tauri::AppHandle, text: &str) -> Result<(), String> {
    let text = text.to_owned();
    on_main_thread(app, move || Target::capture()?.insert(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Fake {
        frontmost: Cell<Option<i32>>,
        focus_pid: Cell<i32>,
        native: Cell<Option<u8>>,
        secure: Cell<bool>,
        secure_input: Cell<bool>,
        focus_error: Cell<bool>,
        stale: Cell<bool>,
        write_error: Cell<bool>,
        writes: RefCell<Vec<String>>,
        typed: RefCell<Vec<String>>,
    }
    impl Default for Fake {
        fn default() -> Self {
            Self {
                frontmost: Cell::new(Some(42)),
                focus_pid: Cell::new(42),
                native: Cell::new(None),
                secure: Cell::new(false),
                secure_input: Cell::new(false),
                focus_error: Cell::new(false),
                stale: Cell::new(false),
                write_error: Cell::new(false),
                writes: RefCell::default(),
                typed: RefCell::default(),
            }
        }
    }
    impl Backend for Fake {
        type Native = u8;
        fn frontmost_pid(&self) -> Option<i32> {
            self.frontmost.get()
        }
        fn own_pid(&self) -> i32 {
            7
        }
        fn secure_input(&self) -> bool {
            self.secure_input.get()
        }
        fn focus(&self) -> Result<Focus<u8>, String> {
            if self.focus_error.get() {
                return Err("AX unavailable".into());
            }
            Ok(Focus {
                pid: self.focus_pid.get(),
                secure: self.secure.get(),
                native: self.native.get(),
            })
        }
        fn validate_native(&self, _: &u8) -> Result<(), String> {
            if self.stale.get() {
                Err("stale".into())
            } else {
                Ok(())
            }
        }
        fn write_native(&self, _: &u8, text: &str) -> Result<(), String> {
            self.writes.borrow_mut().push(text.into());
            if self.write_error.get() {
                Err("uncertain write".into())
            } else {
                Ok(())
            }
        }
        fn type_text(&self, _: i32, text: &str) -> Result<(), String> {
            self.typed.borrow_mut().push(text.into());
            if self.write_error.get() {
                Err("partial typing".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn native_target_preserves_multiline_and_never_uses_keystrokes() {
        let backend = Fake::default();
        backend.native.set(Some(1));
        // An AX element may belong to an app's helper process. The native path
        // continues using its captured element/pid/selection validation.
        backend.frontmost.set(Some(100));
        let (target, pid) = Destination::capture(&backend).unwrap();
        assert_eq!(pid, 42);
        assert!(matches!(target, Destination::Accessibility(1)));
        target.insert(&backend, "a\nb").unwrap();
        assert_eq!(*backend.writes.borrow(), ["a\nb"]);
        assert!(backend.typed.borrow().is_empty());
    }

    #[test]
    fn stale_native_and_uncertain_write_never_fall_back() {
        for stale in [false, true] {
            let backend = Fake::default();
            backend.native.set(Some(1));
            let (target, _) = Destination::capture(&backend).unwrap();
            backend.native.set(None);
            backend.stale.set(stale);
            backend.write_error.set(true);
            assert!(target.insert(&backend, "text").is_err());
            assert_eq!(backend.writes.borrow().len(), usize::from(!stale));
            assert!(backend.typed.borrow().is_empty());
        }
    }

    #[test]
    fn unsupported_focus_uses_captured_pid_and_flattened_text() {
        let backend = Fake::default();
        let (target, pid) = Destination::capture(&backend).unwrap();
        assert_eq!(pid, 42);
        assert!(matches!(target, Destination::Keystrokes(42)));
        target
            .insert(&backend, "First\r\n\n• second\n• third")
            .unwrap();
        assert_eq!(*backend.typed.borrow(), ["First • second • third"]);
        assert!(backend.writes.borrow().is_empty());
    }

    #[test]
    fn capture_rejects_missing_foreground_self_secure_or_unknown_focus() {
        for condition in 0..7 {
            let backend = Fake::default();
            match condition {
                0 => backend.frontmost.set(None),
                1 => {
                    backend.frontmost.set(Some(7));
                    backend.focus_pid.set(7);
                }
                2 => backend.secure.set(true),
                3 => backend.secure_input.set(true),
                4 => backend.focus_error.set(true),
                5 => backend.frontmost.set(Some(43)),
                _ => backend.focus_pid.set(0),
            }
            assert!(
                Destination::capture(&backend).is_err(),
                "condition {condition}"
            );
        }
    }

    #[test]
    fn secure_native_field_is_not_a_fallback_candidate() {
        let backend = Fake::default();
        backend.native.set(Some(1));
        backend.secure.set(true);
        assert!(Destination::capture(&backend).is_err());
    }

    #[test]
    fn delivery_rechecks_foreground_and_both_secure_input_guards() {
        for condition in 0..7 {
            let backend = Fake::default();
            let (target, _) = Destination::capture(&backend).unwrap();
            match condition {
                0 => backend.frontmost.set(None),
                1 => backend.frontmost.set(Some(43)),
                2 => backend.frontmost.set(Some(7)),
                3 => backend.secure.set(true),
                4 => backend.secure_input.set(true),
                5 => backend.focus_error.set(true),
                _ => backend.focus_pid.set(43),
            }
            assert!(
                target.insert(&backend, "text").is_err(),
                "condition {condition}"
            );
            assert!(backend.typed.borrow().is_empty());
            assert!(backend.writes.borrow().is_empty());
        }
    }

    #[test]
    fn fallback_does_not_switch_to_a_new_native_target_at_delivery() {
        let backend = Fake::default();
        let (target, _) = Destination::capture(&backend).unwrap();
        backend.native.set(Some(1));
        target.insert(&backend, "a\nb").unwrap();
        assert_eq!(*backend.typed.borrow(), ["a b"]);
        assert!(backend.writes.borrow().is_empty());
    }

    #[test]
    fn line_breaks_and_controls_cannot_submit_terminal_commands() {
        for (input, expected) in [
            ("", ""),
            ("one\ntwo", "one two"),
            ("one\r\ntwo\rthree", "one two three"),
            ("one\n\ntwo", "one two"),
            ("\n\r\n", " "),
            ("a\u{85}b\u{2028}c\u{2029}d", "a b c d"),
            ("\t\u{1b}\u{0}x\u{7f}", " x "),
            ("  café  日本語 🦀  ", "  café  日本語 🦀  "),
        ] {
            assert_eq!(flatten_newlines(input), expected);
        }
    }

    #[test]
    fn unicode_events_preserve_surrogates_at_chunk_boundaries() {
        for text in ["", "a", "1234567890123456789🦀é日本語🦀🦀🦀🦀🦀🦀"] {
            let chunks: Vec<_> = unicode_chunks(text).collect();
            assert_eq!(chunks.concat(), text);
            assert!(chunks
                .iter()
                .all(|chunk| !chunk.is_empty() && chunk.encode_utf16().count() <= 20));
        }
    }

    #[test]
    fn failed_typing_retains_original_transcript_and_is_not_replayed() {
        let backend = Fake::default();
        let (target, _) = Destination::capture(&backend).unwrap();
        backend.write_error.set(true);
        let mut sessions = crate::session::Sessions::default();
        let id = sessions.begin();
        assert!(sessions
            .deliver(id, "a\nb".into(), |_, text| target.insert(&backend, text))
            .is_err());
        assert_eq!(sessions.recovery().as_deref(), Some("a\nb"));
        assert_eq!(sessions.take_history(id).as_deref(), Some("a\nb"));
        assert!(sessions
            .deliver(id, "a\nb".into(), |_, _| panic!("must not retry"))
            .is_err());
        assert_eq!(*backend.typed.borrow(), ["a b"]);
        assert!(sessions.take_history(id).is_none());
    }

    #[test]
    fn cancelled_and_superseded_sessions_never_type() {
        let backend = Fake::default();
        let (target, _) = Destination::capture(&backend).unwrap();
        let mut sessions = crate::session::Sessions::default();
        let cancelled = sessions.begin();
        sessions.cancel();
        assert!(!sessions
            .deliver(cancelled, "text".into(), |_, text| target
                .insert(&backend, text))
            .unwrap());
        let stale = sessions.begin();
        sessions.begin();
        assert!(!sessions
            .deliver(stale, "text".into(), |_, text| target
                .insert(&backend, text))
            .unwrap());
        assert!(backend.typed.borrow().is_empty());
    }
}
