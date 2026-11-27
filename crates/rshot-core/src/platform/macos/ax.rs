use std::{
    ffi::c_void,
    ptr::{self, NonNull},
};

use objc2_core_foundation::{
    CFArray, CFBoolean, CFGetTypeID, CFIndex, CFNumber, CFRetained, CFString, CFType, CFTypeID,
    CGPoint, CGSize, Type,
};
use objc2_core_graphics::{CGError, CGGetDisplaysWithPoint};

use super::windows;
use crate::{
    capture::MacOsCapture,
    elements::{
        ElementAncestor, ElementError, ElementFilter, ElementHit, ElementInfo, ElementList, walk,
    },
    geometry::{Point, Rect},
};

const SUCCESS: i32 = 0;
const FAILURE: i32 = -25200;
const CANNOT_COMPLETE: i32 = -25204;
const ATTRIBUTE_UNSUPPORTED: i32 = -25205;
const API_DISABLED: i32 = -25211;
const NO_VALUE: i32 = -25212;
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;
const AX_ERROR: u32 = 5;
const MESSAGING_TIMEOUT: f32 = 1.;
const MAX_ANCESTORS: usize = 16;
const MAX_VALUE_CHARS: usize = 256;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> *mut CFType;
    fn AXUIElementCreateApplication(pid: i32) -> *mut CFType;
    fn AXUIElementGetTypeID() -> CFTypeID;
    fn AXUIElementSetMessagingTimeout(element: &CFType, timeout: f32) -> i32;
    fn AXUIElementCopyElementAtPosition(
        application: &CFType,
        x: f32,
        y: f32,
        element: *mut *mut CFType,
    ) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: &CFType,
        name: &CFString,
        value: *mut *mut CFType,
    ) -> i32;
    fn AXUIElementCopyMultipleAttributeValues(
        element: &CFType,
        names: &CFArray<CFString>,
        options: u32,
        values: *mut *mut CFArray<CFType>,
    ) -> i32;
    fn AXUIElementGetAttributeValueCount(
        element: &CFType,
        name: &CFString,
        count: *mut CFIndex,
    ) -> i32;
    fn AXUIElementCopyAttributeValues(
        element: &CFType,
        name: &CFString,
        offset: CFIndex,
        limit: CFIndex,
        values: *mut *mut CFArray<CFType>,
    ) -> i32;
    fn AXValueGetTypeID() -> CFTypeID;
    fn AXValueGetValue(value: &CFType, kind: u32, out: *mut c_void) -> u8;
}

/// Hit-tests the system at a position in global points, with the nearest ancestor first.
pub fn element_at(point: Point<f64>) -> Result<ElementHit, ElementError> {
    let system = accessibility_session()?;
    let (x, y) = (point.x as f32, point.y as f32);
    if !x.is_finite() || !y.is_finite() || is_off_display(point) {
        return Err(ElementError::ElementNotFound);
    }
    let hit = copied("hit testing", |out| {
        // The system-wide AX element and the writable result slot live through the call.
        unsafe { AXUIElementCopyElementAtPosition(&system, x, y, out) }
    })?
    .ok_or(ElementError::ElementNotFound)?;
    let (role, bounds) = frame(&hit)?;
    let element = describe(
        &hit,
        role.ok_or(ElementError::ElementNotFound)?,
        bounds.ok_or(ElementError::ElementNotFound)?,
    )?;

    let mut ancestors = Vec::new();
    let mut node = hit;
    for _ in 0..MAX_ANCESTORS {
        let Some(parent) = attribute(&node, "AXParent")?.filter(|value| is_element(value)) else {
            break;
        };
        let (role, bounds) = frame(&parent)?;
        ancestors.push(ElementAncestor { role, bounds });
        node = parent;
    }
    Ok(ElementHit { element, ancestors })
}

/// Lists bounded elements in the owning process's accessibility tree in preorder.
pub fn elements_in_window(
    window_id: u32,
    filter: ElementFilter<'_>,
) -> Result<ElementList, ElementError> {
    accessibility_session()?;
    let window = windows::find(window_id).ok_or(ElementError::WindowNotFound(window_id))?;
    // Create returns an owned AX object for the supplied WindowServer PID.
    let app = unsafe { retained(AXUIElementCreateApplication(window.owner_pid)) }
        .ok_or(ElementError::ElementNotFound)?;
    walk(
        &app,
        filter.max_results,
        |node| {
            let (role, bounds) = frame(node)?;
            match (role, bounds) {
                (Some(role), Some(bounds)) if filter.matches(&role, bounds, window.bounds) => {
                    describe(node, role, bounds).map(Some)
                }
                _ => Ok(None),
            }
        },
        children,
    )
}

fn is_off_display(point: Point<f64>) -> bool {
    let mut count = 0;
    // A null display buffer requests only the count, written into the supplied slot.
    let status = unsafe {
        CGGetDisplaysWithPoint(
            CGPoint::new(point.x, point.y),
            0,
            ptr::null_mut(),
            &mut count,
        )
    };
    status == CGError::Success && count == 0
}

