---
title: Фичи и состояние
sidebar:
  order: 3
---

Странице нужен список задач. Она показывает задачи и отправляет команды:
добавить новую или очистить список. Правила обработки этих команд принадлежат
фиче. Её можно использовать на другой странице, с другим GUI-бэкендом или в
тесте без интерфейса.

Соберём `TaskList`: состояние для чтения, действия для страницы и обработчики.
Весь код этой главы общий для всех бэкендов.

## Состояние и обновления

Страница читает `Tasks`. Редьюсер описывает, как каждое обновление меняет это
состояние:

<!-- shown: features: state and updates -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Tasks(pub Vec<String>);

#[derive(Clone)]
pub enum Change {
    Added(String),
    Cleared,
}

#[reducer]
fn tasks(this: &mut Tasks, change: Change) {
    match change {
        Change::Added(title) => this.0.push(title),
        Change::Cleared => this.0.clear(),
    }
}
```
<!-- /shown -->

Атрибут `#[reducer]` реализует для `Tasks` трейт `Reducer` с типом обновлений
`Change`. Начальное состояние берётся из `Default` — здесь это пустой список.
`Added` добавляет название задачи, `Cleared` очищает список.

Обновление — это уже выбранное фичей изменение. Действие, напротив, только
просит фичу что-то сделать.

## Что отправляет страница

У страницы есть два действия:

<!-- shown: features: actions -->
```rust
pub struct AddTask(pub String);
pub struct ClearTasks;
```
<!-- /shown -->

В `AddTask` передаётся введённый пользователем текст. В нём могут быть лишние
пробелы, а названия задачи может не оказаться вовсе. Странице не нужно знать,
что фича делает в этих случаях.

Страница отправляет действия, а не формирует обновления `Change`.
Чтение состояния не даёт ей `Push`: ручку записи фича получает при установке.

## Решение принимает актор

Актор обрабатывает действия и отправляет обновления через `Push<Tasks>`:

<!-- shown: features: an actor decides -->
```rust
#[derive(Debug)]
struct Editing {
    push: Push<Tasks>,
}

actor! {
    Editing {
        handlers { AddTask, ClearTasks }
    }
}

#[handler]
fn add(In(this, AddTask(title), _): In<Editing, AddTask>) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }

    this.push.send(Change::Added(title.to_owned()));
}

#[handler]
fn clear(In(this, _, _): In<Editing, ClearTasks>) {
    this.push.send(Change::Cleared);
}
```
<!-- /shown -->

В `actor!` перечислены сообщения, которые обрабатывает `Editing`.
`#[handler]` реализует обработчик соответствующего сообщения. В `In` лежат
актор, сообщение и контекст вызова. Контекст здесь не нужен, поэтому третье
поле пропущено через `_`.

Перед добавлением актор обрезает пробелы по краям названия. Если ничего не
осталось, обновления не будет. Иначе в редьюсер уйдёт `Change::Added` с готовым
названием. Для очистки предварительной работы нет — достаточно отправить
`Change::Cleared`.

Актор `Editing` приватный. Страница использует тип состояния и действия,
но не обращается к актору по имени или адресу. Если реализация фичи
изменится, код страницы может остаться прежним.

## Установка фичи

Объявление фичи перечисляет её экспорты. Функция установки создаёт состояние
и актор:

<!-- shown: features: installation -->
```rust
feature! {
    pub TaskList {
        exports { Tasks }
    }
}

#[installs]
fn task_list(cx: &Install) -> anyhow::Result<TaskList> {
    let (tasks, _) = cx.state::<Tasks>().driven_by(|push| Editing { push });
    Ok(TaskList(tasks))
}
```
<!-- /shown -->

Вызов `cx.state::<Tasks>()` регистрирует состояние за этой фичей.
`driven_by` создаёт актор, передавая его конструктору `Push<Tasks>`. Метод
возвращает ручку `Bound<Tasks>` и адрес актора. Здесь адрес не используется:
для экспорта состояния достаточно `Bound`.

В этом объявлении `feature!` создаёт кортежную структуру `TaskList` с одним
полем — `Bound<Tasks>`. Функция возвращает эту ручку в `TaskList(tasks)`, а
установка фичи публикует объявленный экспорт. После этого `Tasks` доступен
в скоупе установки и его потомках.

Сам экземпляр `TaskList` не владеет актором и ячейкой состояния: ими владеет
скоуп. Поэтому страница может установить фичу и не хранить возвращённый
экземпляр.

Страница устанавливает фичу через `cx.install::<TaskList>(&())?`, а затем
читает `cx.read::<Tasks>()`. Вместе с состоянием она получает `Dispatch`,
который отправляет действия фиче-владельцу. Через него и уходят `AddTask` и
`ClearTasks`. Как связать чтение и события элементов интерфейса, показано в
главе [«Страницы и лейауты»](../pages/).

