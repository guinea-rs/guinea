---
title: Запросы и ответы
sidebar:
  order: 8
---

[Событие](../event-bus/) сообщает что-то подписчикам без возвращаемого
результата. Если нужен ответ от одного сервиса, но адрес его актора
передавать не хочется, используй типизированный запрос. Этот RPC работает
между частями одного приложения через его шину, а не является сетевым
транспортом.

В примерах используются следующие импорты, включая хёрнесс для тестов:

<!-- shown: rpc: imports -->
```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use guinea::actor::event_bus::{RpcRequest, Why};
use guinea::app::Harness;
use guinea::prelude::*;
```
<!-- /shown -->

## Контракт

Запрос объявляет тип своего ответа:

<!-- shown: rpc: request and response -->
```rust
#[derive(Clone, Debug, guinea::Request)]
#[request(reply = Outcome)]
pub struct Terminate(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Done,
    Denied,
}
```
<!-- /shown -->

`Outcome::Denied` — обычный ответ сервиса. Он отличается от ситуации,
когда ответ вообще не удалось получить. Типы запроса и ответа должны
быть `Clone + Send + 'static`.

Пример имитирует завершение процесса, но не обращается к API ОС:
для PID `4` возвращается отказ, для остальных — `Done`.

## Отвечает один сервис

<!-- shown: rpc: an answerer -->
```rust
#[derive(Clone, Debug, Default)]
pub struct ServerParams {
    pub delay: Duration,
    pub completed: Arc<AtomicUsize>,
}

#[derive(Debug)]
struct Processes(ServerParams);

actor! {
    Processes {
        subscribes { RpcRequest<Terminate> }
    }
}

#[handler]
fn terminate(In(this, Terminate(pid), _): In<Processes, Terminate>) -> Reply<Outcome> {
    let params = this.0.clone();
    Reply::later(async move {
        tokio::time::sleep(params.delay).await;
        params.completed.fetch_add(1, Ordering::SeqCst);
        match pid {
            4 => Outcome::Denied,
            _ => Outcome::Done,
        }
    })
}

feature! {
    pub ProcessService {}
}

#[installs]
fn process_service(cx: &Install<ServerParams>) -> anyhow::Result<ProcessService> {
    cx.spawn(Processes(cx.params().clone()));
    Ok(ProcessService)
}
```
<!-- /shown -->

Актор подписан на `RpcRequest<Terminate>`, но типизированный обработчик
принимает `In<Processes, Terminate>`. Возвращаемый `Reply<Outcome>` делает
его отвечающей стороной; сгенерированная реализация доставляет ответ
вызывающему коду.

`Reply::later` выполняет фьючер в фоне. Здесь заданная задержка имитирует
операцию, а атомарный счётчик позволяет тестам заметить её завершение.
Изменяемое заимствование актора заканчивается при возврате обработчика:
скопируй нужные фьючеру входные данные, а не ссылку на актор.

Для немедленного ответа можно вернуть из обработчика сам `Outcome`
или использовать `Reply::now(outcome)`. Медленную работу не нужно ждать
в UI-потоке.

На один тип запроса в шине приходится только один отвечающий обработчик.
Регистрация второго приводит к панике, даже если он в другом скоупе
или первый спит. Другие подписчики могут наблюдать запрос без ответа:
обычный обработчик, который ничего не возвращает, — слушатель, а не
вторая отвечающая сторона.

## Клиент ждёт вне UI-потока

<!-- shown: rpc: a client -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct LastReply(pub Option<Result<Outcome, Why>>);

#[derive(Clone)]
pub struct Answered(pub Result<Outcome, Why>);

#[reducer]
fn last_reply(this: &mut LastReply, answer: Answered) {
    this.0 = Some(answer.0);
}

pub struct Ask(pub u32);
pub struct CancelRequest;

#[derive(Debug)]
struct Client {
    push: Push<LastReply>,
}

actor! {
    Client {
        handlers { Ask, CancelRequest }
    }
}

#[handler]
fn ask(In(this, Ask(pid), cx): In<Client, Ask>) {
    let remote = cx.async_ctx();
    cx.task().latest().pushing(this.push).spawn(async move {
        let answer = remote.request(Terminate(pid), Duration::from_secs(2)).await;
        Answered(answer.map_err(|error| error.why))
    });
}

#[handler]
fn cancel_request(In(_, _, cx): In<Client, CancelRequest>) {
    cx.task().lane::<Answered>().cancel();
}

feature! {
    pub Requesting {
        exports { LastReply }
    }
}

