---
title: 'Фоновые задачи: состояние и порядок запуска'
sidebar:
  order: 11
---

## Когда снимается входной снимок

`spawn(future)` получает фьючер, созданный в месте вызова. Выражения,
нужные для его создания, выполняются сейчас; тело `async`-блока — только
при опросе фьючера. Занятая линия `conflate` сохраняет только последний
ожидающий запрос.

`spawn_with(factory)` может отложить и создание фьючера. Factory вызывается
в UI-потоке при допуске работы в линию, а не при постановке в ожидание.
Он может захватить `Rc`, если возвращаемый фьючер сам удовлетворяет `Send`.
Этот пробный актор хранит вход в `Rc<Cell<u32>>` и записывает допущенные
к запуску снимки:

<!-- shown: advanced tasks: factory probe -->
```rust
#[derive(Debug)]
struct Probe {
    input: Rc<Cell<u32>>,
    started: Rc<RefCell<Vec<String>>>,
    push: Push<Deltas>,
}

struct Schedule;
struct Produced(u32);

actor! {
    Probe {
        handlers {
            Schedule => { task Produced }
            Produced
        }
    }
}
```
<!-- /shown -->

<!-- shown: advanced tasks: a deferred factory takes its snapshot on admission -->
```rust
#[handler]
fn schedule(In(this, _, cx): In<Probe, Schedule>) {
    let input = this.input.clone();
    let started = this.started.clone();
    cx.task().conflate().spawn_with(move |cancel| {
        let snapshot = input.get();
        started.borrow_mut().push(format!("start {snapshot}"));
        async move {
            let _ = cancel
                .guard(tokio::time::sleep(Duration::from_secs(1)))
                .await;
            Produced(snapshot)
        }
    });
}
```
<!-- /shown -->

`input` — это `Rc<Cell<u32>>`, а `started` записывает вызовы factory
и обработку ответов.
Отложенный factory читает вход при запуске; тот мог измениться после
запроса. Если нужен снимок именно на момент запроса, прочитай его до
`spawn_with` и передай полученное значение во владение замыканию.

<!-- shown: advanced tasks: factory installation -->
```rust
#[handler]
fn produced(In(this, Produced(value), _): In<Probe, Produced>) {
    this.started.borrow_mut().push(format!("reply {value}"));
    this.push.send(value);
}

feature! {
    pub Probing {
        exports { Deltas }
    }
}

#[installs]
fn probing(cx: &Install<(Rc<Cell<u32>>, Rc<RefCell<Vec<String>>>)>) -> anyhow::Result<Probing> {
    let (deltas, _) = cx.state::<Deltas>().driven_by(|push| Probe {
        input: cx.params().0.clone(),
        started: cx.params().1.clone(),
        push,
    });
    deltas.emit(Schedule);
    Ok(Probing(deltas))
}
```
<!-- /shown -->

<!-- shown: advanced tasks: deferred input is not a request time snapshot -->
```rust
#[guinea::test]
fn a_waiting_factory_observes_input_only_when_it_is_started(h: &mut Harness) {
    let input = Rc::new(Cell::new(1));
    let started = Rc::new(RefCell::new(Vec::new()));
    h.install::<Probing>(&(input.clone(), started.clone()))
        .unwrap();
    h.settled();
    input.set(2);
    h.dispatch::<Deltas>().emit(Schedule);
    input.set(9);
    assert_eq!(*started.borrow(), ["start 1"]);

    h.advance(Duration::from_secs(1));
    assert_eq!(*started.borrow(), ["start 1", "start 9", "reply 1"]);
    assert_eq!(h.state::<Deltas>().0, [1]);
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Deltas>().0, [1, 9]);
    assert_eq!(
        *started.borrow(),
        ["start 1", "start 9", "reply 1", "reply 9"]
    );
}
```
<!-- /shown -->

Фича `Probing` регистрирует `Deltas`, создаёт этот актор с двумя
переданными ячейками и запускает первую работу. Обработчик `Produced`
добавляет результат. Ожидающий запрос сделан при значении `2`,
но запускается со значением `9`.
Проверяемый журнал также ставит `start 9` перед `reply 1`: следующий
factory допускается до обработки предыдущего ответа.

Сон не приостанавливает ни эту линию, ни её ответы:

<!-- shown: advanced tasks: conflation while asleep -->
```rust
#[guinea::test]
fn conflate_starts_pending_work_and_handles_answers_while_asleep(h: &mut Harness) {
    let input = Rc::new(Cell::new(1));
    let started = Rc::new(RefCell::new(Vec::new()));
    let page = h.child();
    page.install::<Probing>(&(input.clone(), started.clone()))
        .unwrap();
    h.settled();
    input.set(2);
    page.dispatch::<Deltas>().emit(Schedule);
    page.scope().sleep();
    h.advance(Duration::from_secs(1));
    assert_eq!(*started.borrow(), ["start 1", "start 2", "reply 1"]);
    h.advance(Duration::from_secs(1));
    assert_eq!(page.state::<Deltas>().0, [1, 2]);
}
```
<!-- /shown -->

## Расчётчик должен вернуться до следующего запуска

