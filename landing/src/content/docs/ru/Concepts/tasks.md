---
title: Фоновые задачи
sidebar:
  order: 5
---

Обработчик выполняется в UI-потоке. В нём можно принять решение и обновить
состояние, но ожидание ответа сервера или тяжёлая работа заблокируют
интерфейс. Запускай асинхронную работу через `cx.task()`. Результат можно
отправить прямо редьюсеру или получить сообщением в тот же актор.

Здесь два отдельных вопроса: что делать с результатом и как поступить,
если новый запрос пришёл до завершения предыдущего? Соберём поисковую фичу,
которая показывает ответ только на последний запрос. Код общий для всех
GUI-бэкендов.

## Запрос и ответ

Страница отправляет `Query`, фоновая задача возвращает `Found`. Редьюсер
сохраняет результат поиска:

<!-- shown: tasks: state and messages -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Results {
    pub query: String,
    pub names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub query: String,
    pub names: Vec<String>,
}

#[reducer]
fn results(this: &mut Results, found: Found) {
    this.query = found.query;
    this.names = found.names;
}

pub struct Query(pub String);
pub struct CancelSearch;
```
<!-- /shown -->

Чтобы пример был самодостаточным, `search` ждёт секунду и фильтрует
фиксированный список. Сеть здесь не используется. В реальном приложении
на этом месте может быть асинхронный вызов клиентской библиотеки.

<!-- shown: tasks: simulated search -->
```rust
async fn search(query: String) -> Found {
    tokio::time::sleep(Duration::from_secs(1)).await;
    let names = ["guinea", "guinea-app", "shown"]
        .into_iter()
        .filter(|name| name.contains(query.as_str()))
        .map(str::to_owned)
        .collect();
    Found { query, names }
}
```
<!-- /shown -->

Эта функция использует Tokio с фичей `time`. С включённой по умолчанию
фичей guinea `own-runtime` бэкенд при необходимости запускает асинхронный
рантайм. Если её отключить, нужно войти в собственный рантайм до запуска
приложения.

## Запуск из обработчика

<!-- shown: tasks: an actor starts work -->
```rust
#[derive(Debug)]
struct Searching {
    push: Push<Results>,
}

actor! {
    Searching {
        handlers { Query, CancelSearch }
    }
}

#[handler]
fn query(In(this, Query(text), cx): In<Searching, Query>) {
    cx.task().latest().pushing(this.push).spawn(search(text));
}
```
<!-- /shown -->

Метод `pushing(this.push)` отправляет готовый результат прямо редьюсеру
`Results`. Функция `search` возвращает `Found`, который уже является типом
его обновления, поэтому отдельный обработчик `Found` и декларация
`task Found` не нужны.

Билдер, возвращённый `cx.task()`, ничего не запускает до вызова `spawn`.
Фьючер и его результат должны быть `Send + 'static`: передавай во фьючер
данные запроса во владение, а не заимствование актора. Сам актор и его
обработчики остаются в UI-потоке. Там же guinea применяет готовое обновление.

Актор и его состояние устанавливаются обычной фичей:

<!-- shown: tasks: installation -->
```rust
feature! {
    pub Search {
        exports { Results }
    }
}

#[installs]
fn searching(cx: &Install) -> anyhow::Result<Search> {
    let (results, _) = cx.state::<Results>().driven_by(|push| Searching { push });
    Ok(Search(results))
}
```
<!-- /shown -->

Страница читает `Results` и отправляет `Query` через диспетчер. Ей не нужно
хранить хендл задачи или самостоятельно пересылать готовые результаты.

## Если запросы пересекаются

Без политики все задачи выполняются независимо. Каждый готовый ответ
доставляется в порядке завершения работы. Для независимых операций это
подходит, для поисковой строки — нет: медленный ответ на старый запрос
может затереть актуальные результаты.

`latest()` заменяет работу в той же **линии задач** — lane. Старый фьючер
отменяется. Если его ответ уже отправлен в UI-поток, но до доставки в линии
появилась новая работа, старый ответ тоже отбрасывается.

