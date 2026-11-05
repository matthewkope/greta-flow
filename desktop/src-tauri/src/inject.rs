//! Target-bound Accessibility insertion. Never synthesizes Return or pastes into terminals.
use core_foundation::base::{CFEqual, CFRelease, CFRetain, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;

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

#[derive(Clone, Debug)]
pub struct Target {
    element: Object,
    selection: Object,
    pub pid: i32,
}
impl Target {
    pub fn capture() -> Result<Self, String> {
        let system = Object(unsafe { AXUIElementCreateSystemWide() });
        unsafe {
            AXUIElementSetMessagingTimeout(system.0, 0.2);
        }
        let element = system.get("AXFocusedUIElement")?;
        unsafe {
            AXUIElementSetMessagingTimeout(element.0, 0.2);
        }
        let role = element.get("AXRole")?;
        let allowed = ["AXTextField", "AXTextArea"]
            .iter()
            .any(|r| unsafe { CFEqual(role.0, CFString::new(r).as_CFTypeRef()) != 0 });
        let secure = element.get("AXSubrole").is_ok_and(|r| unsafe {
            CFEqual(r.0, CFString::new("AXSecureTextField").as_CFTypeRef()) != 0
        });
        if !allowed || secure {
            return Err("Focus an editable, non-password text field".into());
        }
        let selection = element.get("AXSelectedTextRange")?;
        let mut pid = 0;
        if unsafe { AXUIElementGetPid(element.0, &mut pid) } != 0 {
            return Err("Cannot identify destination".into());
        }
        Ok(Self {
            element,
            selection,
            pid,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        let now = Self::capture()?;
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

pub fn type_text(text: &str) -> Result<(), String> {
    Target::capture()?.insert(text)
}
