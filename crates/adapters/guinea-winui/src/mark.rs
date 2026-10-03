use guinea_core::mark::Mark;
use windows_reactor::*;

/// Puts a [`Mark`] on an element, as its `AutomationId`.
pub trait MarkExt: Sized {
    fn mark(self, mark: impl Mark) -> Self;
}

macro_rules! marked {
    ($($control:ident),* $(,)?) => {
        $(
            impl MarkExt for $control {
                fn mark(self, mark: impl Mark) -> Self {
                    self.automation_id(mark.name())
                }
            }
        )*
    };
}

marked!(
    AppBarButton,
    AppBarSeparator,
    AutoSuggestBox,
    BitmapIcon,
    Border,
    BreadcrumbBar,
    Button,
    CalendarDatePicker,
    CalendarView,
    Canvas,
    CheckBox,
    ColorPicker,
    ComboBox,
    CommandBar,
    DatePicker,
    DropDownButton,
    Ellipse,
    Expander,
    FlipView,
    FontIcon,
    Grid,
    GridView,
    GridViewItem,
    HyperlinkButton,
    Image,
    ImageIcon,
    InfoBadge,
    InfoBar,
    ItemsRepeater,
    Line,
    ListBox,
    ListBoxItem,
    ListView,
    ListViewItem,
    MenuBar,
    MenuBarItem,
    NavigationView,
    NavigationViewItem,
    NumberBox,
    PasswordBox,
    PathIcon,
    PersonPicture,
    Pivot,
    PivotItem,
    ProgressBar,
    ProgressRing,
    RadioButton,
    RadioButtons,
    RatingControl,
    Rectangle,
    RelativePanel,
    RepeatButton,
    RichEditBox,
    RichTextBlock,
    ScrollView,
    ScrollViewer,
    SelectorBar,
    SelectorBarItem,
    Slider,
    SplitButton,
    SplitView,
    StackPanel,
    SwapChainPanel,
    SymbolIcon,
    TabView,
    TabViewItem,
    TeachingTip,
    TextBlock,
    TextBox,
    TimePicker,
    TitleBar,
    ToggleButton,
    ToggleSwitch,
    TreeView,
    VariableSizedWrapGrid,
    Viewbox,
    WebView2,
);
