---
title: Страницы и лейауты
sidebar:
  order: 2
---

Страница связывает фичи с интерфейсом: устанавливает нужные ей фичи, читает
их состояние и отправляет действия из обработчиков ввода. Как описать сам
интерфейс, зависит от тулкита.

В этой главе один счётчик реализован на WinUI и egui. Во вкладках можно
сравнить код двух бэкендов. Правила установки фич, доступа к состоянию и
времени жизни страниц у них общие.

## Страница

Страница реализует трейт `Page`. В её структуре хранятся данные самого
представления. Здесь это `step` — число, которое кнопка прибавит к счётчику.
Значение счётчика хранится в фиче `Counter` из [введения](../../introduction/).

::::backends
:::winui
<!-- shown: winui: a page -->
```rust
use guinea::winui::reactor::{Button, StackPanel, TextBlock, View};
use guinea::winui::{MarkExt, Page, PageCx, UpdateCx, page};

#[derive(guinea::Mark)]
pub enum Marks {
    Add,
    Bigger,
}

pub struct CounterPage {
    step: u32,
}

pub enum Msg {
    Bigger,
}

#[page]
impl Page for CounterPage {
    type Message = Msg;

    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Counter>(&())?;
        Ok(Self { step: 1 })
    }

    fn update(&mut self, message: Msg, _cx: &mut UpdateCx<'_, Self>) {
        match message {
            Msg::Bigger => self.step += 1,
        }
    }

    fn view(&self, cx: &mut PageCx<'_, '_, Self>) -> View {
        let (count, dispatch) = cx.read::<Count>();
        let step = self.step;

        StackPanel::new()
            .children((
                TextBlock::new().text(format!("{} by {step}", count.0)),
                Button::new()
                    .content(TextBlock::new().text("Add"))
                    .on_click(move || dispatch.emit(Add(step)))
                    .mark(Marks::Add),
                Button::new()
                    .content(TextBlock::new().text("Bigger"))
                    .on_click(cx.on(|()| Msg::Bigger))
                    .mark(Marks::Bigger),
            ))
            .into()
    }
}
```
<!-- /shown -->

В бэкенде WinUI построение представления и обработка сообщений разделены:

- **`view`** строит описание интерфейса — `View`. `cx.read::<Count>()`
  возвращает состояние и `Dispatch` для действий фиче-владельцу. Чтение также
  подписывает компонент страницы на изменения `Count`: при изменении
  состояния guinea запрашивает новое представление, а Reactor обновляет
  дерево элементов.
- **`update`** меняет собственное состояние страницы. `cx.on(|()| Msg::Bigger)`
  превращает событие кнопки в сообщение `Msg::Bigger`. Обработчик увеличивает
  `step`, после чего компонент снова вызывает `view`.

Кнопка `Add` отправляет действие напрямую в фичу. Её обработчик сохраняется
после возврата из `view`, поэтому замыкание захватывает `dispatch` и `step`
через `move`. Оба значения имеют типы с `Copy` и копируются внутрь замыкания.
:::
:::egui
<!-- shown: egui: a page -->
```rust
use guinea::eframe::{Page, PageCx, page};

pub struct CounterPage {
    step: u32,
}

#[page]
impl Page for CounterPage {
    fn install(cx: &Install) -> anyhow::Result<Self> {
        cx.install::<Counter>(&())?;
        Ok(Self { step: 1 })
    }

    fn render(&mut self, cx: &mut PageCx<'_, Self>) {
        let (count, dispatch) = cx.read::<Count>();
        let ui = cx.ui();

        ui.label(format!("{} by {}", count.0, self.step));
        if ui.button("Add").clicked() {
            dispatch.emit(Add(self.step));
        }
        if ui.button("Bigger").clicked() {
            self.step += 1;
        }
    }
}
```
<!-- /shown -->

В egui описание интерфейса и обработка ввода находятся в **`render`**.
Метод получает `&mut self`, поэтому может сразу менять поля страницы.
`clicked()` сообщает о нажатии кнопки, и код обрабатывает его в этом же
вызове. Отдельные сообщения страницы и метод `update` не нужны.

`cx.read::<Count>()` возвращает состояние и `Dispatch`. В отличие от WinUI,
чтение не создаёт подписку компонента: при построении следующего кадра
`render` снова прочитает текущее значение.
:::
::::