fn accessibility_session() -> Result<CFRetained<CFType>, ElementError> {
    if MacOsCapture.is_screen_locked() {
        return Err(ElementError::ScreenLocked);
    }
    // AXIsProcessTrusted takes no arguments and does not prompt the user.
    if unsafe { AXIsProcessTrusted() } == 0 {
        return Err(ElementError::AccessibilityDenied);
    }
    // Create returns an owned AX object; the system-wide timeout applies to its descendants too.
    let system =
        unsafe { retained(AXUIElementCreateSystemWide()) }.ok_or(ElementError::ElementNotFound)?;
    let status = unsafe { AXUIElementSetMessagingTimeout(&system, MESSAGING_TIMEOUT) };
    check_status("set messaging timeout", status)?;
    Ok(system)
}

fn frame(node: &CFType) -> Result<(Option<String>, Option<Rect<f64>>), ElementError> {
    let [role, position, size] = attributes(node, ["AXRole", "AXPosition", "AXSize"])?;
    let bounds = position.zip(size).and_then(|(position, size)| {
        let mut point = CGPoint::default();
        let mut extent = CGSize::default();
        // The AX kinds match the types of the writable outputs.
        if !unsafe {
            read_ax_value(&position, AX_POINT, &mut point)
                && read_ax_value(&size, AX_SIZE, &mut extent)
        } {
            return None;
        }
        let CGPoint { x, y } = point;
        let CGSize { width, height } = extent;
        (width > 0.
            && height > 0.
            && [x, y, x + width, y + height]
                .into_iter()
                .all(f64::is_finite))
        .then(|| Rect::new(x, y, width, height))
    });
    Ok((text(role), bounds))
}

fn describe(node: &CFType, role: String, bounds: Rect<f64>) -> Result<ElementInfo, ElementError> {
    let [
        subrole,
        title,
        description,
        value,
        identifier,
        enabled,
        focused,
    ] = attributes(
        node,
        [
            "AXSubrole",
            "AXTitle",
            "AXDescription",
            "AXValue",
            "AXIdentifier",
            "AXEnabled",
            "AXFocused",
        ],
    )?;
    Ok(ElementInfo {
        role,
        subrole: text(subrole),
        title: text(title),
        description: text(description),
        value: value.as_deref().and_then(display_value),
        identifier: text(identifier),
        bounds,
        enabled: boolean(enabled),
        focused: boolean(focused),
    })
}

fn attribute(
    node: &CFType,
    name: &'static str,
) -> Result<Option<CFRetained<CFType>>, ElementError> {
    let key = CFString::from_static_str(name);
    copied(name, |out| {
        // Node and key remain alive and the output slot is writable.
        unsafe { AXUIElementCopyAttributeValue(node, &key, out) }
    })
}

