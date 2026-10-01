//! The monitors' work areas, asked of the system.
//!
//! winit knows the monitors but not their work areas - the taskbar is not its
//! business - so Windows is asked directly. Elsewhere nothing is known yet, and
//! geometry is applied as it comes.

use guinea_app::app::windows::WorkArea;

/// Every monitor's work area, the primary one first.
#[cfg(windows)]
pub(crate) fn work_areas() -> Vec<WorkArea> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
    use windows_sys::core::BOOL;

    unsafe extern "system" fn each(monitor: HMONITOR, _: HDC, _: *mut RECT, found: LPARAM) -> BOOL {
        let found = unsafe { &mut *(found as *mut Vec<WorkArea>) };

        if let Some((area, primary)) = work_area(monitor) {
            match primary {
                true => found.insert(0, area),
                false => found.push(area),
            }
        }
        1
    }

    let mut found = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(each),
            &mut found as *mut Vec<WorkArea> as LPARAM,
        );
    }
    found
}

/// One monitor's work area, and whether it is the primary one.
#[cfg(windows)]
fn work_area(monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> Option<(WorkArea, bool)> {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    const PRIMARY: u32 = 1;

    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }

    let (mut dpi, mut unused) = (0_u32, 0_u32);
    let read = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused) };
    let scale = match read == 0 && dpi > 0 {
        true => f64::from(dpi) / 96.0,
        false => 1.0,
    };

    let work = info.rcWork;
    let area = WorkArea {
        x: f64::from(work.left),
        y: f64::from(work.top),
        width: f64::from(work.right - work.left),
        height: f64::from(work.bottom - work.top),
        scale,
    };
    Some((area, info.dwFlags & PRIMARY != 0))
}

/// Nothing known here yet: geometry goes on as it comes.
#[cfg(not(windows))]
pub(crate) fn work_areas() -> Vec<WorkArea> {
    Vec::new()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_this_runs_on_has_a_primary_monitor_with_room_on_it() {
        let areas = work_areas();

        let primary = areas.first().expect("a desktop has at least one monitor");
        assert!(primary.width > 0.0 && primary.height > 0.0, "{primary:?}");
        assert!(primary.scale >= 1.0, "{primary:?}");
    }
}
