---
title: Сон, пробуждение и фоновая работа
sidebar:
  order: 10
---

Сохранённая через `keep` страница остаётся установленной. Сон — не удаление
и не общая пауза для всего скоупа. У каждого механизма свои правила:

| Ресурс | Во сне | После пробуждения | При удалении |
| --- | --- | --- | --- |
| Поля страницы, лейаута и актора, состояние редьюсера | Сохраняются | Те же экземпляры | Освобождаются со скоупом |
| Подписки акторов и замыканий на шину | Пропускают события | Без воспроизведения | Снимаются |
| Подписчик фида | Не вызывается | Один актуальный изменившийся снимок | Подписка снимается |
| Источник стрима | Приостанавливает извлечение и доставку; начатое извлечение может закончиться | Доставляет удержанные элементы по порядку и продолжает чтение | Отменяется |
| Таймер | Не вызывает обработчик | Не больше одного просроченного тика, затем следующий период | Отменяется |
| Обычная задача, включая `latest` и `conflate` | Продолжает работу; ответ может изменить состояние | Специального воспроизведения нет | Отменяется; ожидающий результат отбрасывается |
| Кооперативная задача `spawn_with` | Продолжает работу | Специального воспроизведения нет | Получает отмену; очистка может пережить скоуп |

`keep()` сохраняет не только состояние фичи, но и локальные поля страницы.
Удаление родителя удаляет поддерево. Сам по себе сон родителя не стоит
считать обещанием рекурсивного сна произвольных скоупов, созданных вручную:
пользуйся жизненным циклом смонтированного дерева, а не только переключением
одного родительского флага.

## Обычный ответ может прийти во сне

Фича ниже запускает обычную задачу с ответом актору, ещё одну через
`pushing` и подписывается на фид. Вокруг подписки регистрируются два
обработчика пробуждения:

<!-- shown: lifecycle: ordinary work and wake hooks -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Log(pub Vec<String>);

#[reducer]
fn log(this: &mut Log, item: String) {
    this.0.push(item);
}

pub struct Begin;
pub struct Answered;
pub struct Queue;
pub struct Direct;

#[derive(Debug)]
struct Worker {
    push: Push<Log>,
}

actor! {
    Worker {
        handlers {
            Begin => { task Answered }
            Answered,
            Queue => { send Answered }
            Direct
        }
    }
}

#[handler]
fn begin(In(_, _, cx): In<Worker, Begin>) {
    cx.task().latest().spawn(async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Answered
    });
}

#[handler]
fn answered(In(this, _, _): In<Worker, Answered>) {
    this.push.send("answer".into());
}

#[handler]
fn queue(In(_, _, cx): In<Worker, Queue>) {
    cx.send(Answered);
}

#[handler]
fn direct(In(this, _, cx): In<Worker, Direct>) {
    cx.task().pushing(this.push).spawn(async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        "direct".to_owned()
    });
}

feature! {
    pub Working {
        exports { Log }
    }
}

#[installs]
fn working(cx: &Install<Feed<u32>>) -> anyhow::Result<Working> {
    let (log, _) = cx.state::<Log>().driven_by(|push| Worker { push });
    cx.on_wake(move || log.push("before feed".into()));
    cx.follow(*cx.params(), move |value| log.push(format!("feed {value}")));
    cx.on_wake(move || log.push("after feed".into()));
    Ok(Working(log))
}
```
<!-- /shown -->

Импорты: `guinea::prelude::*`, `guinea::app::Harness` и
`std::time::Duration`. Имитация работы использует фичу Tokio `time`.
Тест открывает фид в родительском скоупе хёрнесса и отправляет дочерний
скоуп спать, пока задачи ожидают:

<!-- shown: lifecycle: a task can answer during sleep -->
```rust
#[guinea::test]
fn sleep_does_not_pause_ordinary_tasks_or_their_answers(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    writer.set(1);
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    page.dispatch::<Log>().emit(Begin);
    page.dispatch::<Log>().emit(Direct);
    h.settled();

    page.scope().sleep();
    h.advance(Duration::from_secs(1));
    let log = page.state::<Log>();
    assert_eq!(log.0.len(), 3);
    assert!(log.0.contains(&"answer".into()));
    assert!(log.0.contains(&"direct".into()));
}
```
<!-- /shown -->

Оба результата меняют состояние спящей страницы. Их взаимный порядок
не гарантирован. Ответ обычной задачи — не событие шины и не элемент
стрима. Прямые отправки по адресу и действия фичи тоже не блокируются
сном. Сообщения, отправленные актором самому себе во время обработки,
разбираются после текущего обработчика: сон не превращает эту очередь
в очередь до пробуждения. Удаление актора, в отличие от сна, прекращает
дальнейшую обработку очереди.

Политика `latest` продолжает отбрасывать заменённые ответы, а `conflate`
может завершить текущую работу и начать последний ожидающий запрос
во сне. Ни одна из них не означает «ждать, пока страница видима».
Если видимость должна останавливать работу, явно свяжи обработчики
сна и пробуждения с её отменой и повторным запуском.

Заменённый запуск отбрасывается и тогда, когда между запросами страница
заснула:

<!-- shown: lifecycle: latest while asleep -->
```rust
#[guinea::test]
fn latest_still_rejects_the_old_answer_while_the_page_sleeps(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    let dispatch = page.dispatch::<Log>();
    dispatch.emit(Begin);
    h.settled();
    page.scope().sleep();
    h.advance(Duration::from_millis(500));
    dispatch.emit(Begin);
    h.settled();
    h.advance(Duration::from_millis(500));
    assert!(page.state::<Log>().0.is_empty());
    h.advance(Duration::from_millis(500));
    assert_eq!(page.state::<Log>().0, ["answer"]);
}
```
<!-- /shown -->

## Порядок пробуждения

Пробуждение сначала помечает скоуп активным, затем вызывает обработчики
в порядке регистрации. `follow` тоже регистрирует такой обработчик.
Общего правила «все пользовательские хуки до актуализации фида» или
«все после неё» нет:

<!-- shown: lifecycle: wake hooks follow registration order -->
```rust
#[guinea::test]
fn feed_catchup_runs_between_the_hooks_registered_around_follow(h: &mut Harness) {
    let writer = h.segment().scope().section().feed::<u32>();
    writer.set(1);
    let page = h.child();
    page.install::<Working>(&writer.feed()).unwrap();
    page.scope().sleep();
    writer.set(2);
    assert_eq!(page.state::<Log>().0, ["feed 1"]);

    page.scope().wake();
    assert_eq!(
        page.state::<Log>().0,
        ["feed 1", "before feed", "feed 2", "after feed"]
    );
}
```
<!-- /shown -->

В примере хук, зарегистрированный до `follow`, видит старое состояние;
затем подписка фида обновляет его, и более поздний хук вызывается после
неё. Это порядок этих синхронных обработчиков, а не всех фоновых
и поставленных в UI-очередь последствий. Доставка источников возобновляется
асинхронно. Если несколько входов нужно согласовать вместе, делай это
явно: пробуждение не даёт единого глобально установившегося снимка.

## Выбор времени жизни

Используй `keep` для состояния, которое должно переживать временную
смену маршрута. Сбор данных, который должен продолжаться после удаления
страницы, размещай в лейауте или приложении. Спящая страница — не надёжный
сборщик событий: события шины пропускаются, а буферы внешних источников
могут заполниться или переполниться.

Удаление отменяет обычную работу независимо от того, спала ли страница.
Кооперативная очистка устроена иначе: пример с каналами и тесты есть
в главе о [фоновых задачах](../tasks/#асинхронная-очистка-после-отмены).