Доступны следующие политики:

| Политика | Новый запрос приходит, пока линия занята |
| --- | --- |
| Без политики | Выполнять независимо и доставлять каждый ответ. |
| `latest()` | Отменить старую работу и начать новую. |
| `conflate()` | Завершить текущую работу, затем выполнить только последний из ожидающих запросов. |
| `exhaust()` | Игнорировать новый запрос, пока текущая работа не закончится. |

Например, `conflate` позволяет объединять повторные запросы на обновление
снимка, а `exhaust` — игнорировать повторные нажатия, пока операция ещё
выполняется. Ни одна из этих политик не сохраняет все запросы в очереди.

Линия принадлежит одному экземпляру актора. По умолчанию она определяется
типом результата фьючера: здесь это `Found`. Явное имя не требуется.
Для независимых операций с одинаковым типом результата можно разделить
линии через `lane::<K>()`.
Метод `keyed(value)` дополнительно разделяет работу по ключу, сравниваемому
на равенство, например по ID документа. Первая выбранная политика
закрепляется за линией до конца жизни актора. Попытка запустить в ней
работу с другой политикой приводит к панике.

## Проверка замены без настоящего ожидания

<!-- shown: tasks: only the newest query answers -->
```rust
#[guinea::test]
fn a_new_query_replaces_the_search_in_flight(h: &mut Harness) {
    h.install::<Search>(&()).unwrap();
    let dispatch = h.dispatch::<Results>();

    dispatch.emit(Query("shown".into()));
    h.settled();
    h.advance(Duration::from_millis(500));

    dispatch.emit(Query("guinea".into()));
    h.settled();
    h.advance(Duration::from_millis(500));
    assert_eq!(*h.state::<Results>(), Results::default());

    h.advance(Duration::from_millis(500));
    let results = h.state::<Results>();
    assert_eq!(results.query, "guinea");
    assert_eq!(results.names, ["guinea", "guinea-app"]);
}
```
<!-- /shown -->

`h.settled()` выполняет готовую работу, не двигая время. `h.advance()`
переводит приостановленные часы Tokio и выполняет то, что стало готово.
Тест запускает второй запрос посередине первого. К моменту завершения
первого запроса результата ещё нет: ответить разрешено только его замене.

## Явная отмена

Кнопка отмены может отправлять `CancelSearch`. Обработчик указывает,
какую линию остановить:

<!-- shown: tasks: cancelling a lane -->
```rust
#[handler]
fn cancel_search(In(_, _, cx): In<Searching, CancelSearch>) {
    cx.task().lane::<Found>().cancel();
}
```
<!-- /shown -->

`cancel()` отменяет текущую работу и удаляет ожидающую работу этой линии.
С `keyed(value)` отменяется только указанный ключ, без него — все ключи
линии. При отмене линию нужно указать явно, даже если она определяется
типом результата фьючера. Здесь `lane::<Found>()` указывает ту же линию,
которую `spawn(search(text))` выбирает автоматически.

Отмена не удаляет актор и не закрывает линию навсегда:

<!-- shown: tasks: an explicit cancellation -->
```rust
#[guinea::test]
fn a_cancelled_search_does_not_change_the_results(h: &mut Harness) {
    h.install::<Search>(&()).unwrap();
    let dispatch = h.dispatch::<Results>();
    dispatch.emit(Query("guinea".into()));
    h.settled();

    dispatch.emit(CancelSearch);
    h.advance(Duration::from_secs(2));

    assert_eq!(*h.state::<Results>(), Results::default());
    assert_eq!(h.stuck(), 0);

    dispatch.emit(Query("shown".into()));
    h.settled();
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Results>().query, "shown");
}
```
<!-- /shown -->

## Уход со страницы

Время жизни актора ограничивает и его задачи. Удаление скоупа страницы
удаляет актор, отменяет его работу и прекращает доставку ответов:

