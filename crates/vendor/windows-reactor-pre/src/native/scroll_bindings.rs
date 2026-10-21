#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentControl(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    ContentControl,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(
    ContentControl,
    Control,
    FrameworkElement,
    UIElement,
    DependencyObject
);
impl windows_core::RuntimeType for ContentControl {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IContentControl>();
}
unsafe impl windows_core::Interface for ContentControl {
    type Vtable = <IContentControl as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IContentControl as windows_core::Interface>::IID;
}
impl core::ops::Deref for ContentControl {
    type Target = IContentControl;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for ContentControl {
    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ContentControl";
}
unsafe impl Send for ContentControl {}
unsafe impl Sync for ContentControl {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Control(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    Control,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(Control, FrameworkElement, UIElement, DependencyObject);
impl windows_core::RuntimeType for Control {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IControl>();
}
unsafe impl windows_core::Interface for Control {
    type Vtable = <IControl as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IControl as windows_core::Interface>::IID;
}
impl core::ops::Deref for Control {
    type Target = IControl;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for Control {
    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.Control";
}
unsafe impl Send for Control {}
unsafe impl Sync for Control {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyObject(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    DependencyObject,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl windows_core::RuntimeType for DependencyObject {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IDependencyObject>();
}
unsafe impl windows_core::Interface for DependencyObject {
    type Vtable = <IDependencyObject as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IDependencyObject as windows_core::Interface>::IID;
}
impl core::ops::Deref for DependencyObject {
    type Target = IDependencyObject;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for DependencyObject {
    const NAME: &'static str = "Microsoft.UI.Xaml.DependencyObject";
}
unsafe impl Send for DependencyObject {}
unsafe impl Sync for DependencyObject {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventHandler<T>(windows_core::IUnknown, core::marker::PhantomData<T>)
where
    T: windows_core::RuntimeType + 'static;
unsafe impl<T: windows_core::RuntimeType + 'static> windows_core::Interface for EventHandler<T> {
    type Vtable = EventHandler_Vtbl<T>;
    const IID: windows_core::GUID =
        windows_core::GUID::from_signature(<Self as windows_core::RuntimeType>::SIGNATURE);
}
impl<T: windows_core::RuntimeType + 'static> windows_core::RuntimeType for EventHandler<T> {
    const SIGNATURE: windows_core::imp::ConstBuffer = windows_core::imp::ConstBuffer::new()
        .push_slice(b"pinterface({9de1c535-6ae1-11e0-84e1-18a905bcc53f}")
        .push_slice(b";")
        .push_other(T::SIGNATURE)
        .push_slice(b")");
}
#[repr(C)]
pub struct EventHandler_Vtbl<T>
where
    T: windows_core::RuntimeType + 'static,
{
    base__: windows_core::IUnknown_Vtbl,
    Invoke: unsafe extern "system" fn(
        this: *mut core::ffi::c_void,
        sender: *mut core::ffi::c_void,
        args: windows_core::imp::AbiType<T>,
    ) -> windows_core::HRESULT,
    T: core::marker::PhantomData<T>,
}
struct EventHandlerBox<
    T,
    F: Fn(windows_core::Ref<windows_core::IInspectable>, windows_core::Ref<T>) + 'static,
>(core::marker::PhantomData<(T, fn() -> F)>)
where
    T: windows_core::RuntimeType + 'static;
impl<
    T: windows_core::RuntimeType + 'static,
    F: Fn(windows_core::Ref<windows_core::IInspectable>, windows_core::Ref<T>) + 'static,
> EventHandlerBox<T, F>
{
    const VTABLE: EventHandler_Vtbl<T> = EventHandler_Vtbl::<T> {
        base__: windows_core::IUnknown_Vtbl {
            QueryInterface: windows_core::imp::DelegateBox::<EventHandler<T>, F>::QueryInterface,
            AddRef: windows_core::imp::DelegateBox::<EventHandler<T>, F>::AddRef,
            Release: windows_core::imp::DelegateBox::<EventHandler<T>, F>::Release,
        },
        Invoke: Self::Invoke,
        T: core::marker::PhantomData::<T>,
    };
    unsafe extern "system" fn Invoke(
        this: *mut core::ffi::c_void,
        sender: *mut core::ffi::c_void,
        args: windows_core::imp::AbiType<T>,
    ) -> windows_core::HRESULT {
        unsafe {
            let this = &mut *(this as *mut *mut core::ffi::c_void
                as *mut windows_core::imp::DelegateBox<EventHandler<T>, F>);
            (this.invoke)(
                core::mem::transmute_copy(&sender),
                core::mem::transmute_copy(&args),
            );
            windows_core::HRESULT(0)
        }
    }
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameworkElement(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    FrameworkElement,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(FrameworkElement, UIElement, DependencyObject);
impl windows_core::RuntimeType for FrameworkElement {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IFrameworkElement>();
}
unsafe impl windows_core::Interface for FrameworkElement {
    type Vtable = <IFrameworkElement as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IFrameworkElement as windows_core::Interface>::IID;
}
impl core::ops::Deref for FrameworkElement {
    type Target = IFrameworkElement;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for FrameworkElement {
    const NAME: &'static str = "Microsoft.UI.Xaml.FrameworkElement";
}
unsafe impl Send for FrameworkElement {}
unsafe impl Sync for FrameworkElement {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneralTransform(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    GeneralTransform,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(GeneralTransform, DependencyObject);
impl windows_core::RuntimeType for GeneralTransform {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IGeneralTransform>();
}
unsafe impl windows_core::Interface for GeneralTransform {
    type Vtable = <IGeneralTransform as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IGeneralTransform as windows_core::Interface>::IID;
}
impl core::ops::Deref for GeneralTransform {
    type Target = IGeneralTransform;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for GeneralTransform {
    const NAME: &'static str = "Microsoft.UI.Xaml.Media.GeneralTransform";
}
unsafe impl Send for GeneralTransform {}
unsafe impl Sync for GeneralTransform {}
windows_core::imp::define_interface!(
    IContentControl,
    IContentControl_Vtbl,
    0x07e81761_11b2_52ae_8f8b_4d53d2b5900a
);
impl windows_core::RuntimeType for IContentControl {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
#[repr(C)]
pub struct IContentControl_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IControl,
    IControl_Vtbl,
    0x857d6e8a_d45a_5c69_a99c_bf6a5c54fb38
);
impl windows_core::RuntimeType for IControl {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
#[repr(C)]
pub struct IControl_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IDependencyObject,
    IDependencyObject_Vtbl,
    0xe7beaee7_160e_50f7_8789_d63463f979fa
);
impl windows_core::RuntimeType for IDependencyObject {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
#[repr(C)]
pub struct IDependencyObject_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
}
windows_core::imp::define_interface!(
    IFrameworkElement,
    IFrameworkElement_Vtbl,
    0xfe08f13d_dc6a_5495_ad44_c2d8d21863b0
);
impl windows_core::RuntimeType for IFrameworkElement {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
impl IFrameworkElement {
    pub(crate) fn LayoutUpdated<F>(
        &self,
        handler: F,
    ) -> windows_core::Result<windows_core::EventRevoker>
    where
        F: Fn(
                windows_core::Ref<windows_core::IInspectable>,
                windows_core::Ref<windows_core::IInspectable>,
            ) + 'static,
    {
        let handler: EventHandler<windows_core::IInspectable> = {
            let com =
                windows_core::imp::DelegateBox::<EventHandler<windows_core::IInspectable>, F>::new(
                    &EventHandlerBox::<windows_core::IInspectable, F>::VTABLE,
                    handler,
                );
            unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
        };
        unsafe {
            let mut result__ = core::mem::zeroed();
            let token__ = (windows_core::Interface::vtable(self).LayoutUpdated)(
                windows_core::Interface::as_raw(self),
                windows_core::Interface::as_raw(&handler),
                &mut result__,
            )
            .map(|| result__)?;
            Ok(windows_core::EventRevoker::new(
                self.clone(),
                token__,
                windows_core::Interface::vtable(self).RemoveLayoutUpdated,
            ))
        }
    }
}
#[repr(C)]
pub struct IFrameworkElement_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    Triggers: usize,
    Resources: usize,
    SetResources: usize,
    Tag: usize,
    SetTag: usize,
    Language: usize,
    SetLanguage: usize,
    ActualWidth: usize,
    ActualHeight: usize,
    Width: usize,
    SetWidth: usize,
    Height: usize,
    SetHeight: usize,
    MinWidth: usize,
    SetMinWidth: usize,
    MaxWidth: usize,
    SetMaxWidth: usize,
    MinHeight: usize,
    SetMinHeight: usize,
    MaxHeight: usize,
    SetMaxHeight: usize,
    HorizontalAlignment: usize,
    SetHorizontalAlignment: usize,
    VerticalAlignment: usize,
    SetVerticalAlignment: usize,
    Margin: usize,
    SetMargin: usize,
    Name: usize,
    SetName: usize,
    BaseUri: usize,
    DataContext: usize,
    SetDataContext: usize,
    AllowFocusOnInteraction: usize,
    SetAllowFocusOnInteraction: usize,
    FocusVisualMargin: usize,
    SetFocusVisualMargin: usize,
    FocusVisualSecondaryThickness: usize,
    SetFocusVisualSecondaryThickness: usize,
    FocusVisualPrimaryThickness: usize,
    SetFocusVisualPrimaryThickness: usize,
    FocusVisualSecondaryBrush: usize,
    SetFocusVisualSecondaryBrush: usize,
    FocusVisualPrimaryBrush: usize,
    SetFocusVisualPrimaryBrush: usize,
    AllowFocusWhenDisabled: usize,
    SetAllowFocusWhenDisabled: usize,
    Style: usize,
    SetStyle: usize,
    Parent: usize,
    FlowDirection: usize,
    SetFlowDirection: usize,
    RequestedTheme: usize,
    SetRequestedTheme: usize,
    IsLoaded: usize,
    ActualTheme: usize,
    Loaded: usize,
    RemoveLoaded: usize,
    Unloaded: usize,
    RemoveUnloaded: usize,
    DataContextChanged: usize,
    RemoveDataContextChanged: usize,
    SizeChanged: usize,
    RemoveSizeChanged: usize,
    pub LayoutUpdated: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut i64,
    ) -> windows_core::HRESULT,
    pub RemoveLayoutUpdated:
        unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IGeneralTransform,
    IGeneralTransform_Vtbl,
    0x04eedeeb_31e5_54c0_ae3f_8bd06645d339
);
impl windows_core::RuntimeType for IGeneralTransform {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
impl IGeneralTransform {
    pub(crate) fn TransformPoint(&self, point: Point) -> windows_core::Result<Point> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).TransformPoint)(
                windows_core::Interface::as_raw(self),
                point,
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IGeneralTransform_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    Inverse: usize,
    pub TransformPoint: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        Point,
        *mut Point,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IScrollViewer,
    IScrollViewer_Vtbl,
    0x1dc28c2e_996c_5394_89c3_4dc656b4ad46
);
impl windows_core::RuntimeType for IScrollViewer {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
impl IScrollViewer {
    pub(crate) fn HorizontalOffset(&self) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).HorizontalOffset)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub(crate) fn ViewportWidth(&self) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ViewportWidth)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub(crate) fn ExtentWidth(&self) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ExtentWidth)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub(crate) fn VerticalOffset(&self) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).VerticalOffset)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub(crate) fn ViewportHeight(&self) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ViewportHeight)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub(crate) fn ExtentHeight(&self) -> windows_core::Result<f64> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ExtentHeight)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
    pub(crate) fn ViewChanged<F>(
        &self,
        handler: F,
    ) -> windows_core::Result<windows_core::EventRevoker>
    where
        F: Fn(
                windows_core::Ref<windows_core::IInspectable>,
                windows_core::Ref<ScrollViewerViewChangedEventArgs>,
            ) + 'static,
    {
        let handler: EventHandler<ScrollViewerViewChangedEventArgs> = {
            let com = windows_core::imp::DelegateBox::<
                EventHandler<ScrollViewerViewChangedEventArgs>,
                F,
            >::new(
                &EventHandlerBox::<ScrollViewerViewChangedEventArgs, F>::VTABLE,
                handler,
            );
            unsafe { core::mem::transmute(windows_core::imp::box_new(com)) }
        };
        unsafe {
            let mut result__ = core::mem::zeroed();
            let token__ = (windows_core::Interface::vtable(self).ViewChanged)(
                windows_core::Interface::as_raw(self),
                windows_core::Interface::as_raw(&handler),
                &mut result__,
            )
            .map(|| result__)?;
            Ok(windows_core::EventRevoker::new(
                self.clone(),
                token__,
                windows_core::Interface::vtable(self).RemoveViewChanged,
            ))
        }
    }
    pub(crate) fn ChangeViewWithOptionalAnimation(
        &self,
        horizontaloffset: Option<f64>,
        verticaloffset: Option<f64>,
        zoomfactor: Option<f32>,
        disableanimation: bool,
    ) -> windows_core::Result<bool> {
        let horizontaloffset__ =
            horizontaloffset.map(<windows_reference::IReference<f64> as From<_>>::from);
        let verticaloffset__ =
            verticaloffset.map(<windows_reference::IReference<f64> as From<_>>::from);
        let zoomfactor__ = zoomfactor.map(<windows_reference::IReference<f32> as From<_>>::from);
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).ChangeViewWithOptionalAnimation)(
                windows_core::Interface::as_raw(self),
                windows_core::Param::param(horizontaloffset__.as_ref()).abi(),
                windows_core::Param::param(verticaloffset__.as_ref()).abi(),
                windows_core::Param::param(zoomfactor__.as_ref()).abi(),
                disableanimation,
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IScrollViewer_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    HorizontalScrollBarVisibility: usize,
    SetHorizontalScrollBarVisibility: usize,
    VerticalScrollBarVisibility: usize,
    SetVerticalScrollBarVisibility: usize,
    IsHorizontalRailEnabled: usize,
    SetIsHorizontalRailEnabled: usize,
    IsVerticalRailEnabled: usize,
    SetIsVerticalRailEnabled: usize,
    IsHorizontalScrollChainingEnabled: usize,
    SetIsHorizontalScrollChainingEnabled: usize,
    IsVerticalScrollChainingEnabled: usize,
    SetIsVerticalScrollChainingEnabled: usize,
    IsZoomChainingEnabled: usize,
    SetIsZoomChainingEnabled: usize,
    IsScrollInertiaEnabled: usize,
    SetIsScrollInertiaEnabled: usize,
    IsZoomInertiaEnabled: usize,
    SetIsZoomInertiaEnabled: usize,
    HorizontalScrollMode: usize,
    SetHorizontalScrollMode: usize,
    VerticalScrollMode: usize,
    SetVerticalScrollMode: usize,
    ZoomMode: usize,
    SetZoomMode: usize,
    HorizontalSnapPointsAlignment: usize,
    SetHorizontalSnapPointsAlignment: usize,
    VerticalSnapPointsAlignment: usize,
    SetVerticalSnapPointsAlignment: usize,
    HorizontalSnapPointsType: usize,
    SetHorizontalSnapPointsType: usize,
    VerticalSnapPointsType: usize,
    SetVerticalSnapPointsType: usize,
    ZoomSnapPointsType: usize,
    SetZoomSnapPointsType: usize,
    pub HorizontalOffset:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut f64) -> windows_core::HRESULT,
    pub ViewportWidth:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut f64) -> windows_core::HRESULT,
    ScrollableWidth: usize,
    ComputedHorizontalScrollBarVisibility: usize,
    pub ExtentWidth:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut f64) -> windows_core::HRESULT,
    pub VerticalOffset:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut f64) -> windows_core::HRESULT,
    pub ViewportHeight:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut f64) -> windows_core::HRESULT,
    ScrollableHeight: usize,
    ComputedVerticalScrollBarVisibility: usize,
    pub ExtentHeight:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut f64) -> windows_core::HRESULT,
    MinZoomFactor: usize,
    SetMinZoomFactor: usize,
    MaxZoomFactor: usize,
    SetMaxZoomFactor: usize,
    ZoomFactor: usize,
    ZoomSnapPoints: usize,
    TopLeftHeader: usize,
    SetTopLeftHeader: usize,
    LeftHeader: usize,
    SetLeftHeader: usize,
    TopHeader: usize,
    SetTopHeader: usize,
    ReduceViewportForCoreInputViewOcclusions: usize,
    SetReduceViewportForCoreInputViewOcclusions: usize,
    HorizontalAnchorRatio: usize,
    SetHorizontalAnchorRatio: usize,
    VerticalAnchorRatio: usize,
    SetVerticalAnchorRatio: usize,
    CanContentRenderOutsideBounds: usize,
    SetCanContentRenderOutsideBounds: usize,
    AnchorRequested: usize,
    RemoveAnchorRequested: usize,
    ViewChanging: usize,
    RemoveViewChanging: usize,
    pub ViewChanged: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut i64,
    ) -> windows_core::HRESULT,
    pub RemoveViewChanged:
        unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> windows_core::HRESULT,
    DirectManipulationStarted: usize,
    RemoveDirectManipulationStarted: usize,
    DirectManipulationCompleted: usize,
    RemoveDirectManipulationCompleted: usize,
    ScrollToHorizontalOffset: usize,
    ScrollToVerticalOffset: usize,
    ZoomToFactor: usize,
    ChangeView: usize,
    pub ChangeViewWithOptionalAnimation: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        bool,
        *mut bool,
    ) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IScrollViewerViewChangedEventArgs,
    IScrollViewerViewChangedEventArgs_Vtbl,
    0xbf7bb85b_1d46_5004_a370_ecb626630588
);
impl windows_core::RuntimeType for IScrollViewerViewChangedEventArgs {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
impl IScrollViewerViewChangedEventArgs {
    pub(crate) fn IsIntermediate(&self) -> windows_core::Result<bool> {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).IsIntermediate)(
                windows_core::Interface::as_raw(self),
                &mut result__,
            )
            .map(|| result__)
        }
    }
}
#[repr(C)]
pub struct IScrollViewerViewChangedEventArgs_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    pub IsIntermediate:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut bool) -> windows_core::HRESULT,
}
windows_core::imp::define_interface!(
    IUIElement,
    IUIElement_Vtbl,
    0xc3c01020_320c_5cf6_9d24_d396bbfa4d8b
);
impl windows_core::RuntimeType for IUIElement {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_interface::<Self>();
}
impl IUIElement {
    pub(crate) fn TransformToVisual<P0>(&self, visual: P0) -> windows_core::Result<GeneralTransform>
    where
        P0: windows_core::Param<UIElement>,
    {
        unsafe {
            let mut result__ = core::mem::zeroed();
            (windows_core::Interface::vtable(self).TransformToVisual)(
                windows_core::Interface::as_raw(self),
                visual.param().abi(),
                &mut result__,
            )
            .and_then(|| windows_core::imp::Type::from_abi(result__))
        }
    }
}
#[repr(C)]
pub struct IUIElement_Vtbl {
    pub base__: windows_core::IInspectable_Vtbl,
    DesiredSize: usize,
    AllowDrop: usize,
    SetAllowDrop: usize,
    Opacity: usize,
    SetOpacity: usize,
    Clip: usize,
    SetClip: usize,
    RenderTransform: usize,
    SetRenderTransform: usize,
    Projection: usize,
    SetProjection: usize,
    Transform3D: usize,
    SetTransform3D: usize,
    RenderTransformOrigin: usize,
    SetRenderTransformOrigin: usize,
    IsHitTestVisible: usize,
    SetIsHitTestVisible: usize,
    Visibility: usize,
    SetVisibility: usize,
    RenderSize: usize,
    UseLayoutRounding: usize,
    SetUseLayoutRounding: usize,
    Transitions: usize,
    SetTransitions: usize,
    CacheMode: usize,
    SetCacheMode: usize,
    IsTapEnabled: usize,
    SetIsTapEnabled: usize,
    IsDoubleTapEnabled: usize,
    SetIsDoubleTapEnabled: usize,
    CanDrag: usize,
    SetCanDrag: usize,
    IsRightTapEnabled: usize,
    SetIsRightTapEnabled: usize,
    IsHoldingEnabled: usize,
    SetIsHoldingEnabled: usize,
    ManipulationMode: usize,
    SetManipulationMode: usize,
    PointerCaptures: usize,
    ContextFlyout: usize,
    SetContextFlyout: usize,
    CompositeMode: usize,
    SetCompositeMode: usize,
    Lights: usize,
    CanBeScrollAnchor: usize,
    SetCanBeScrollAnchor: usize,
    ExitDisplayModeOnAccessKeyInvoked: usize,
    SetExitDisplayModeOnAccessKeyInvoked: usize,
    IsAccessKeyScope: usize,
    SetIsAccessKeyScope: usize,
    AccessKeyScopeOwner: usize,
    SetAccessKeyScopeOwner: usize,
    AccessKey: usize,
    SetAccessKey: usize,
    KeyTipPlacementMode: usize,
    SetKeyTipPlacementMode: usize,
    KeyTipHorizontalOffset: usize,
    SetKeyTipHorizontalOffset: usize,
    KeyTipVerticalOffset: usize,
    SetKeyTipVerticalOffset: usize,
    KeyTipTarget: usize,
    SetKeyTipTarget: usize,
    XYFocusKeyboardNavigation: usize,
    SetXYFocusKeyboardNavigation: usize,
    XYFocusUpNavigationStrategy: usize,
    SetXYFocusUpNavigationStrategy: usize,
    XYFocusDownNavigationStrategy: usize,
    SetXYFocusDownNavigationStrategy: usize,
    XYFocusLeftNavigationStrategy: usize,
    SetXYFocusLeftNavigationStrategy: usize,
    XYFocusRightNavigationStrategy: usize,
    SetXYFocusRightNavigationStrategy: usize,
    KeyboardAccelerators: usize,
    KeyboardAcceleratorPlacementTarget: usize,
    SetKeyboardAcceleratorPlacementTarget: usize,
    KeyboardAcceleratorPlacementMode: usize,
    SetKeyboardAcceleratorPlacementMode: usize,
    HighContrastAdjustment: usize,
    SetHighContrastAdjustment: usize,
    TabFocusNavigation: usize,
    SetTabFocusNavigation: usize,
    OpacityTransition: usize,
    SetOpacityTransition: usize,
    Translation: usize,
    SetTranslation: usize,
    TranslationTransition: usize,
    SetTranslationTransition: usize,
    Rotation: usize,
    SetRotation: usize,
    RotationTransition: usize,
    SetRotationTransition: usize,
    Scale: usize,
    SetScale: usize,
    ScaleTransition: usize,
    SetScaleTransition: usize,
    TransformMatrix: usize,
    SetTransformMatrix: usize,
    CenterPoint: usize,
    SetCenterPoint: usize,
    RotationAxis: usize,
    SetRotationAxis: usize,
    ActualOffset: usize,
    ActualSize: usize,
    XamlRoot: usize,
    SetXamlRoot: usize,
    Shadow: usize,
    SetShadow: usize,
    RasterizationScale: usize,
    SetRasterizationScale: usize,
    FocusState: usize,
    UseSystemFocusVisuals: usize,
    SetUseSystemFocusVisuals: usize,
    XYFocusLeft: usize,
    SetXYFocusLeft: usize,
    XYFocusRight: usize,
    SetXYFocusRight: usize,
    XYFocusUp: usize,
    SetXYFocusUp: usize,
    XYFocusDown: usize,
    SetXYFocusDown: usize,
    IsTabStop: usize,
    SetIsTabStop: usize,
    TabIndex: usize,
    SetTabIndex: usize,
    KeyUp: usize,
    RemoveKeyUp: usize,
    KeyDown: usize,
    RemoveKeyDown: usize,
    GotFocus: usize,
    RemoveGotFocus: usize,
    LostFocus: usize,
    RemoveLostFocus: usize,
    DragStarting: usize,
    RemoveDragStarting: usize,
    DropCompleted: usize,
    RemoveDropCompleted: usize,
    CharacterReceived: usize,
    RemoveCharacterReceived: usize,
    DragEnter: usize,
    RemoveDragEnter: usize,
    DragLeave: usize,
    RemoveDragLeave: usize,
    DragOver: usize,
    RemoveDragOver: usize,
    Drop: usize,
    RemoveDrop: usize,
    PointerPressed: usize,
    RemovePointerPressed: usize,
    PointerMoved: usize,
    RemovePointerMoved: usize,
    PointerReleased: usize,
    RemovePointerReleased: usize,
    PointerEntered: usize,
    RemovePointerEntered: usize,
    PointerExited: usize,
    RemovePointerExited: usize,
    PointerCaptureLost: usize,
    RemovePointerCaptureLost: usize,
    PointerCanceled: usize,
    RemovePointerCanceled: usize,
    PointerWheelChanged: usize,
    RemovePointerWheelChanged: usize,
    Tapped: usize,
    RemoveTapped: usize,
    DoubleTapped: usize,
    RemoveDoubleTapped: usize,
    Holding: usize,
    RemoveHolding: usize,
    ContextRequested: usize,
    RemoveContextRequested: usize,
    ContextCanceled: usize,
    RemoveContextCanceled: usize,
    RightTapped: usize,
    RemoveRightTapped: usize,
    ManipulationStarting: usize,
    RemoveManipulationStarting: usize,
    ManipulationInertiaStarting: usize,
    RemoveManipulationInertiaStarting: usize,
    ManipulationStarted: usize,
    RemoveManipulationStarted: usize,
    ManipulationDelta: usize,
    RemoveManipulationDelta: usize,
    ManipulationCompleted: usize,
    RemoveManipulationCompleted: usize,
    AccessKeyDisplayRequested: usize,
    RemoveAccessKeyDisplayRequested: usize,
    AccessKeyDisplayDismissed: usize,
    RemoveAccessKeyDisplayDismissed: usize,
    AccessKeyInvoked: usize,
    RemoveAccessKeyInvoked: usize,
    ProcessKeyboardAccelerators: usize,
    RemoveProcessKeyboardAccelerators: usize,
    GettingFocus: usize,
    RemoveGettingFocus: usize,
    LosingFocus: usize,
    RemoveLosingFocus: usize,
    NoFocusCandidateFound: usize,
    RemoveNoFocusCandidateFound: usize,
    PreviewKeyDown: usize,
    RemovePreviewKeyDown: usize,
    PreviewKeyUp: usize,
    RemovePreviewKeyUp: usize,
    BringIntoViewRequested: usize,
    RemoveBringIntoViewRequested: usize,
    Measure: usize,
    Arrange: usize,
    CapturePointer: usize,
    ReleasePointerCapture: usize,
    ReleasePointerCaptures: usize,
    AddHandler: usize,
    RemoveHandler: usize,
    pub TransformToVisual: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        *mut core::ffi::c_void,
        *mut *mut core::ffi::c_void,
    ) -> windows_core::HRESULT,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