fn attributes<const N: usize>(
    node: &CFType,
    names: [&'static str; N],
) -> Result<[Option<CFRetained<CFType>>; N], ElementError> {
    let keys = names.map(CFString::from_static_str);
    let keys = CFArray::from_retained_objects(&keys);
    let values = copied("read attributes", |out| {
        // Options 0 returns a parallel array with per-attribute errors for partial results.
        unsafe { AXUIElementCopyMultipleAttributeValues(node, &keys, 0, out) }
    })?;
    let mut attributes = std::array::from_fn(|_| None);
    if let Some(values) = values {
        for (index, (value, name)) in values.iter().zip(names).enumerate() {
            let mut error = SUCCESS;
            // The error kind writes an AXError (i32) to the result slot.
            if unsafe { read_ax_value(&value, AX_ERROR, &mut error) } {
                check_attribute_status(name, error)?;
            } else {
                attributes[index] = Some(value);
            }
        }
    }
    Ok(attributes)
}

fn children(
    node: &CFRetained<CFType>,
    limit: usize,
) -> Result<impl Iterator<Item = CFRetained<CFType>> + use<>, ElementError> {
    let operation = "AXChildren";
    let key = CFString::from_static_str(operation);
    let mut count = 0;
    // The framework writes a CFIndex into count; node and key outlive the call.
    let status = unsafe { AXUIElementGetAttributeValueCount(node, &key, &mut count) };
    let values = if is_missing(status) {
        None
    } else {
        check_status(operation, status)?;
        let count = count.min(limit as CFIndex);
        if count <= 0 {
            None
        } else {
            copied(operation, |out| {
                // Read only the prefix needed by the remaining traversal budget.
                unsafe { AXUIElementCopyAttributeValues(node, &key, 0, count, out) }
            })?
        }
    };
    Ok(values
        .into_iter()
        .flatten()
        .filter(|value| is_element(value)))
}

fn text(value: Option<CFRetained<CFType>>) -> Option<String> {
    Some(value?.downcast_ref::<CFString>()?.to_string())
}

fn boolean(value: Option<CFRetained<CFType>>) -> Option<bool> {
    Some(value?.downcast_ref::<CFBoolean>()?.value())
}

fn display_value(value: &CFType) -> Option<String> {
    if let Some(text) = value.downcast_ref::<CFString>() {
        return Some(text.to_string().chars().take(MAX_VALUE_CHARS).collect());
    }
    if let Some(value) = value.downcast_ref::<CFBoolean>() {
        return Some(value.value().to_string());
    }
    let number = value.downcast_ref::<CFNumber>()?;
    if number.is_float_type() {
        number.as_f64().map(|value| value.to_string())
    } else {
        number.as_i64().map(|value| value.to_string())
    }
}

fn copied<T: Type>(
    operation: &'static str,
    copy: impl FnOnce(&mut *mut T) -> i32,
) -> Result<Option<CFRetained<T>>, ElementError> {
    let mut out = ptr::null_mut();
    let status = copy(&mut out);
    if is_missing(status) {
        return Ok(None);
    }
    check_status(operation, status)?;
    // The AX Copy calls give the caller ownership of successful output values.
    Ok(unsafe { retained(out) })
}

unsafe fn retained<T: Type>(value: *mut T) -> Option<CFRetained<T>> {
    NonNull::new(value).map(|ptr| unsafe { CFRetained::from_raw(ptr) })
}

fn is_element(value: &CFType) -> bool {
    // The AX element type identifier is a framework constant.
    CFGetTypeID(Some(value)) == unsafe { AXUIElementGetTypeID() }
}

unsafe fn read_ax_value<T>(value: &CFType, kind: u32, out: &mut T) -> bool {
    // Only AXValue objects can be decoded, and the caller matches kind to the writable type.
    unsafe {
        CFGetTypeID(Some(value)) == AXValueGetTypeID()
            && AXValueGetValue(value, kind, ptr::from_mut(out).cast()) != 0
    }
}

fn is_missing(code: i32) -> bool {
    matches!(code, ATTRIBUTE_UNSUPPORTED | NO_VALUE)
}

fn check_attribute_status(operation: &'static str, code: i32) -> Result<(), ElementError> {
    // Finder returns a per-attribute failure for unavailable AXValue data in a successful batch.
    if is_missing(code) || code == FAILURE {
        return Ok(());
    }
    check_status(operation, code)
}

fn check_status(operation: &'static str, code: i32) -> Result<(), ElementError> {
    match code {
        SUCCESS => Ok(()),
        CANNOT_COMPLETE => Err(ElementError::Timeout { operation }),
        API_DISABLED => Err(ElementError::AccessibilityDenied),
        _ => Err(ElementError::Ax { operation, code }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_scalar_values_and_truncates_by_character() {
        let text = CFString::from_str(&"界".repeat(MAX_VALUE_CHARS + 1));
        assert_eq!(display_value(&text), Some("界".repeat(MAX_VALUE_CHARS)));
        assert_eq!(
            display_value(CFBoolean::new(false)).as_deref(),
            Some("false")
        );
        assert_eq!(
            display_value(&CFNumber::new_i64(9_007_199_254_740_993)).as_deref(),
            Some("9007199254740993")
        );
        assert_eq!(
            display_value(&CFNumber::new_f64(2.5)).as_deref(),
            Some("2.5")
        );
    }

    #[test]
    fn classifies_native_messaging_and_permission_errors() {
        assert!(matches!(
            check_status("AXRole", CANNOT_COMPLETE),
            Err(ElementError::Timeout {
                operation: "AXRole"
            })
        ));
        assert!(matches!(
            check_status("AXRole", API_DISABLED),
            Err(ElementError::AccessibilityDenied)
        ));
        assert!(matches!(
            check_status("AXRole", FAILURE),
            Err(ElementError::Ax { code: FAILURE, .. })
        ));
    }

    #[test]
    fn an_unavailable_attribute_does_not_discard_the_element() {
        assert!(check_attribute_status("AXValue", FAILURE).is_ok());
        assert!(check_attribute_status("AXValue", ATTRIBUTE_UNSUPPORTED).is_ok());
        assert!(matches!(
            check_attribute_status("AXValue", CANNOT_COMPLETE),
            Err(ElementError::Timeout {
                operation: "AXValue"
            })
        ));
    }

    #[test]
    #[ignore = "requires Accessibility permission and a visible window in an unlocked session"]
    fn queries_the_frontmost_window_and_hit_tests_a_listed_element() {
        let window = MacOsCapture
            .windows()
            .into_iter()
            .find(|window| window.is_on_screen)
            .unwrap();
        let list = elements_in_window(window.window_id, ElementFilter::default()).unwrap();
        let entry = list.elements.first().unwrap();
        let visible = entry.element.bounds.intersection(window.bounds).unwrap();
        let hit = element_at(Point::new(
            visible.origin.x + visible.size.width / 2.,
            visible.origin.y + visible.size.height / 2.,
        ))
        .unwrap();
        assert!(!hit.element.role.is_empty());

        let left_edge = MacOsCapture
            .displays()
            .unwrap()
            .iter()
            .map(|display| display.bounds.origin.x)
            .min_by(f64::total_cmp)
            .unwrap();
        assert!(matches!(
            element_at(Point::new(left_edge - 10., 0.)),
            Err(ElementError::ElementNotFound)
        ));
    }
}
