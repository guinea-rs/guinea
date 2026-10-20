---
title: Стримы и источники
sidebar:
  order: 9
---

Задача возвращает один ответ. Стрим выдаёт элементы с течением времени:
записи соединения, изменения файлов или активность процессов.
Метод `cx.task().pour(...)` связывает такой стрим с актором. guinea
владеет работающим источником в скоупе актора, доставляет элементы
в UI-потоке и завершает источник при удалении актора.

В отличие от [фида](../feeds/), этот путь не заменяет один доставленный
элемент более новым снимком. Он подходит, когда потребителю нужны
отдельные записи соединения, а не только последнее состояние.

Импорты для примеров и их тестов:

<!-- shown: streams: imports -->
```rust
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use guinea::actor::Stream;
use guinea::app::Harness;
use guinea::prelude::*;
use tokio::sync::mpsc;
```
<!-- /shown -->

## Адаптер соединения

Вместо сетевого соединения пример использует ограниченный канал Tokio.
Источник отправляет записи или терминальную ошибку; удаление всех
отправителей означает EOF. Небольшая реализация `Stream` переводит
эти условия в сообщения, понятные фиче:

<!-- shown: streams: adapting a connection -->
```rust
#[derive(Clone, Debug)]
pub enum Incoming {
    Opened,
    Record(u32),
    Failed(String),
    Closed,
}

#[derive(Debug)]
pub struct Connection {
    receiver: mpsc::Receiver<Result<u32, String>>,
    ended: bool,
}

impl Stream for Connection {
    type Item = Incoming;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Incoming>> {
        if self.ended {
            return Poll::Ready(None);
        }
        match self.receiver.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(record))) => Poll::Ready(Some(Incoming::Record(record))),
            Poll::Ready(Some(Err(error))) => {
                self.ended = true;
                Poll::Ready(Some(Incoming::Failed(error)))
            }
            Poll::Ready(None) => {
                self.ended = true;
                Poll::Ready(Some(Incoming::Closed))
            }
        }
    }
}

pub fn connection_pair() -> (mpsc::Sender<Result<u32, String>>, Connection) {
    let (sender, receiver) = mpsc::channel(2);
    (
        sender,
        Connection {
            receiver,
            ended: false,
        },
    )
}
```
<!-- /shown -->

Трейт `Stream` доступен через `guinea::actor::Stream`. Готовый стрим
библиотеки можно использовать напрямую, если он удовлетворяет ограничениям.
Собственный адаптер здесь задаёт наш протокол, а не выполняет обязательное
требование guinea. Для канала в примере нужна фича Tokio `sync`.

Ошибка `Failed` завершает соединение по решению этого адаптера.
Сам по себе элемент `Result<T, E>` не останавливает источник при `Err`.
Обычный EOF тоже просто завершает источник, не создавая сообщения актору.
Наш адаптер один раз выдаёт `Closed` перед возвратом `None`, чтобы фича
могла отразить конец соединения. `Opened` отправляет обработчик ниже.

## Получение элементов в акторе

<!-- shown: streams: an actor consumes a stream -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Activity {
    pub records: Vec<u32>,
    pub error: Option<String>,
    pub closed: bool,
}

#[reducer]
fn activity(this: &mut Activity, incoming: Incoming) {
    match incoming {
        Incoming::Opened => {
            this.error = None;
            this.closed = false;
        }
        Incoming::Record(record) => this.records.push(record),
        Incoming::Failed(error) => {
            this.error = Some(error);
            this.closed = true;
        }
        Incoming::Closed => this.closed = true,
    }
}

pub struct Connect(pub Connection);
pub struct Disconnect;

#[derive(Debug)]
struct Collector {
    push: Push<Activity>,
}

actor! {
    Collector {
        handlers {
            Connect => { task Incoming }
            Incoming,
            Disconnect
        }
    }
}

#[handler]
fn connect(In(this, Connect(connection), cx): In<Collector, Connect>) {
    this.push.send(Incoming::Opened);
    cx.task().latest().pour(connection, std::convert::identity);
}

#[handler]
fn incoming(In(this, incoming, _): In<Collector, Incoming>) {
    this.push.send(incoming);
}

#[handler]
fn disconnect(In(this, _, cx): In<Collector, Disconnect>) {
    cx.task().lane::<Incoming>().cancel();
    this.push.send(Incoming::Closed);
}