impl windows_core::imp::TypeKind for Point {
    type TypeKind = windows_core::imp::CopyType;
}
impl windows_core::RuntimeType for Point {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::from_slice(b"struct(Windows.Foundation.Point;f4;f4)");
}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrollViewer(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    ScrollViewer,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(
    ScrollViewer,
    ContentControl,
    Control,
    FrameworkElement,
    UIElement,
    DependencyObject
);
impl windows_core::RuntimeType for ScrollViewer {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IScrollViewer>();
}
unsafe impl windows_core::Interface for ScrollViewer {
    type Vtable = <IScrollViewer as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IScrollViewer as windows_core::Interface>::IID;
}
impl core::ops::Deref for ScrollViewer {
    type Target = IScrollViewer;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for ScrollViewer {
    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ScrollViewer";
}
unsafe impl Send for ScrollViewer {}
unsafe impl Sync for ScrollViewer {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScrollViewerViewChangedEventArgs(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    ScrollViewerViewChangedEventArgs,
    windows_core::IUnknown,
    windows_core::IInspectable
);
impl windows_core::RuntimeType for ScrollViewerViewChangedEventArgs {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IScrollViewerViewChangedEventArgs>();
}
unsafe impl windows_core::Interface for ScrollViewerViewChangedEventArgs {
    type Vtable = <IScrollViewerViewChangedEventArgs as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID =
        <IScrollViewerViewChangedEventArgs as windows_core::Interface>::IID;
}
impl core::ops::Deref for ScrollViewerViewChangedEventArgs {
    type Target = IScrollViewerViewChangedEventArgs;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for ScrollViewerViewChangedEventArgs {
    const NAME: &'static str = "Microsoft.UI.Xaml.Controls.ScrollViewerViewChangedEventArgs";
}
unsafe impl Send for ScrollViewerViewChangedEventArgs {}
unsafe impl Sync for ScrollViewerViewChangedEventArgs {}
#[repr(transparent)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UIElement(windows_core::IUnknown);
windows_core::imp::interface_hierarchy!(
    UIElement,
    windows_core::IUnknown,
    windows_core::IInspectable
);
windows_core::imp::required_hierarchy!(UIElement, DependencyObject);
impl windows_core::RuntimeType for UIElement {
    const SIGNATURE: windows_core::imp::ConstBuffer =
        windows_core::imp::ConstBuffer::for_class::<Self, IUIElement>();
}
unsafe impl windows_core::Interface for UIElement {
    type Vtable = <IUIElement as windows_core::Interface>::Vtable;
    const IID: windows_core::GUID = <IUIElement as windows_core::Interface>::IID;
}
impl core::ops::Deref for UIElement {
    type Target = IUIElement;
    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute(self) }
    }
}
impl windows_core::RuntimeName for UIElement {
    const NAME: &'static str = "Microsoft.UI.Xaml.UIElement";
}
unsafe impl Send for UIElement {}
unsafe impl Sync for UIElement {}
