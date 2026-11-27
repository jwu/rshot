#[cfg(target_os = "macos")]
pub(super) use crate::macos::physical_footprint;

#[cfg(not(target_os = "macos"))]
pub(super) fn physical_footprint() -> Option<u64> {
    None
}