**`install`** создаёт экземпляр страницы при первом монтировании. В примере
он устанавливает `Counter` и возвращает `CounterPage` с начальным шагом `1`.
При последующих перерисовках установка не повторяется. Ресурсы фичи живут
в скоупе страницы; подробнее об этом — в главе [«Скоупы»](../scopes/).

Две кнопки показывают разницу между состоянием страницы и фичи. `Bigger`
меняет поле `step` самой страницы. `Add` отправляет `Add(step)` фиче, которая
обрабатывает действие и обновляет `Count`.

Макрос `#[page]` заполняет необязательные части реализации: если `install`
не написан, он создаёт страницу через `Default`. Если не объявлен тип
`Params`, используется `()`.

## Лейаут

Лейаут реализует трейт `Layout`. Он размещает общие элементы интерфейса и
монтирует дочерние страницы или лейауты.

В примере `Shell` показывает кнопку перехода и одну из двух страниц.
Маршрут описан перечислением `Place`. Через `cx.outlet` лейаут получает
текущий маршрут, а в `match` выбирает, что смонтировать.

::::backends
:::winui
<!-- shown: winui: a layout -->
```rust
#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Place {
    Counter,
    About,
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn view(&self, cx: &mut LayoutCx<'_, '_, Self>) -> View {
        let nav = cx.navigate::<Place>();
        let to_about = nav.clone();

        StackPanel::new()
            .children((
                Button::new()
                    .content(TextBlock::new().text("About"))
                    .on_click(move || {
                        let _ = to_about.to(Place::About);
                    }),
                cx.outlet(&nav, |place, cx| match place {
                    Place::Counter => cx.mount::<CounterPage>(&()),
                    Place::About => cx.mount::<About>(&()),
                }),
            ))
            .into()
    }
}
```
<!-- /shown -->
:::
:::egui
<!-- shown: egui: a layout -->
```rust
#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Place {
    Counter,
    About,
}

#[derive(Default)]
pub struct Shell;

#[layout]
impl Layout for Shell {
    fn render(&mut self, cx: &mut LayoutCx<'_, Self>) {
        let nav = cx.navigate::<Place>();
        if cx.ui().button("About").clicked() {
            let _ = nav.to(Place::About);
        }

        let page = cx.outlet(&nav, |place, cx| match place {
            Place::Counter => cx.mount::<CounterPage>(&()),
            Place::About => cx.mount::<About>(&()),
        });
        page.draw(cx.ui());
    }
}
```
<!-- /shown -->

В egui `cx.outlet` возвращает `Node` — объект с отложенным вызовом
отрисовки. Метод `draw` выполняет его в переданном `ui`. Так лейаут выбирает,
где разместить дочернее содержимое.
:::
::::

`cx.navigate::<Place>()` ищет навигатор этого типа в текущем скоупе и его
предках. В примере он создан на уровне окна. Вызов `nav.to(Place::About)`
запрашивает переход. Если переход разрешён, при следующем построении
содержимого `outlet` выбирает `About`. `mount` повторно использует страницу
с тем же типом и равными параметрами или устанавливает новый экземпляр.
Страницы, которые лейаут
больше не монтирует, удаляются, а отмеченные `keep` — засыпают.

Метод `install` лейаута работает так же, как у страницы. Фичи, установленные
в лейауте, могут экспортировать состояние для его дочерних страниц.

## Приложение

Функция `app` получает `FeatureBuilder`. Через него она устанавливает
общие фичи и плагины, предоставляет данные приложения и возвращает описание
первого окна. В примере общих зависимостей нет, поэтому достаточно создать
`Window`:

::::backends
:::winui
<!-- shown: winui: an application -->
```rust
use guinea::winui::Window;

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}
```
<!-- /shown -->
:::
:::egui
<!-- shown: egui: an application -->
```rust
use guinea::eframe::Window;

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}
```
<!-- /shown -->
:::
::::

`Window::of::<Shell>()` задаёт корневой лейаут окна, а `.at(..)` создаёт
для окна навигатор с начальным маршрутом. Фичи, установленные через
`app.install`, живут в скоупе приложения, общем для всех его окон.

