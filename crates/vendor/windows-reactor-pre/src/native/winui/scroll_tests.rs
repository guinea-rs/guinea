use super::*;

#[test]
#[ignore = "opens a native WinUI window; run alone on an interactive Windows desktop"]
fn native_scroll_viewer_moves_full_height_content_and_reports_layout_changes() {
    let verified = Rc::new(Cell::new(false));
    let result = Rc::clone(&verified);
    App::run_with(move |context| {
        let reference = ElementRef::<ScrollViewer>::new();
        let content = ElementRef::<Grid>::new();
        let snapshots = Rc::new(RefCell::new(Vec::new()));
        let received = Rc::clone(&snapshots);
        let observation = reference.observe_scroll_viewer(move |metrics| {
            received.borrow_mut().push(metrics);
        });
        let view = |height, viewport| {
            ScrollViewer::new()
                .height(viewport)
                .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                .element_ref(&reference)
                .content(Grid::new().height(height).element_ref(&content))
        };
        let mut runtime = Runtime::new(WinUiAdapter::default());
        runtime.update(view(10_000.0, 300.0)).unwrap();
        let window = runtime
            .adapter()
            .create_window_with_policy(
                runtime.graph().root().unwrap(),
                &WindowPolicy::new().client_size(400.0, 400.0),
            )
            .unwrap();
        window.activate().unwrap();
        let runtime = Rc::new(RefCell::new(runtime));
        let tick_runtime = Rc::clone(&runtime);
        let ticks = Cell::new(0);
        let stage = Cell::new(0);
        let accepted = Rc::new(Cell::new(None));
        let tick_context = context.clone();
        let timer = subscribe_live_tick(move || {
            ticks.set(ticks.get() + 1);
            assert!(
                ticks.get() < 300,
                "native scroll test timed out: {:?}",
                snapshots.borrow()
            );
            let mut runtime = tick_runtime.borrow_mut();
            runtime.dispatch_native_events().unwrap();
            let Some(metrics) = snapshots.borrow().last().copied() else {
                return;
            };
            match stage.get() {
                0 if metrics.extent_height == 10_000.0 && metrics.viewport_height > 0.0 => {
                    assert!(metrics.viewport_height <= 300.0);
                    let received = Rc::clone(&accepted);
                    assert!(reference.request_change_view(500.0, true, move |result| {
                        received.set(Some(result));
                    }));
                    stage.set(1);
                }
                1 if metrics.vertical_offset == 500.0 && !metrics.is_intermediate => {
                    assert_eq!(accepted.get(), Some(Ok(true)));
                    let child = runtime
                        .adapter()
                        .ui_element(content.get().unwrap())
                        .unwrap();
                    let viewer = runtime
                        .adapter()
                        .ui_element(reference.get().unwrap())
                        .unwrap();
                    let child = child
                        .cast::<super::super::scroll_bindings::UIElement>()
                        .unwrap();
                    let viewer = viewer
                        .cast::<super::super::scroll_bindings::UIElement>()
                        .unwrap();
                    let origin = child
                        .TransformToVisual(&viewer)
                        .unwrap()
                        .TransformPoint(super::super::scroll_bindings::Point { x: 0.0, y: 0.0 })
                        .unwrap();
                    assert!(
                        (f64::from(origin.y) + 500.0).abs() < 1.0,
                        "content origin: {origin:?}"
                    );
                    let next = ScrollViewer::new()
                        .height(300.0)
                        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                        .element_ref(&reference)
                        .content(Grid::new().height(20_000.0).element_ref(&content));
                    runtime.update(next).unwrap();
                    stage.set(2);
                }
                2 if metrics.extent_height == 20_000.0 && metrics.viewport_height == 300.0 => {
                    assert_eq!(metrics.vertical_offset, 500.0);
                    let next = ScrollViewer::new()
                        .height(250.0)
                        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
                        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
                        .element_ref(&reference)
                        .content(Grid::new().height(20_000.0).element_ref(&content));
                    runtime.update(next).unwrap();
                    stage.set(3);
                }
                3 if metrics.extent_height == 20_000.0 && metrics.viewport_height == 250.0 => {
                    assert_eq!(metrics.vertical_offset, 500.0);
                    assert!(snapshots.borrow().windows(2).all(|pair| pair[0] != pair[1]));
                    result.set(true);
                    tick_context.exit().unwrap();
                    stage.set(4);
                }
                _ => {}
            }
        })?;
        Ok((runtime, window, observation, timer))
    })
    .unwrap();
    assert!(verified.get());
}
