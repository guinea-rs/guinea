//! A window's content rendered and driven without an operating-system window.

use guinea_app::app::{Harness, Segment};

use crate::run::{Window, frame};

/// A window declaration mounted under a domain harness.
///
/// ```
/// use guinea_app::app::Harness;
/// use guinea_eframe::{Layout, LayoutCx, Window, layout};
/// use guinea_eframe::harness::Mounted;
///
/// #[derive(Default)]
/// struct Greeting;
///
/// #[layout]
/// impl Layout for Greeting {
///     fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
///         cx.ui().label("Hello");
///     }
/// }
///
/// let harness = Harness::new(0);
/// let mut page = Mounted::window(&harness, Window::of::<Greeting>());
/// assert!(page.has_text("Hello"));
/// page.settle();
/// assert!(page.has_text("Hello"));
/// ```
pub struct Mounted<'h> {
    segment: Segment<'h>,
    window: Window,
    context: egui::Context,
    text: Vec<(String, egui::Rect)>,
    _navigator: Option<std::rc::Rc<dyn guinea_router::navigator::Asking>>,
}

impl<'h> Mounted<'h> {
    /// Installs and draws the declared root and its initial route.
    /// See [`Mounted`] for a complete example.
    pub fn window(harness: &'h Harness, mut window: Window) -> Self {
        let segment = harness.child();
        let navigator = window.start.take().map(|start| start(segment.scope()));
        let mut mounted = Self {
            segment,
            window,
            context: egui::Context::default(),
            text: Vec::new(),
            _navigator: navigator,
        };
        mounted.settle();
        mounted
    }

    /// Whether the latest frame drew this exact text.
    /// See [`Mounted`] for a complete example.
    pub fn has_text(&self, text: &str) -> bool {
        self.text.iter().any(|(shown, _)| shown == text)
    }

    /// Runs pending domain work and redraws the window.
    /// See [`Mounted`] for a complete example.
    pub fn settle(&mut self) {
        self.segment.harness().settled();
        self.draw(Vec::new());
    }

    /// Presses and releases the primary pointer button at the text's center,
    /// then runs the work and redraws. Panics if the text is missing or ambiguous.
    ///
    /// ```
    /// use guinea_app::app::Harness;
    /// use guinea_eframe::{Layout, LayoutCx, Window, layout};
    /// use guinea_eframe::harness::Mounted;
    ///
    /// #[derive(Default)]
    /// struct Counter(u32);
    ///
    /// #[layout]
    /// impl Layout for Counter {
    ///     fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
    ///         if cx.ui().button("Add").clicked() {
    ///             self.0 += 1;
    ///         }
    ///         cx.ui().label(format!("Count: {}", self.0));
    ///     }
    /// }
    ///
    /// let harness = Harness::new(0);
    /// let mut page = Mounted::window(&harness, Window::of::<Counter>());
    /// page.click_text("Add");
    /// assert!(page.has_text("Count: 1"));
    /// ```
    pub fn click_text(&mut self, text: &str) {
        let found: Vec<_> = self
            .text
            .iter()
            .filter(|(shown, _)| shown == text)
            .collect();
        assert_eq!(
            found.len(),
            1,
            "expected exactly one `{text}`, found {}",
            found.len()
        );
        let position = found[0].1.center();

        self.draw(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        self.draw(vec![egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.settle();
    }

    fn draw(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let scope = self.segment.scope();
        let window = &self.window;
        let mut output = self
            .context
            .run_ui(input, |ui| frame(scope, &window.root, ui));
        output.textures_delta.clear();
        self.text.clear();
        for shape in output.shapes {
            text_in(&shape.shape, &mut self.text);
        }
    }
}

fn text_in(shape: &egui::epaint::Shape, text: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::epaint::Shape::Text(shown) => {
            text.push((
                shown.galley.text().to_owned(),
                shown.galley.rect.translate(shown.pos.to_vec2()),
            ));
        }
        egui::epaint::Shape::Vec(shapes) => {
            for child in shapes {
                text_in(child, text);
            }
        }
        _ => {}
    }
}
