---
title: Страницы и лейауты
sidebar:
  order: 2
---

Фича хранит то, что знает приложение; страница это показывает. Страница
рисуется так, как рисует её тулкит, поэтому здесь бэкенды и расходятся - во
вкладках ниже один и тот же счётчик на каждом из них. Всё остальное в guinea
под ними одинаково: что страница устанавливает, что читает и когда уходит.

## Страница

Страница - это структура, и структура и есть собственное состояние страницы:
то, о чём имеет мнение только она, например шаг, на который она прибавляет.
То, чем владеет фича, - счётчик - страница читает.

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

WinUI держит дерево контролов и перерисовывает то, что изменилось, поэтому
страница пишется по-элмовски:

- **`view`** описывает страницу. `cx.read::<Count>()` отдаёт состояние и
  `Dispatch`, чтобы действовать на фичу, которой оно принадлежит, и
  подписывает страницу: когда `Count` меняется, страница рисуется заново.
- **`update`** меняет собственное состояние страницы. `cx.on(|()| Msg::Bigger)`
  превращает событие виджета в сообщение страницы, и сообщение приходит сюда.
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
`update`: **`render`** принимает `&mut self`, и кадр, который увидел клик, на
него и отвечает. `cx.read::<Count>()` отдаёт состояние и `Dispatch`;
подписываться не на что - следующий кадр прочтёт его заново.
:::
::::

**`install`** выполняется один раз, когда страницу монтируют. То, что он
устанавливает, живёт в скоупе страницы и уходит вместе с ней - см.
[Скоупы](../scopes/). Возвращает состояние, с которого страница начинает.

Выходит, у клика два пути, и выбор говорит о том, чьё это состояние. Клик,
который меняет то, как страница показывает себя, меняет её собственное
состояние. Клик, который просит что-то у домена, уходит фиче через
`dispatch.emit` и возвращается новым состоянием, которое страница прочтёт.

`#[page]` дописывает пропущенное: страница без `install` начинает с
`Default`, а страница без параметров обходится без `Params`.

## Лейаут

Лейаут рисует то, что вокруг страниц, и монтирует одну из них. Где находится
окно - это маршрут, enum, и `cx.outlet` монтирует страницу для текущего.

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

`cx.outlet` не рисует страницу, а отдаёт её отрисовку, так что лейаут сам
решает, куда в его `ui` она попадёт.
:::
::::

`cx.navigate::<Place>()` - ближайший сверху навигатор по `Place`, тот, что
открыло окно. Сдвинуть его - значит смонтировать страницу нового места и
удалить скоуп той, что была. `install` у лейаута работает так же, как у
страницы, а то, что он устанавливает, делят страницы под ним.

## Приложение

Приложение - это функция, которая устанавливает то, что нужно приложению, и
возвращает окно, которое надо открыть. `run` вызывает её, открывает окно и
работает, пока оно не закроется.

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

`Window::of::<Shell>()` - лейаут, с которого начинается окно, а `.at(..)` -
откуда стартует его навигатор. То, что функция устанавливает через
`app.install`, живёт в скоупе приложения, над всеми окнами.

## Тесты

Страница монтируется без окна, под тем же харнессом, что и фича, а то, что
она нарисовала, читается обратно деревом.

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

`#[derive(guinea::Mark)]` и `.mark(..)` дают имена контролам, до которых
дотягивается тест, чтобы тест не зависел от их текста и места в дереве;
`click_text` находит контрол по тексту, когда так проще. `settle` выполняет
всё, что запустил клик, - действие, ход актора, запись в редьюсер, повторную
отрисовку страницы - в порядке, который выбирает сид.

Лейаут тестируется так же, только стартует с маршрута:

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

`Mounted` лежит за фичей `harness`:

```toml
[dev-dependencies]
guinea = { version = "0.31", features = ["harness"] }
```
:::
:::egui
Харнесса для страниц на egui пока нет. Фичи за страницей тестируются без
него, как показано в главе [Скоупы](../scopes/).
:::
::::
