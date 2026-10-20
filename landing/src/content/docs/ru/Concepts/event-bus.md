---
title: Шина событий
sidebar:
  order: 7
---

Иногда акторам нужно общаться без передачи адресов друг друга. Например,
контроллер соединения может попросить запустить агент, не зная конкретный
актор, который им управляет. Общий контракт здесь — тип сообщения,
а не `Addr`.

Шина событий связывает эти стороны. Отправитель публикует типизированное
событие, а подписанные на его тип акторы получают его. Ссылки друг на друга
им не нужны. В одном дереве скоупов приложения одна общая шина для всех
окон, лейаутов и страниц. Это не отдельная шина каждого скоупа и не
глобальный синглтон на весь процесс.

## Пример из uniproc

В uniproc `AgentLinkActor` обрабатывает запрос на запуск Windows-агента
внутри процесса приложения. Если необходимые проверки пройдены, он
публикует `RunWindowsAgentInProcess`. Актор Windows-агента подписан
на это сообщение и переключает источник данных на внутрипроцессную
реализацию.

Контроллеру не нужны ни адрес актора агента, ни конкретный тип его бэкенда.
Обе стороны знают общий контракт сообщения. Следующий пример сохраняет
это разделение, но использует текущий API guinea и счётчики вместо
запуска настоящего агента. Это не копия интеграционного кода uniproc.

## Общий контракт

<!-- shown: event bus: a shared contract -->
```rust
#[derive(Clone, Debug, Event)]
pub struct RunInProcess;

pub struct Start;
```
<!-- /shown -->

`Start` — действие для конкретной фичи, отправляемое через её диспетчер.
`RunInProcess` передаётся по шине. Атрибут `#[derive(Event)]` явно
помечает этот общий контракт; события шины должны быть
`Clone + Send + 'static`.

Общие типы сообщений располагаются там, откуда их могут использовать
обе стороны. Отправителю не нужно зависеть от реализации подписчика
и наоборот.

## Публикация из актора

<!-- shown: event bus: publishing from an actor -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Requests(pub u32);

#[reducer]
fn requests(this: &mut Requests, count: u32) {
    this.0 += count;
}

#[derive(Debug)]
struct Control {
    push: Push<Requests>,
}

actor! {
    Control {
        handlers { Start }
        publishes { RunInProcess }
    }
}

#[handler]
fn start(In(this, _, cx): In<Control, Start>) {
    this.push.send(1);
    cx.publish(RunInProcess);
}

feature! {
    pub AgentControl {
        exports { Requests }
    }
}

#[installs]
fn agent_control(cx: &Install) -> anyhow::Result<AgentControl> {
    let (requests, _) = cx.state::<Requests>().driven_by(|push| Control { push });
    Ok(AgentControl(requests))
}
```
<!-- /shown -->

Страница отправляет `Start` через диспетчер, полученный вместе с `Requests`.
Актор `Control` учитывает запрос и публикует `RunInProcess` через свой
контекст. Секция `publishes` объявляет типы событий, которые он вправе
публиковать. Счётчик фиксирует попытку, а не подтверждает запуск агента.

## Подписка актора

<!-- shown: event bus: subscribing an actor -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Runs(pub u32);

#[reducer]
fn runs(this: &mut Runs, count: u32) {
    this.0 += count;
}

#[derive(Debug)]
struct Runtime {
    push: Push<Runs>,
}

actor! {
    Runtime {
        subscribes { RunInProcess }
    }
}

#[handler]
fn run_in_process(In(this, _, _): In<Runtime, RunInProcess>) {
    this.push.send(1);
}

feature! {
    pub AgentRuntime {
        exports { Runs }
    }
}

#[installs]
fn agent_runtime(cx: &Install) -> anyhow::Result<AgentRuntime> {
    let (runs, _) = cx.state::<Runs>().driven_by(|push| Runtime { push });
    Ok(AgentRuntime(runs))
}
```
<!-- /shown -->

Секция `subscribes` подключает актор к шине при его создании.
Входящее событие принимает обычный `#[handler]`. Вызывать подписку
вручную и хранить хендл отписки не нужно.

Акторы могут находиться в соседних скоупах, хотя обычный поиск состояния
между такими скоупами не работает:

<!-- shown: event bus: actors in sibling scopes communicate -->
```rust
#[guinea::test]
fn control_can_reach_a_runtime_without_its_address(h: &mut Harness) {
    let controls = h.child();
    let backend = h.child();
    controls.install::<AgentControl>(&()).unwrap();
    backend.install::<AgentRuntime>(&()).unwrap();

    controls.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(controls.state::<Requests>().0, 1);
    assert_eq!(backend.state::<Runs>().0, 1);
}
```
<!-- /shown -->