## Проверка правил

Харнес устанавливает ту же фичу и отправляет те же действия, что и страница:

<!-- shown: features: actions become state -->
```rust
#[guinea::test]
fn a_task_gets_a_title_before_it_is_added(h: &mut Harness) {
    h.install::<TaskList>(&()).unwrap();
    let dispatch = h.dispatch::<Tasks>();

    dispatch.emit(AddTask("  Read the book  ".into()));
    dispatch.emit(AddTask("   ".into()));
    h.settled();

    assert_eq!(h.state::<Tasks>().0, vec!["Read the book"]);

    dispatch.emit(ClearTasks);
    h.settled();

    assert!(h.state::<Tasks>().0.is_empty());
}
```
<!-- /shown -->

`h.settled()` выполняет работу, поставленную действиями в очередь. После этого
можно проверять результат: пробелы по краям убраны, пустая задача не добавлена,
очистка удалила все задачи.

`Dispatch` адресован той фиче, которой принадлежит прочитанное состояние.
Поэтому два экземпляра в разных скоупах обрабатывают одинаковые действия
независимо:

<!-- shown: features: separate instances -->
```rust
#[guinea::test]
fn two_pages_can_have_separate_task_lists(h: &mut Harness) {
    let work = h.child();
    let home = h.child();
    work.install::<TaskList>(&()).unwrap();
    home.install::<TaskList>(&()).unwrap();

    work.dispatch::<Tasks>()
        .emit(AddTask("Review a change".into()));
    home.dispatch::<Tasks>().emit(AddTask("Buy bread".into()));
    h.settled();

    assert_eq!(work.state::<Tasks>().0, vec!["Review a change"]);
    assert_eq!(home.state::<Tasks>().0, vec!["Buy bread"]);
}
```
<!-- /shown -->

Если страницам нужен общий список, `TaskList` устанавливают в их общем
лейауте. Тогда обе страницы читают один экземпляр и отправляют действия ему.
Связь между местом установки, доступностью и временем жизни разобрана в
главе [«Скоупы»](../scopes/).

## Асинхронные значения: `Load<T>`

Пустой список и список, который ещё не загрузился, — разные состояния.
Если значение приходит асинхронно, используй `Load<T>` из
`guinea::prelude::*`:

| Вариант | Значение |
| --- | --- |
| `NotAsked` | Загрузка ещё не запрашивалась. |
| `Loading` | Ожидание значения. |
| `Ready(T)` | Значение получено, в том числе пустая коллекция. |
| `Failed(Arc<anyhow::Error>)` | Загрузка завершилась ошибкой. |

**`Load::default()` — это `Loading`, а не `NotAsked`.** Тип хранит статус,
но не запускает задачу и не выполняет переходы автоматически. Фича,
которая ждёт явного действия пользователя, может выбрать `NotAsked`
в начальном состоянии:

<!-- shown: loading: state -->
```rust
#[derive(Clone, PartialEq, Debug)]
pub struct Listing {
    pub rows: Load<Vec<String>>,
}

impl Default for Listing {
    fn default() -> Self {
        Self {
            rows: Load::NotAsked,
        }
    }
}

#[reducer]
fn listing(this: &mut Listing, rows: Load<Vec<String>>) {
    this.rows = rows;
}
```
<!-- /shown -->

Редьюсер заменяет состояние загрузки полученным обновлением. Актор
сначала отправляет `Loading`, затем запускает работу и превращает
её результат в `Ready` или `Failed`:

<!-- shown: loading: a loader -->
```rust
pub struct Refresh {
    pub simulate_failure: bool,
}

async fn fetch(simulate_failure: bool) -> anyhow::Result<Vec<String>> {
    tokio::time::sleep(Duration::from_secs(1)).await;
    anyhow::ensure!(!simulate_failure, "service unavailable");
    Ok(vec!["guinea".into(), "shown".into()])
}

#[derive(Debug)]
struct Loader {
    push: Push<Listing>,
}

actor! {
    Loader {
        handlers { Refresh }
    }
}

#[handler]
fn refresh(In(this, Refresh { simulate_failure }, cx): In<Loader, Refresh>) {
    this.push.send(Load::Loading);
    cx.task().latest().pushing(this.push).spawn(async move {
        match fetch(simulate_failure).await {
            Ok(rows) => Load::Ready(rows),
            Err(error) => Load::failed(error),
        }
    });
}

feature! {
    pub Catalogue {
        exports { Listing }
    }
}

#[installs]
fn catalogue(cx: &Install) -> anyhow::Result<Catalogue> {
    let (listing, _) = cx.state::<Listing>().driven_by(|push| Loader { push });
    Ok(Catalogue(listing))
}
```
<!-- /shown -->