Для расчёта скоростей обычно нужны предыдущие счётчики и временные метки.
Можно передать это рабочее состояние задаче и вернуть его вместе
с результатом. Из входов, пришедших за это время, достаточно обработать
последний.

Но factory встроенного `conflate` **не** ждёт, пока обработчик предыдущего
ответа вернёт расчётчик в актор. Линия допускает следующую работу до
обработки этого ответа. Рассчитывать в этот момент забрать
`Option<Calculator>` из полей актора нельзя.

Явная машина состояний актора задаёт нужный порядок передачи владения:

<!-- shown: advanced tasks: calculation state travels with the work -->
```rust
#[derive(Default, Debug)]
struct Calculator {
    previous: u32,
}

pub struct Sample(pub u32);
pub struct Calculated {
    calculator: Calculator,
    delta: u32,
}

#[derive(Default, Clone, PartialEq, Debug)]
pub struct Deltas(pub Vec<u32>);

#[reducer]
fn deltas(this: &mut Deltas, delta: u32) {
    this.0.push(delta);
}

#[derive(Debug)]
struct Computing {
    calculator: Option<Calculator>,
    pending: Option<u32>,
    push: Push<Deltas>,
}

actor! {
    Computing {
        handlers {
            Sample => { task Calculated }
            Calculated => { send Sample loop }
        }
    }
}

async fn calculate(mut calculator: Calculator, snapshot: u32) -> Calculated {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let delta = snapshot.saturating_sub(calculator.previous);
    calculator.previous = snapshot;
    Calculated { calculator, delta }
}

#[handler]
fn sample(In(this, Sample(snapshot), cx): In<Computing, Sample>) {
    if let Some(calculator) = this.calculator.take() {
        cx.task().spawn(calculate(calculator, snapshot));
    } else {
        this.pending = Some(snapshot);
    }
}

#[handler]
fn calculated(In(this, result, cx): In<Computing, Calculated>) {
    this.calculator = Some(result.calculator);
    this.push.send(result.delta);
    if let Some(snapshot) = this.pending.take() {
        cx.send(Sample(snapshot));
    }
}

feature! {
    pub Calculating {
        exports { Deltas }
    }
}

#[installs]
fn calculating(cx: &Install) -> anyhow::Result<Calculating> {
    let (deltas, _) = cx.state::<Deltas>().driven_by(|push| Computing {
        calculator: Some(Calculator::default()),
        pending: None,
        push,
    });
    Ok(Calculating(deltas))
}
```
<!-- /shown -->

Здесь семантика объединения реализована без `.conflate()`: одна активная
задача и один заменяемый ожидающий снимок. Обработчик результата сначала
возвращает расчётчик, затем ставит `Sample` в очередь собственного актора.
Пометка `loop` объявляет этот намеренный цикл сообщений; сама по себе
она никакой цикл не запускает. Функция `calculate` владеет всеми входами
и не хранит ссылку на актор.

<!-- shown: advanced tasks: conflating while preserving calculation state -->
```rust
#[guinea::test]
fn only_the_last_pending_sample_runs_with_the_returned_calculator(h: &mut Harness) {
    h.install::<Calculating>(&()).unwrap();
    let dispatch = h.dispatch::<Deltas>();
    dispatch.emit(Sample(10));
    h.settled();
    dispatch.emit(Sample(20));
    dispatch.emit(Sample(30));

    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Deltas>().0, [10]);
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Deltas>().0, [10, 20]);
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Вход `20` вообще не запускается. Вторая разница равна `30 - 10`:
значит, использован расчётчик, вернувшийся из первой задачи.
Ожидающий снимок — последний вход, принятый `Sample`, а не более
позднее чтение общего состояния. Если актор должен восстанавливать
расчётчик после ошибки или отмены без собственного удаления, это
нужно обработать явно: удаление фьючера не возвращает перемещённое
состояние в актор автоматически.

## Хелперы и `Cx<A, M>`

`M` — тип входного сообщения обработчика. Это не необязательная метка:
при объявленном графе он определяет, какие сообщения обработчик вправе
отправить или получить из задачи. Хелпер с `Cx<A>` принимает `Cx<A, ()>`,
а не «контекст любого обработчика A».

Хелпер, который только получает асинхронный контекст, может быть обобщённым:

<!-- shown: advanced tasks: generic context helper -->
```rust
fn capture_context<A: 'static, M>(cx: &Cx<A, M>) -> AsyncContext<A> {
    cx.async_ctx()
}
```
<!-- /shown -->

Для расчётов более простой публичной границей часто будет фьючер
с входами во владении, как у `calculate` выше. Вызов `cx.task().spawn(...)`
или `cx.send(...)` оставь в конкретном обработчике, где известен его
тип сообщения.

У текущего публичного API есть ограничение: необходимый обобщённой
отправке или запуску bound `Allows<M, Out>` не экспортирован через
поддерживаемый публичный API акторов. Он доступен лишь для раскрытия
макросов через `__private`. Книга не предлагает зависеть от этого
скрытого пути. Пока публичного экспорта нет, стабильный полностью
обобщённый хелпер с таким bound описать нельзя.