<!-- shown: tasks: leaving cancels the work -->
```rust
#[guinea::test]
fn leaving_the_page_cancels_its_search(h: &mut Harness) {
    let page = h.child();
    page.install::<Search>(&()).unwrap();
    page.dispatch::<Results>().emit(Query("guinea".into()));
    h.settled();
    assert_eq!(h.stuck(), 1);

    page.leave();
    h.settled();
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

`h.stuck()` считает задачи, которые ещё чего-то ждут. Здесь проверяется,
что незавершённый поиск не переживает страницу. Если поиск должен
продолжаться после перехода, установи его фичу в общем лейауте или
приложении. Сохранить страницу спящей — не то же самое, что удалить её
скоуп.

## Эквивалент: ответ через актор

Без `pushing` задача возвращает результат актору сообщением. В этом
примере пересылка такого сообщения редьюсеру даёт тот же результат:

<!-- shown: tasks: replying through the actor -->
```rust
#[derive(Debug)]
struct Searching {
    push: Push<Results>,
}

actor! {
    Searching {
        handlers {
            Query => { task Found }
            Found,
            CancelSearch
        }
    }
}

#[handler]
fn query(In(_, Query(text), cx): In<Searching, Query>) {
    cx.task().latest().spawn(search(text));
}

#[handler]
fn found(In(this, answer, _): In<Searching, Found>) {
    this.push.send(answer);
}
```
<!-- /shown -->

Этот вариант заменяет объявление актора и обработчики выше, а не добавляет
второй обработчик `Query` тому же актору. Запись `task Found` объявляет
ответ, а обработчик `Found` передаёт его в состояние. Установка и отмена
остаются прежними; в обоих вариантах линия по умолчанию определяется
типом `Found`.

Такой вариант нужен, когда актор должен проверить результат, изменить
собственные рабочие поля или выбрать следующее действие до обновления
состояния. Если обработчик только пересылает обновление, `pushing`
убирает этот лишний шаг. Если достаточно преобразовать результат
в обновление в UI-потоке, используй `pushing_with`; само преобразование
должно быть дешёвым.

## Чего отмена не делает

При отмене обычного `spawn` фьючер удаляется на границе ожидания. Это
не отменяет уже принятый сервером запрос, уже записанный файл или
синхронный код, который сейчас блокирует рабочий поток. Не выполняй
блокирующие операции в UI-обработчиках; при необходимости используй
предназначенный для них API асинхронного рантайма.

## Асинхронная очистка после отмены

Допустим, задача использует сеанс, который нужно закрыть запросом
и подтверждением другой стороны. Одного удаления фьючера для этого
протокола недостаточно: после остановки основной работы ещё нужно
дождаться подтверждения.

Вот небольшая модель сеанса на двух одноразовых каналах, без сети.
Метод `close` отправляет запрос и ждёт подтверждения. Функция
`session_pair` возвращает тесту противоположные концы каналов,
чтобы он мог выступить другой стороной:

<!-- shown: tasks: a session closes asynchronously -->
```rust
use tokio::sync::oneshot;

#[derive(Debug)]
pub struct Session {
    close_requested: oneshot::Sender<()>,
    closed: oneshot::Receiver<()>,
}

impl Session {
    async fn close(self) -> bool {
        if self.close_requested.send(()).is_err() {
            return false;
        }
        self.closed.await.is_ok()
    }
}