#[installs]
fn requesting(cx: &Install) -> anyhow::Result<Requesting> {
    let (reply, _) = cx.state::<LastReply>().driven_by(|push| Client { push });
    Ok(Requesting(reply))
}
```
<!-- /shown -->

Страница отправляет `Ask` через диспетчер своей фичи. Обработчик получает
`AsyncContext`, запускает задачу и уже в ней ждёт `request`.
Результат — либо `Outcome`, либо ошибка `Unanswered`. В примере
в состояние попадает её поле `why`; полная ошибка также содержит
имя типа запроса.

`pushing` применяет результат в UI-потоке. Политика `latest` не позволяет
старой клиентской задаче затереть результат более новой. Линия по умолчанию
определяется типом результата задачи `Answered`, а не типом запроса
`Terminate`.

Сервис и клиент можно установить в соседних скоупах:

<!-- shown: rpc: an answer without an address -->
```rust
#[guinea::test]
fn a_client_receives_the_typed_answer(h: &mut Harness) {
    let server = h.child();
    let client = h.child();
    server
        .install::<ProcessService>(&ServerParams::default())
        .unwrap();
    client.install::<Requesting>(&()).unwrap();

    client.dispatch::<LastReply>().emit(Ask(42));
    h.settled();
    assert_eq!(client.state::<LastReply>().0, Some(Ok(Outcome::Done)));

    client.dispatch::<LastReply>().emit(Ask(4));
    h.settled();
    assert_eq!(client.state::<LastReply>().0, Some(Ok(Outcome::Denied)));
}
```
<!-- /shown -->

## Подмена сервиса в тесте

`answer_fn` задаёт синхронный ответ без создания актора:

<!-- shown: rpc: a fake answerer -->
```rust
#[guinea::test]
fn a_test_can_replace_the_service_with_a_callback(h: &mut Harness) {
    let _answerer = h.bus().answer_fn(|_: Terminate| Outcome::Denied);
    h.install::<Requesting>(&()).unwrap();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();

    assert_eq!(h.state::<LastReply>().0, Some(Ok(Outcome::Denied)));
}
```
<!-- /shown -->

Возвращённый guard подписки нужно хранить до конца теста: его удаление
отключает обработчик. Подмена занимает то же единственное место
отвечающего, поэтому настоящий сервис рядом устанавливать нельзя.
В отличие от `cx.subscribe`, у этого низкоуровневого замыкания нет
потребительского скоупа, который автоматически отправлял бы его спать.

## Когда ответа нет

`request` возвращает `Unanswered` с одной из причин:

| `Why` | Значение |
| --- | --- |
| `Nobody` | Никто не зарегистрирован отвечающей стороной. |
| `Asleep { answerer }` | Скоуп зарегистрированного обработчика спит. |
| `Dropped` | Возможность ответить удалена без отправки ответа. |
| `TimedOut` | Истёк срок ожидания клиента. |
| `TreeGone` | При попытке доставки дерево приложения уже было удалено. |

`Nobody` и `Asleep` возвращаются без ожидания тайм-аута. Спящий сервис —
не очередь: пробуждение не воспроизводит неудавшийся запрос. Клиент сам
решает, нужно ли повторять попытку и когда.

## Тайм-аут не отменяет операцию

Клиент выше ждёт две секунды. Пусть операция сервиса занимает пять —
посмотрим на обе стороны:

<!-- shown: rpc: timeout does not cancel the operation -->
```rust
#[guinea::test]
fn a_timeout_ends_waiting_but_the_service_can_finish_later(h: &mut Harness) {
    let params = ServerParams {
        delay: Duration::from_secs(5),
        ..Default::default()
    };
    h.install::<ProcessService>(&params).unwrap();
    h.install::<Requesting>(&()).unwrap();
    h.dispatch::<LastReply>().emit(Ask(42));
    h.settled();

    h.advance(Duration::from_secs(2));
    assert_eq!(h.state::<LastReply>().0, Some(Err(Why::TimedOut)));
    assert_eq!(params.completed.load(Ordering::SeqCst), 0);

    h.advance(Duration::from_secs(3));
    assert_eq!(params.completed.load(Ordering::SeqCst), 1);
    assert_eq!(h.state::<LastReply>().0, Some(Err(Why::TimedOut)));
}
```
<!-- /shown -->

Клиент перестал ждать, но сервис всё равно заканчивает работу. Поздний
ответ не заменяет `TimedOut` в клиентском состоянии. В реальной операции
побочный эффект уже мог произойти, хотя клиент не получил ответ.
Для автоматического повтора поэтому нужны правила идемпотентности
на уровне приложения.

## Отмена клиента

Отмена задачи, ожидающей `request`, тоже не отзывает запрос, который
уже доставлен сервису:

<!-- shown: rpc: cancelling the caller is not remote cancellation -->
```rust
#[guinea::test]
fn cancelling_the_client_does_not_cancel_an_accepted_request(h: &mut Harness) {
    let params = ServerParams {
        delay: Duration::from_secs(5),
        ..Default::default()
    };
    h.install::<ProcessService>(&params).unwrap();
    h.install::<Requesting>(&()).unwrap();
    let dispatch = h.dispatch::<LastReply>();
    dispatch.emit(Ask(42));
    h.settled();

    dispatch.emit(CancelRequest);
    h.advance(Duration::from_secs(5));

    assert_eq!(params.completed.load(Ordering::SeqCst), 1);
    assert_eq!(h.state::<LastReply>().0, None);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Счётчик завершений сервиса увеличился, но редьюсер отменённого клиента
не получил результат. Замена клиентской задачи через `latest` или
удаление её страницы имеет то же ограничение: останавливается работа
этого клиента, а не принятая операция другого актора.

Уже запущенный фьючер `Reply::later` также не удаляется автоматически
при уходе актора, который отвечает. Если операция должна поддерживать
отмену, задай явный протокол сервиса: ID операции и запрос отмены
или токен, который сервис действительно проверяет. Отсутствие
ожидающего клиента само по себе не означает отмену операции.