Функция `fetch` заменяет вызов сервиса: через секунду она возвращает
два имени или запрошенную имитацию ошибки. Настоящего обращения к серверу
нет. В примере используются `Duration` из `std::time` и фича Tokio `time`,
как в главе о [фоновых задачах](../tasks/).

`Load::failed(error)` помещает ошибку в `Arc`, чтобы состояние с ней
можно было дёшево клонировать. Пересечением запросов управляет `latest`:
сам `Load` не знает, какой запрос новее. Без этой политики или другой
явной проверки старая ошибка могла бы затереть актуальный результат.

### Отображение состояния

Страница может разобрать четыре варианта и выбрать нужные виджеты.
Этот независимый от бэкенда хелпер выдаёт текст, чтобы те же решения
можно было проверить без окна:

<!-- shown: loading: presenting each state -->
```rust
fn status(listing: &Listing) -> String {
    match &listing.rows {
        Load::NotAsked => "Load the catalogue".into(),
        Load::Loading => "Loading…".into(),
        Load::Ready(rows) => format!("{} items", rows.len()),
        Load::Failed(error) => format!("Could not load: {error}"),
    }
}
```
<!-- /shown -->

<!-- shown: loading: successful loading -->
```rust
#[guinea::test]
fn a_request_moves_from_not_asked_through_loading_to_ready(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    assert!(matches!(h.state::<Listing>().rows, Load::NotAsked));
    assert_eq!(status(&h.state::<Listing>()), "Load the catalogue");

    h.dispatch::<Listing>().emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    assert!(h.state::<Listing>().rows.is_loading());
    assert_eq!(status(&h.state::<Listing>()), "Loading…");

    h.advance(Duration::from_secs(1));
    assert_eq!(
        h.state::<Listing>().rows.ready().unwrap(),
        &["guinea", "shown"]
    );
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}
```
<!-- /shown -->

Метод `ready()` возвращает `Some(&T)` только для `Ready`.
Метод `is_loading()` проверяет `Loading`, а `error()` заимствует ошибку
только из `Failed`. Пустой `Ready(Vec::new())` — всё ещё успешный
результат, а не индикатор загрузки.

### Ошибка и повтор

Повторная попытка — ещё один `Refresh`: актор снова отправляет `Loading`
и может заменить ошибку успешным результатом.

<!-- shown: loading: failure and retry -->
```rust
#[guinea::test]
fn a_failed_load_can_be_retried(h: &mut Harness) {
    h.install::<Catalogue>(&()).unwrap();
    let dispatch = h.dispatch::<Listing>();
    dispatch.emit(Refresh {
        simulate_failure: true,
    });
    h.settled();
    h.advance(Duration::from_secs(1));

    let failed = h.state::<Listing>();
    assert!(failed.rows.ready().is_none());
    assert_eq!(
        failed.rows.error().unwrap().to_string(),
        "service unavailable"
    );
    assert_eq!(status(&failed), "Could not load: service unavailable");

    dispatch.emit(Refresh {
        simulate_failure: false,
    });
    h.settled();
    assert!(h.state::<Listing>().rows.is_loading());
    assert!(h.state::<Listing>().rows.error().is_none());
    h.advance(Duration::from_secs(1));
    assert_eq!(status(&h.state::<Listing>()), "2 items");
}
```
<!-- /shown -->

Варианты `Failed` сравниваются по идентичности `Arc` ошибки, а не
по её тексту. Клонирование сохраняет равенство; новая ошибка с тем же
текстом считается другим значением. В `Ready` сравниваются сами данные,
если `T` поддерживает сравнение.

В этом примере обновление готового списка заменяет его на `Loading`,
так что старых строк в состоянии уже нет. Если их нужно показывать
во время обновления, опиши это отдельно — например, последним успешным
значением и статусом запроса. Сам `Load` предыдущее значение не сохраняет.

## Что относить к фиче

Список принятых задач хранится в фиче. Текст, который пользователь ещё
вводит, выбранная строка и локальный фильтр могут оставаться полями страницы.
Если эти данные понадобятся другим экранам или должны будут пережить
страницу, для них можно выбрать фичу с подходящим временем жизни.

Актор полезен, когда обработка действия требует решения, нескольким
обработчикам нужны общие рабочие данные или фоновые задачи возвращают
сообщения. Но состояние можно создать и без актора: цепочка
`cx.state::<Tasks>().plain()` возвращает `Bound<Tasks>`. Её метод `push`
отправляет обновления редьюсеру напрямую. Например, установка может
зарегистрировать подписку, которая преобразует входящие значения и
обновляет состояние через эту ручку.