pub fn session_pair() -> (Session, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (close_requested, closing) = oneshot::channel();
    let (confirm_closed, closed) = oneshot::channel();
    (
        Session {
            close_requested,
            closed,
        },
        closing,
        confirm_closed,
    )
}
```
<!-- /shown -->

`spawn_with` передаёт токен отмены в функцию, создающую фьючер.
В отличие от `spawn`, при отмене этот фьючер не удаляется: он должен
сам заметить токен и завершиться:

<!-- shown: tasks: cooperative work -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Completed(pub u32);

#[derive(Clone)]
pub struct SessionFinished(pub bool);

#[reducer]
fn completed(this: &mut Completed, finished: SessionFinished) {
    if finished.0 {
        this.0 += 1;
    }
}

pub struct StartSession(pub Session);
pub struct StopSession;

#[derive(Debug)]
struct Working {
    push: Push<Completed>,
}

actor! {
    Working {
        handlers { StartSession, StopSession }
    }
}

#[handler]
fn start_session(In(this, StartSession(session), cx): In<Working, StartSession>) {
    cx.task()
        .latest()
        .pushing(this.push)
        .spawn_with(|cancel| async move {
            let _ = cancel
                .guard(tokio::time::sleep(Duration::from_secs(1)))
                .await;
            SessionFinished(session.close().await)
        });
}

#[handler]
fn stop_session(In(_, _, cx): In<Working, StopSession>) {
    cx.task().lane::<SessionFinished>().cancel();
}

feature! {
    pub SessionWork {
        exports { Completed }
    }
}

#[installs]
fn session_work(cx: &Install) -> anyhow::Result<SessionWork> {
    let (completed, _) = cx.state::<Completed>().driven_by(|push| Working { push });
    Ok(SessionWork(completed))
}
```
<!-- /shown -->

Ожидание секунды изображает основную операцию сеанса.
Вызов `cancel.guard(...)` прекращает её при отмене и возвращает `None`;
при обычном завершении возвращает `Some(())`. В обоих случаях фьючер
затем ждёт `session.close()`. Очистка намеренно находится **вне**
защищённой токеном операции, иначе та же отмена могла бы прервать
и её.

`Completed` считает успешные результаты, принятые редьюсером,
а не закрытые сеансы. После отмены фьючер всё равно закрывает сеанс
и возвращает `SessionFinished`, но guinea отбрасывает этот ответ:

<!-- shown: tasks: cancellation lets cleanup finish but drops the answer -->
```rust
#[guinea::test]
fn cancellation_waits_for_session_cleanup_without_applying_its_answer(h: &mut Harness) {
    let (session, mut closing, confirm_closed) = session_pair();
    h.install::<SessionWork>(&()).unwrap();
    let dispatch = h.dispatch::<Completed>();
    dispatch.emit(StartSession(session));
    h.settled();
    assert_eq!(
        closing.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    );

    dispatch.emit(StopSession);
    h.settled();
    assert_eq!(closing.try_recv(), Ok(()));
    assert_eq!(h.stuck(), 1);
    assert_eq!(h.state::<Completed>().0, 0);

    confirm_closed.send(()).unwrap();
    h.settled();
    assert_eq!(h.stuck(), 0);
    assert_eq!(h.state::<Completed>().0, 0);
}
```
<!-- /shown -->

Первый `settled` проверяет, что работающий сеанс ещё не запросил
закрытие. После `StopSession` другая сторона получает запрос, но задача
по-прежнему ждёт подтверждения: `h.stuck()` равен `1`. Отправка
подтверждения позволяет закончить очистку. Задачи больше нет,
а `Completed` остаётся равным `0`. Без отмены та же очистка завершается
до доставки результата, после чего счётчик увеличивается.

Удаление страницы тоже сигнализирует отмену, а не удаляет такой
фьючер. Он может закончить закрытие сеанса уже после ухода страницы:

<!-- shown: tasks: leaving also allows cooperative cleanup -->
```rust
#[guinea::test]
fn session_cleanup_can_finish_after_its_page_leaves(h: &mut Harness) {
    let (session, mut closing, confirm_closed) = session_pair();
    let page = h.child();
    page.install::<SessionWork>(&()).unwrap();
    page.dispatch::<Completed>().emit(StartSession(session));
    h.settled();

    page.leave();
    h.settled();
    assert_eq!(closing.try_recv(), Ok(()));
    assert_eq!(h.stuck(), 1);

    confirm_closed.send(()).unwrap();
    h.settled();
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

В этом тесте хёрнесс продолжает работать. Это не гарантия, что выход
из приложения дождётся очистки: асинхронный рантайм должен оставаться
живым, чтобы её выполнить. Если другая сторона не подтвердит закрытие,
пример продолжит ждать; в настоящем протоколе нужно выбрать тайм-аут
и обработку ошибки. Аналогично, фьючер `spawn_with`, который игнорирует
токен, не остановится только потому, что ему запросили отмену.

## Работа без ответа

Допустим, страница отправляет диагностический отчёт. Показывать результат
отправки не нужно: задача должна выполнить действие и закончиться,
не посылая актору новое сообщение. Для этого есть `detached()`.
Его фьючер возвращает `()`, отдельный обработчик ответа не нужен.

В примере канал изображает принимающий отчёты сервис, а секундное ожидание —
задержку отправки. `Reports` считает запросы на отправку, не ответы сервиса:

<!-- shown: tasks: reporting without an actor reply -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Reports(pub u32);

#[reducer]
fn reports(this: &mut Reports, _: ()) {
    this.0 += 1;
}

pub struct SendReport(pub String);
pub struct ReportUpload;

struct Reporting {
    push: Push<Reports>,
    outgoing: tokio::sync::mpsc::UnboundedSender<String>,
}

actor! {
    Reporting {
        handlers { SendReport }
    }
}

#[handler]
fn send_report(In(this, SendReport(text), cx): In<Reporting, SendReport>) {
    this.push.send(());
    let outgoing = this.outgoing.clone();
    cx.task()
        .lane::<ReportUpload>()
        .latest()
        .detached()
        .spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let _ = outgoing.send(text);
        });
}

feature! {
    pub ReportSending {
        exports { Reports }
    }
}

#[installs]
fn report_sending(
    cx: &Install<tokio::sync::mpsc::UnboundedSender<String>>,
) -> anyhow::Result<ReportSending> {
    let (reports, _) = cx.state::<Reports>().driven_by(|push| Reporting {
        push,
        outgoing: cx.params().clone(),
    });
    Ok(ReportSending(reports))
}
```
<!-- /shown -->

Если пользователь запросил новый отчёт до отправки предыдущего, `latest()`
отменяет старую отправку. `ReportUpload` — пустой тип, которым мы назвали
эту группу задач. Здесь `.lane::<ReportUpload>()` обязателен: без ответа
guinea не может взять его тип за имя линии. Для другой группы задач выбери
другой тип, чтобы они не отменяли друг друга.

<!-- shown: tasks: only the newest detached report is sent -->
```rust
#[guinea::test]
fn a_new_detached_report_replaces_the_old_one_without_an_actor_reply(h: &mut Harness) {
    let (outgoing, mut received) = tokio::sync::mpsc::unbounded_channel();
    h.install::<ReportSending>(&outgoing).unwrap();
    let dispatch = h.dispatch::<Reports>();
    dispatch.emit(SendReport("old".into()));
    h.settled();
    h.advance(Duration::from_millis(500));
    dispatch.emit(SendReport("new".into()));
    h.settled();
    h.advance(Duration::from_millis(500));
    assert!(received.try_recv().is_err());
    h.advance(Duration::from_millis(500));
    assert_eq!(received.try_recv(), Ok("new".into()));
    assert!(received.try_recv().is_err());
    assert_eq!(h.state::<Reports>().0, 2);
}
```
<!-- /shown -->

«Без ответа» не означает «продолжать после закрытия страницы». Задачей
по-прежнему владеет актор: удаление его скоупа отменяет ещё не законченную
отправку, что проверяется отдельно:

<!-- shown: tasks: detached work is cancelled when its page leaves -->
```rust
#[guinea::test]
fn leaving_cancels_a_detached_report_before_it_is_sent(h: &mut Harness) {
    let (outgoing, mut received) = tokio::sync::mpsc::unbounded_channel();
    let page = h.child();
    page.install::<ReportSending>(&outgoing).unwrap();
    page.dispatch::<Reports>().emit(SendReport("report".into()));
    h.settled();
    page.leave();
    h.advance(Duration::from_secs(1));
    assert!(received.try_recv().is_err());
    assert_eq!(h.stuck(), 0);
}
```
<!-- /shown -->

Если работа должна переживать страницу, установи её актор в скоупе,
который живёт дольше, например в приложении. `detached()` само по себе
время жизни работы не меняет.