Точка входа исполняемого файла вызывает `run(app)` выбранного бэкенда: он
открывает окно и запускает цикл событий. Тесты ниже тоже вызывают `app`,
но монтируют возвращённое окно без системного цикла событий.

## Тесты

Харнес бэкенда позволяет выполнить код страницы без системного окна:
передать ввод, дождаться обработки действий и проверить результат
отрисовки. Он использует тот же `Harness`, что и тесты фич.

::::backends
:::winui
<!-- shown: winui: testing a page -->
```rust
#[guinea::test]
fn adds_the_step_it_was_told(h: &mut Harness) {
    let mut page = Mounted::<CounterPage>::mount(h.segment(), ()).unwrap();

    page.click(Marks::Bigger).settle();
    page.click(Marks::Add).settle();
    page.settle();

    assert!(page.find_text("2 by 2").is_some(), "{:#?}", page.tree());
}
```
<!-- /shown -->

`#[derive(guinea::Mark)]` и `.mark(..)` задают имена элементов, по которым
их находит тест. При изменении надписи или положения кнопки такое имя может
остаться прежним. Для поиска по тексту есть `click_text`.

`settle` выполняет работу, вызванную нажатием: обработку действия, обновление
редьюсера и перерисовку страницы. Порядок выполнения фоновой работы задаёт
сид теста, чтобы неудачный запуск можно было воспроизвести.

Лейаут тестируется так же, только монтируется сразу на нужный маршрут:

<!-- shown: winui: testing a layout -->
```rust
#[guinea::test]
fn the_about_button_moves_the_window(h: &mut Harness) {
    let mut shell = Mounted::<Shell>::mount_at(h.segment(), (), Place::Counter).unwrap();
    assert!(shell.find_text("0 by 1").is_some(), "{:#?}", shell.tree());

    shell.click_text("About").settle();
    shell.settle();

    assert_eq!(shell.navigator::<Place>().current(), Some(Place::About));
    assert!(shell.find_text("guinea").is_some(), "{:#?}", shell.tree());
}
```
<!-- /shown -->

`Mounted` доступен с фичей `harness`:

```toml
[dev-dependencies]
guinea = { version = "0.30", features = ["harness"] }
```

Можно проверить и само окно, которое вернула функция `app`, включая
объявленный начальный маршрут:

<!-- shown: winui: testing an application -->
```rust
#[guinea::test]
fn the_application_opens_at_its_declared_route(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut shell = Mounted::<Place>::from_window(h, window).unwrap();
    assert_eq!(shell.route(), Place::Counter);
    assert!(shell.find_text("0 by 1").is_some());

    shell.click_text("About").settle();

    assert_eq!(shell.route(), Place::About);
    assert!(shell.find_text("guinea").is_some());
}
```
<!-- /shown -->
:::
:::egui
Харнес egui выполняет кадры без системного окна. Он передаёт egui события
нажатия и отпускания кнопки мыши, а затем читает текст из фигур, которые
отрисовала страница:

<!-- shown: egui: testing a page -->
```rust
#[guinea::test]
fn adds_the_step_it_was_told(h: &mut Harness) {
    let mut page = Mounted::window(h, Window::of::<CounterPage>());

    page.click_text("Bigger");
    page.click_text("Add");

    assert!(page.has_text("2 by 2"));
}
```
<!-- /shown -->

`click_text` нажимает в центре найденного текста, выполняет ожидающую работу
фич и отрисовывает следующий кадр. Если текст не найден или совпадений
несколько, тест падает. `has_text` проверяет текст последнего кадра.

Тот же харнес монтирует окно, возвращённое `app`. Так проверяются начальный
маршрут и кнопка перехода в лейауте:

<!-- shown: egui: testing an application -->
```rust
#[guinea::test]
fn the_about_button_moves_the_window(h: &mut Harness) {
    let window = h.app(app).unwrap();
    let mut shell = Mounted::window(h, window);
    assert!(shell.has_text("0 by 1"));

    shell.click_text("About");

    assert!(shell.has_text("guinea"));
    assert!(!shell.has_text("0 by 1"));
}
```
<!-- /shown -->

Для `guinea::eframe::harness::Mounted` включи у guinea фичи `eframe` и
`harness`.
:::
::::
