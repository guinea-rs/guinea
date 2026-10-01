//! The window contract, over Slint's own.
//!
//! Slint answers almost all of it directly - size, position, maximised,
//! fullscreen, visible, close - which is why the contract was written by
//! looking at winit and Slint rather than at the poorest backend.
//!
//! What Slint does not have is *notice*: there is no callback for "the window
//! was resized" or "the window moved". So changes are noticed by asking, on a
//! timer, and published only when the answer differs from the last one. Not
//! elegant, but honest and cheap - four getters, and the only party that would
//! care (something storing the geometry) has to debounce anyway.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use guinea_app::app::roots::RootId;
use guinea_app::app::windows::{
    Done, Geometry, Position, Size, Unsupported, WindowChanged, Windows, fit,
};
use guinea_core::actor::event_bus::GlobalEventBus;
use slint::{ComponentHandle, LogicalSize, PhysicalPosition};

use crate::monitors;

#[cfg(feature = "winit")]
use slint::winit_030::WinitWindowAccessor;

/// How often the windows are asked where they are.
///
/// Slow enough to be free, quick enough that a drag that ends in a crash still
/// leaves a nearly-right position behind.
const POLL: Duration = Duration::from_millis(250);

/// One shell's windows, keyed by the root inside them.
#[derive(Default)]
pub(crate) struct SlintWindows {
    entries: Mutex<HashMap<RootId, Entry>>,
}

struct Entry {
    window: Box<dyn Handle>,
    /// The last geometry published, so a poll that finds nothing new says
    /// nothing.
    last: Option<Geometry>,
}

impl SlintWindows {
    /// Puts a root's window on the register.
    pub(crate) fn attach<W: ComponentHandle + 'static>(
        self: &std::sync::Arc<Self>,
        root: RootId,
        window: &W,
    ) {
        let entry = Entry {
            window: Box::new(window.as_weak()),
            last: None,
        };

        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(root, entry);
        }

        self.watch_events(window);
    }

    /// Asks winit to tell us, instead of asking the window on a timer.
    #[cfg(feature = "winit")]
    fn watch_events<W: ComponentHandle + 'static>(self: &std::sync::Arc<Self>, window: &W) {
        use slint::winit_030::{EventResult, winit::event::WindowEvent};

        let windows = self.clone();
        window.window().on_winit_window_event(move |_, event| {
            if matches!(
                event,
                WindowEvent::Resized(_) | WindowEvent::Moved(_) | WindowEvent::Occluded(_)
            ) {
                windows.poll();
            }
            // Only listening: Slint still handles the event as it would.
            EventResult::Propagate
        });
    }

    #[cfg(not(feature = "winit"))]
    fn watch_events<W: ComponentHandle + 'static>(self: &std::sync::Arc<Self>, _window: &W) {}

    pub(crate) fn detach(&self, root: RootId) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&root);
        }
    }

    /// Asks every window where it is, and publishes what moved.
    ///
    /// Call from the thread that owns the windows - a weak handle upgrades
    /// nowhere else.
    pub(crate) fn poll(&self) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };

        for (root, entry) in entries.iter_mut() {
            let Some(geometry) = entry.window.geometry() else {
                continue;
            };
            if entry.last == Some(geometry) {
                continue;
            }

            entry.last = Some(geometry);
            tracing::debug!(%root, ?geometry, "window changed");
            GlobalEventBus::publish(WindowChanged {
                root: *root,
                geometry,
            });
        }
    }

    fn with<T>(&self, root: RootId, f: impl FnOnce(&dyn Handle) -> T) -> Option<T> {
        let entries = self.entries.lock().ok()?;
        entries.get(&root).map(|entry| f(entry.window.as_ref()))
    }
}

impl Windows for SlintWindows {
    fn geometry(&self, root: RootId) -> Option<Geometry> {
        self.with(root, |window| window.geometry()).flatten()
    }

