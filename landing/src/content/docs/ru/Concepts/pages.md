---
title: Страницы и лейауты
sidebar:
  order: 2
---

Фича хранит данные приложения, страница их показывает. Рисует страницу
тулкит, и у каждого бэкенда это устроено по-своему, поэтому ниже один и тот
же счётчик показан для каждого из них во вкладках. Всё остальное от бэкенда
не зависит: что страница ставит, что читает и когда уничтожается.

## Страница

Страница — это структура, и её поля — собственное состояние страницы: то, что
важно только ей, например шаг, на который она увеличивает счётчик. Сам же
счётчик принадлежит фиче, и страница его только читает.

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

WinUI хранит дерево контролов и перерисовывает только то, что изменилось,
поэтому страница пишется в стиле Elm:

- **`view`** описывает, что нарисовать. `cx.read::<Count>()` возвращает
  состояние и `Dispatch` для отправки действий фиче-владельцу, а заодно
  подписывает страницу: как только `Count` изменится, страница перерисуется.
- **`update`** меняет собственное состояние страницы. `cx.on(|()| Msg::Bigger)`
  превращает событие контрола в сообщение страницы, и оно приходит сюда.
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

egui рисует каждый кадр с нуля, поэтому у страницы нет ни сообщений, ни
`update`. **`render`** получает `&mut self`, и клик обрабатывается в том же
кадре, в котором случился. `cx.read::<Count>()` возвращает состояние и
`Dispatch`; подписка не нужна — следующий кадр всё равно прочитает состояние
заново.
:::
::::

**`install`** вызывается один раз, при монтировании страницы. Всё, что он
ставит, живёт в скоупе страницы и уничтожается вместе с ней (см.
[Скоупы](../scopes/)). Возвращает он начальное состояние страницы.

Получается, что у клика два пути, и выбор зависит от того, чьё это
состояние. Если клик меняет только то, как выглядит сама страница, он меняет
её собственное состояние. Если он что-то просит у домена, действие уходит
фиче через `dispatch.emit`, а результат возвращается новым состоянием,
которое страница прочитает.

`#[page]` дописывает то, что вы опустили: без `install` страница создаётся
через `Default`, а без параметров не нужно объявлять `Params`.

## Лейаут

Лейаут рисует обрамление вокруг страниц и монтирует одну из них. Текущее
место в окне — это маршрут, обычный enum, а `cx.outlet` монтирует страницу,
которая этому маршруту соответствует.

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

`cx.outlet` не рисует страницу сам, а возвращает её отрисовку — так лейаут
сам решает, в какое место своего `ui` её положить.
:::
::::

`cx.navigate::<Place>()` возвращает ближайший сверху навигатор по `Place` —
тот, который создало окно. Когда навигатор переходит на новое место,
монтируется страница этого места, а скоуп предыдущей удаляется. `install` у
лейаута работает так же, как у страницы, и всё, что он ставит, доступно
страницам под ним.

## Приложение

Приложение — это функция, которая ставит то, что нужно всему приложению, и
возвращает окно, которое надо открыть. `run` вызывает её, открывает окно и
работает, пока его не закроют.

::::backends
:::winui
<!-- shown: winui: an application -->
```rust
use guinea::winui::{Window, run};

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}

fn main() -> anyhow::Result<()> {
    run(app)
}
```
<!-- /shown -->
:::
:::egui
<!-- shown: egui: an application -->
```rust
use guinea::eframe::{Window, run};

fn app(_app: &mut FeatureBuilder) -> anyhow::Result<Window> {
    Ok(Window::of::<Shell>().title("Counter").at(Place::Counter))
}

fn main() -> anyhow::Result<()> {
    run(app)
}
```
<!-- /shown -->
:::
::::

`Window::of::<Shell>()` задаёт лейаут, с которого начинается окно, а
`.at(..)` — начальный маршрут навигатора. Всё, что функция ставит через
`app.install`, живёт в скоупе приложения, над всеми окнами.

## Тесты

Страница монтируется без окна, в том же харнессе, что и фичи, а то, что она
нарисовала, можно прочитать как дерево.

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

`#[derive(guinea::Mark)]` и `.mark(..)` дают контролам имена, по которым их
находит тест, — так тест не зависит ни от текста кнопок, ни от их места в
дереве. Если искать по тексту проще, есть `click_text`. `settle` прогоняет
всё, что запустил клик: действие, обработку в акторе, запись в редьюсер и
перерисовку страницы — в порядке, который задаёт сид.

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
guinea = { version = "0.31", features = ["harness"] }
```
:::
:::egui
Харнесса для страниц на egui пока нет. Фичи, на которых стоит страница,
тестируются и без него — как в главе [Скоупы](../scopes/).
:::
::::
