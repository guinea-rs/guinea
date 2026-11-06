//! Native visibility observation without changing the window procedure owner.
use guinea_app::app::windows::set_presentation;
use guinea_core::scope::Scope;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            IsIconic, IsWindowVisible, WM_NCDESTROY, WM_SHOWWINDOW, WM_SIZE, WM_WINDOWPOSCHANGED,
        },
    },
};

pub(crate) struct Observation {
    hwnd: HWND,
    scope: Box<Scope>,
}
impl Observation {
    pub fn install(hwnd: HWND, scope: Scope) -> std::io::Result<Self> {
        let observation = Self {
            hwnd,
            scope: Box::new(scope),
        };
        let id = &*observation.scope as *const Scope as usize;
        // The scope box stays pinned until Drop removes this UI-thread subclass.
        if unsafe { SetWindowSubclass(hwnd, Some(observe), id, id) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        publish(hwnd, scope);
        Ok(observation)
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        unsafe {
            RemoveWindowSubclass(
                self.hwnd,
                Some(observe),
                &*self.scope as *const Scope as usize,
            );
        }
    }
}
fn publish(hwnd: HWND, scope: Scope) {
    if scope.is_alive() {
        set_presentation(
            scope,
            unsafe { IsWindowVisible(hwnd) } != 0,
            unsafe { IsIconic(hwnd) } != 0,
        );
    }
}
unsafe extern "system" fn observe(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    // Copy before dispatch: downstream code or callbacks may remove the scope
    // and its observation. Never access data again after invoking other code.
    let scope = unsafe { *(data as *const Scope) };
    if msg == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(observe), id);
        }
    }
    let result = unsafe { DefSubclassProc(hwnd, msg, w, l) };
    if matches!(
        msg,
        WM_SHOWWINDOW | WM_SIZE | WM_WINDOWPOSCHANGED | WM_NCDESTROY
    ) {
        let reported = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if msg == WM_NCDESTROY {
                if scope.is_alive() {
                    set_presentation(scope, false, false);
                }
            } else {
                publish(hwnd, scope);
            }
        }));
        if reported.is_err() {
            tracing::error!("window presentation callback panicked");
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use guinea_app::{
        app::{GuineaHost, roots::Root},
        feature::ScopeContext,
    };
    use std::{cell::RefCell, ptr::null_mut, rc::Rc};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    #[test]
    #[ignore = "requires a Windows desktop; creates an offscreen non-activating test window"]
    fn native_show_hide_minimize_and_restore_report_distinct_values() {
        let host = GuineaHost::new();
        let root = Root::under(&host.context());
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                windows_sys::w!("STATIC"),
                windows_sys::w!("Guinea presentation test"),
                WS_POPUP,
                -30000,
                -30000,
                1,
                1,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        assert!(!hwnd.is_null());
        struct Window(HWND);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        let _window = Window(hwnd);
        let observation = Observation::install(hwnd, root.scope()).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let copy = seen.clone();
        ScopeContext::new(root.scope().child())
            .on_window_presented(move |v| copy.borrow_mut().push(v));
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
            ShowWindow(hwnd, SW_HIDE);
        }
        assert_eq!(&*seen.borrow(), &[false, true, false]);
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        assert_eq!(&*seen.borrow(), &[false, true, false, true]);
        drop(observation);
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
        assert_eq!(&*seen.borrow(), &[false, true, false, true]);
    }
}