feature! {
    pub Collecting {
        exports { Activity }
    }
}

#[installs]
fn collecting(cx: &Install) -> anyhow::Result<Collecting> {
    let (activity, _) = cx.state::<Activity>().driven_by(|push| Collector { push });
    Ok(Collecting(activity))
}
```
<!-- /shown -->

Декларация `Connect => { task Incoming }` разрешает обработчику запустить
работу, которая доставляет сообщения `Incoming`. Метод `pour` преобразует
элементы стрима в эти сообщения; здесь достаточно `identity`, поскольку
типы уже совпадают. Стрим должен быть `Send + 'static`; функция
преобразования и получившиеся сообщения — тоже `Send + 'static`.

Политика `latest` относится ко **всему источнику**, а не к каждой записи.
Новый `Connect` заменяет старый источник в линии `Incoming`. При этом
актор обрабатывает каждый принятый элемент выбранного соединения.

Тест подаёт две записи, затем закрывает вход. Ни одна запись не
замещается другой:

<!-- shown: streams: items and eof -->
```rust
#[guinea::test]
fn a_connection_delivers_its_records_and_then_reports_eof(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    h.install::<Collecting>(&()).unwrap();
    h.dispatch::<Activity>().emit(Connect(connection));
    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Ok(2)).unwrap();
    drop(sender);
    h.settled();

    let activity = h.state::<Activity>();
    assert_eq!(activity.records, [1, 2]);
    assert!(activity.closed);
    assert_eq!(activity.error, None);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Терминальная ошибка адаптера тоже доходит до редьюсера и закрывает источник:

<!-- shown: streams: a terminal error -->
```rust
#[guinea::test]
fn a_connection_error_is_a_terminal_message(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    h.install::<Collecting>(&()).unwrap();
    h.dispatch::<Activity>().emit(Connect(connection));
    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Err("connection lost".into())).unwrap();
    h.settled();

    assert_eq!(h.state::<Activity>().records, [1]);
    assert_eq!(
        h.state::<Activity>().error.as_deref(),
        Some("connection lost")
    );
    assert!(h.state::<Activity>().closed);
    assert!(sender.is_closed());
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Если обработчик только пересылает каждый элемент редьюсеру, можно
использовать `cx.task().pushing(push).pour(stream, into)`.
Для источника, которому не нужна политика задач актора, при установке
также доступен `cx.pour(stream, receiver)`.

## Переподключение и поздние элементы

Замена источника отменяет старый и прекращает дальнейшую доставку из него.
Уже принятые в состояние записи сохраняются; ожидающие старые записи
могут не дойти. Автоматической очистки истории при замене нет:

<!-- shown: streams: replacing the connection -->
```rust
#[guinea::test]
fn reconnecting_discards_the_old_connections_pending_items(h: &mut Harness) {
    let (old_sender, old_connection) = connection_pair();
    h.install::<Collecting>(&()).unwrap();
    let dispatch = h.dispatch::<Activity>();
    dispatch.emit(Connect(old_connection));
    old_sender.try_send(Ok(1)).unwrap();
    h.settled();

    old_sender.try_send(Ok(2)).unwrap();
    let (sender, connection) = connection_pair();
    dispatch.emit(Connect(connection));
    sender.try_send(Ok(3)).unwrap();
    h.settled();

    assert_eq!(h.state::<Activity>().records, [1, 3]);
    assert!(old_sender.is_closed());
    dispatch.emit(Disconnect);
    h.settled();
    assert!(sender.is_closed());
    assert!(h.state::<Activity>().closed);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

`Disconnect` отменяет указанную линию. Обработчик явно обновляет статус
соединения: отмена — не естественный EOF, поэтому она не обязана
создавать элемент `Closed` нашего адаптера.

## Сон, буферизация и обратное давление

Спящий скоуп не получает элементы источника. Извлечение приостанавливается,
хотя уже ожидающий элемент запрос может завершиться и удержать его
до пробуждения. Элементы, уже направленные в UI-поток, тоже удерживаются
до пробуждения. В этом примере входной канал сохраняет записи,
пока страница спит:

<!-- shown: streams: sleeping pauses consumption -->
```rust
#[guinea::test]
fn buffered_records_wait_until_the_page_wakes(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    let page = h.child();
    page.install::<Collecting>(&()).unwrap();
    page.dispatch::<Activity>().emit(Connect(connection));
    h.settled();
    page.scope().sleep();

    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Ok(2)).unwrap();
    h.settled();
    assert!(page.state::<Activity>().records.is_empty());

    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Activity>().records, [1, 2]);

    page.leave();
    h.settled();
    assert!(sender.is_closed());
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