    fn apply(&self, root: RootId, geometry: Geometry) -> Done {
        self.with(root, |window| window.apply(geometry))
            .unwrap_or(Err(Unsupported))
    }

    fn set_visible(&self, root: RootId, visible: bool) -> Done {
        self.with(root, |window| window.set_visible(visible))
            .unwrap_or(Err(Unsupported))
    }

    fn close(&self, root: RootId) -> Done {
        self.set_visible(root, false)
    }

    /// Asked of winit, through the window Slint is standing on: dragging is
    /// the window manager's gesture, and Slint's own API does not reach it.
    fn start_drag(&self, root: RootId) -> Done {
        self.with(root, |window| window.start_drag())
            .unwrap_or(Err(Unsupported))
    }
}

/// One window, reachable only from the thread that owns it.
trait Handle: Send + Sync {
    fn geometry(&self) -> Option<Geometry>;
    fn apply(&self, geometry: Geometry) -> Done;
    fn set_visible(&self, visible: bool) -> Done;
    fn start_drag(&self) -> Done;
}

impl<W: ComponentHandle + 'static> Handle for slint::Weak<W> {
    fn geometry(&self) -> Option<Geometry> {
        let component = self.upgrade()?;
        let window = component.window();

        // Nothing to read while minimised: Windows answers `-32000, -32000`
        // for the position of one, and that is a value to drop rather than to
        // pass on.
        if window.is_minimized() {
            return Some(Geometry {
                minimized: true,
                ..Geometry::default()
            });
        }

        let size = window.size().to_logical(window.scale_factor());
        let position = window.position();

        Some(Geometry {
            size: Some(Size {
                width: size.width as f64,
                height: size.height as f64,
            }),
            position: Some(Position {
                x: f64::from(position.x),
                y: f64::from(position.y),
            }),
            maximized: window.is_maximized(),
            fullscreen: window.is_fullscreen(),
            minimized: false,
        })
    }

    fn apply(&self, geometry: Geometry) -> Done {
        let Some(component) = self.upgrade() else {
            // Either the window is gone or this is not its thread; both mean
            // the caller cannot be answered here.
            return Err(Unsupported);
        };
        let window = component.window();
        let geometry = fit(geometry, &monitors::work_areas());

        if let Some(size) = geometry.size {
            window.set_size(LogicalSize::new(size.width as f32, size.height as f32));
        }
        if let Some(position) = geometry.position {
            window.set_position(PhysicalPosition::new(
                position.x.round() as i32,
                position.y.round() as i32,
            ));
        }
        window.set_maximized(geometry.maximized);
        window.set_fullscreen(geometry.fullscreen);
        window.set_minimized(geometry.minimized);

        Ok(())
    }

    fn set_visible(&self, visible: bool) -> Done {
        let Some(component) = self.upgrade() else {
            return Err(Unsupported);
        };

        let shown = if visible {
            component.show()
        } else {
            component.hide()
        };
        shown.map_err(|_| Unsupported)
    }

    #[cfg(feature = "winit")]
    fn start_drag(&self) -> Done {
        let Some(component) = self.upgrade() else {
            return Err(Unsupported);
        };

        component
            .window()
            .with_winit_window(|window| window.drag_window().map_err(|_| Unsupported))
            .unwrap_or(Err(Unsupported))
    }

    /// Without the `winit` feature there is no way down to the window manager,
    /// and dragging is its gesture.
    #[cfg(not(feature = "winit"))]
    fn start_drag(&self) -> Done {
        Err(Unsupported)
    }
}

/// Keeps an eye on the windows for as long as the application runs.
///
/// With the `winit` feature the windows say when they moved and this is only a
/// slow safety net; without it, it is the only way to notice at all.
pub(crate) fn watch(windows: std::sync::Arc<SlintWindows>) -> slint::Timer {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, POLL, move || windows.poll());
    timer
}