Шина не делает их состояние общим. Каждый актор по-прежнему обновляет
редьюсер своей фичи. Через границу проходит только сообщение.

## Рассылка, а не выбор получателя

Публикация адресована всем активным подписчикам этого типа события
в том же дереве. Она не ищет ближайший актор и не выбирает один экземпляр:

<!-- shown: event bus: every subscriber hears it -->
```rust
#[guinea::test]
fn publication_is_not_a_request_to_one_selected_actor(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    let first = h.child();
    let second = h.child();
    first.install::<AgentRuntime>(&()).unwrap();
    second.install::<AgentRuntime>(&()).unwrap();

    h.dispatch::<Requests>().emit(Start);
    h.settled();

    assert_eq!(first.state::<Runs>().0, 1);
    assert_eq!(second.state::<Runs>().0, 1);
}
```
<!-- /shown -->

Здесь оба экземпляра runtime — тестовые счётчики. В приложении две
подписки на команду, которую нужно выполнить один раз, были бы ошибкой
сборки связей: шина не выбирает единственного исполнителя.
Публиковать без подписчиков тоже разрешено; подтверждения выполнения
при этом нет.

Общение без адресов убирает зависимость от конкретного актора,
но не общий контракт сообщений и не необходимость правильно подключить
слушателей. Если нужен результат от одного обработчика, используй
отдельный контракт запросов и ответов шины — `Request`, `RpcRequest`
и `request`, — а не воспринимай `publish` как вызов с возвращаемым
результатом.

## Время жизни и пропущенные события

Подписки актора завершаются при его удалении. Пока его скоуп спит,
события ему не доставляются. Пробуждение не воспроизводит пропущенное:

<!-- shown: event bus: sleeping subscribers miss events -->
```rust
#[guinea::test]
fn waking_does_not_replay_events_published_during_sleep(h: &mut Harness) {
    h.install::<AgentControl>(&()).unwrap();
    let backend = h.child();
    backend.install::<AgentRuntime>(&()).unwrap();

    backend.scope().sleep();
    h.dispatch::<Requests>().emit(Start);
    h.settled();
    assert_eq!(backend.state::<Runs>().0, 0);

    backend.scope().wake();
    h.settled();
    assert_eq!(backend.state::<Runs>().0, 0);

    h.dispatch::<Requests>().emit(Start);
    h.settled();
    assert_eq!(backend.state::<Runs>().0, 1);
}
```
<!-- /shown -->

Шина не хранит текущее значение и очередь для будущих подписчиков.
Потребитель, созданный после публикации, тоже не получит это событие.
Если новой или проснувшейся странице нужен последний снимок данных,
используй [фид](../feeds/).

По той же причине шина — не надёжная очередь фоновых заданий.
Если работа должна выполняться и после ухода со страницы, размещай
потребителя в скоупе с нужным временем жизни. При необходимости
подтверждение и повторные попытки должны быть частью явного протокола.

## Обработчик без актора

Для небольшой реакции фича может зарегистрировать обычное замыкание:

<!-- shown: event bus: a scoped callback -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Notices(pub u32);

#[reducer]
fn notices(this: &mut Notices, count: u32) {
    this.0 += count;
}

feature! {
    pub Observing {
        exports { Notices }
    }
}

#[installs]
fn observing(cx: &Install) -> anyhow::Result<Observing> {
    let notices = cx.state::<Notices>().plain();
    cx.subscribe(move |_: RunInProcess| notices.push(1));
    Ok(Observing(notices))
}
```
<!-- /shown -->

`cx.subscribe` связывает подписку со скоупом фичи. Замыкание тоже
ничего не получает во время сна скоупа и удаляется вместе с ним.
Такой вариант подходит для небольшой реакции; актор нужен, когда
несколько обработчиков используют общее рабочее состояние или запускают
фоновые задачи.

## Доставка и фоновая работа

Акторы обрабатывают события в UI-потоке. Эти обработчики должны быть
короткими, как и обработчики действий страницы; медленную работу
запускай через [`cx.task()`](../tasks/). Публикация сама по себе
не запускает каждого подписчика отдельной фоновой задачей.

Метод `cx.async_ctx()` даёт контекст, через который можно публиковать
из фоновой работы. Его `publish` ставит доставку в UI-поток
и по-прежнему проверяет объявленный актором контракт `publishes`.
Это не добавляет шине хранение текущего состояния.