После пробуждения приходят обе записи — в отличие от фида, который
доставляет только актуальный снимок. Удаление страницы вместо сна
отменяет источник и удаляет получателя канала. Очередь для следующего
экземпляра страницы при этом не сохраняется.

Ёмкость канала и переполнение — часть контракта источника. Здесь
`try_send` возвращает `Full`, а не молча теряет данные:

<!-- shown: streams: bounded input -->
```rust
#[guinea::test]
fn a_bounded_connection_exposes_backpressure(h: &mut Harness) {
    let (sender, connection) = connection_pair();
    sender.try_send(Ok(1)).unwrap();
    sender.try_send(Ok(2)).unwrap();
    assert!(matches!(
        sender.try_send(Ok(3)),
        Err(mpsc::error::TrySendError::Full(_))
    ));

    h.install::<Collecting>(&()).unwrap();
    h.dispatch::<Activity>().emit(Connect(connection));
    h.settled();
    sender.try_send(Ok(3)).unwrap();
    h.settled();
    assert_eq!(h.state::<Activity>().records, [1, 2, 3]);
}
```
<!-- /shown -->

Асинхронный отправитель может ждать свободного места через
`send(...).await` или выбрать явную политику переполнения. Ёмкость
канала не ограничивает весь конвейер: уже извлечённые элементы
и очередь доставки в UI-поток находятся вне него. guinea не обеспечивает
долговременное хранение или сквозное обратное давление для произвольного
внешнего источника.

Если журнал активности должен переживать разрыв соединения или переход
между страницами, размещай сбор в достаточно долгоживущем скоупе.
Повторное чтение истории и объединение с живыми записями требуют курсора
или порядкового номера из протокола источника. Метод `pour` сам
не добавляет воспроизведение, дедупликацию и доставку ровно один раз.
Фид с объединением обновлений такой протокол не заменяет.

## Таймеры — тоже источники

`repeat` открывает источник тиков, которым владеет скоуп установки:

<!-- shown: streams: a timer is a source -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Pulses(pub u32);

#[reducer]
fn pulses(this: &mut Pulses, _: ()) {
    this.0 += 1;
}

feature! {
    pub Polling {
        exports { Pulses }
    }
}

#[installs]
fn polling(cx: &Install) -> anyhow::Result<Polling> {
    let pulses = cx.state::<Pulses>().plain();
    cx.repeat(Duration::from_secs(1), move || pulses.push(()))
        .named("poll");
    Ok(Polling(pulses))
}
```
<!-- /shown -->

Первый тик ждёт заданный период, а не вызывает обработчик немедленно.
После сна не воспроизводится каждый пропущенный интервал:

<!-- shown: streams: timer sleep and wake -->
```rust
#[guinea::test]
fn a_timer_waits_for_its_first_period_and_does_not_replay_every_missed_tick(h: &mut Harness) {
    let page = h.child();
    page.install::<Polling>(&()).unwrap();
    h.settled();
    assert_eq!(page.state::<Pulses>().0, 0);
    h.advance(Duration::from_secs(1));
    assert_eq!(page.state::<Pulses>().0, 1);

    page.scope().sleep();
    h.advance(Duration::from_secs(5));
    assert_eq!(page.state::<Pulses>().0, 1);
    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Pulses>().0, 2);
    h.advance(Duration::from_secs(1));
    assert_eq!(page.state::<Pulses>().0, 3);

    page.leave();
    h.settled();
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Метод `every(period, message, receiver)` вместо вызова замыкания отправляет
на каждый тик заново созданное сообщение. `Ticks::every(period)`
предоставляет сам стрим тиков для композиции. Фиксированный период
задаётся через `Duration`.

Соответствующие методы возвращают хендл `Source`. Через `.named(...)`
источник можно назвать для инструментов; удаление хендла не останавливает
работу, а его хранение не продлевает жизнь скоупа. Источником владеет
скоуп. Для досрочной остановки источника задачи актора используется
`cancel` его линии, как в обработчике `Disconnect` выше.
