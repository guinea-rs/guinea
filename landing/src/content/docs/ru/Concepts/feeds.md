---
title: Фиды
sidebar:
  order: 6
---

У фонового источника может быть значение, нужное нескольким фичам:
текущая температура, снимок процессов или состояние соединения.
Потребителям важно знать, что актуально **сейчас**, в том числе при первом
появлении или пробуждении сохранённой страницы. Все промежуточные изменения
при этом нужны не всегда.

`Feed<T>` передаёт такой последний снимок. Фича или плагин, открывшие фид,
получают `FeedWriter<T>`, а потребители — доступный только для чтения
`Feed<T>`. Источник сам решает, кому передать клон writer. Потребителю
не нужно знать, какой актор, задача или внешняя библиотека поставляет
значения.

## Открытие фида

Небольшой плагин открывает фид показаний температуры. Он не подключается
к датчику: writer возвращается приложению, которое может передать его
источнику данных. В тестах через него задаются показания.

<!-- shown: feeds: opening a feed -->
```rust
#[derive(Debug)]
pub struct Reading(pub i32);

plugin! {
    pub Thermometer {
        feeds { Reading }
        provides { FeedWriter<Reading> }
    }
}

#[builds]
fn thermometer(app: &mut PluginBuilder) -> anyhow::Result<Thermometer> {
    let writer = app.feed::<Reading>();
    Ok(Thermometer(writer.feed(), writer))
}
```
<!-- /shown -->

`app.feed::<Reading>()` открывает фид в скоупе приложения.
Секция `feeds { Reading }` объявляет возвращаемый хендл чтения,
а `provides` добавляет writer в возвращаемое значение плагина.
Как и с другими сервисами плагина, возврат writer не вызывает
`app.provide` автоматически.

Фича точно так же открывает фид в своём скоупе через `cx.feed::<T>()`.
В одном скоупе может быть только один фид каждого типа: повторное открытие
приводит к панике. В разных скоупах фиды одного типа независимы.

Тип `T` должен быть `Send + Sync + 'static`. Метод `set` принимает
как `T`, так и `Arc<T>`; фид сохраняет последнее значение. До первого
`set` начального значения нет.

## Получение снимков в фиче

Функция установки может запросить `Feed<Reading>` аргументом:

<!-- shown: feeds: following from a feature -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Seen(pub Vec<i32>);

#[reducer]
fn seen(this: &mut Seen, temperature: i32) {
    this.0.push(temperature);
}

feature! {
    pub Watching {
        exports { Seen }
    }
}

#[installs]
fn watching(cx: &Install, readings: Feed<Reading>) -> anyhow::Result<Watching> {
    let seen = cx.state::<Seen>().plain();
    cx.follow(readings, move |reading| seen.push(reading.0));
    Ok(Watching(seen))
}
```
<!-- /shown -->

Фид находится в ближайшем **родительском** скоупе, который его открыл.
Поиск не проверяет соседние скоупы и сам скоуп установки. Если фид
открыла другая фича в том же скоупе, передай возвращённый ею хендл явно,
например через параметры установки.

`cx.follow` регистрирует обработчик для скоупа потребителя. Бодрствующий
потребитель сразу получает текущее значение, если оно уже есть, а затем
получает новые снимки, пока остаётся активным. Обработчик вызывается
в UI-потоке с `Arc<Reading>` и может отправить обновление через `Bound`
или `Push`. Хендл отписки хранить не нужно: удаление скоупа потребителя
завершает подписку.

Здесь `Seen` записывает доставленные показания, чтобы тесты могли показать
их порядок. В приложении редьюсер мог бы просто заменять одну отображаемую
температуру. Сам фид историю не хранит.

<!-- shown: feeds: the current value reaches a new follower -->
```rust
#[guinea::test]
fn a_new_follower_receives_the_current_reading(h: &mut Harness) {
    let Thermometer(readings, writer) = h.plugin::<Thermometer>(()).unwrap();
    assert!(readings.get().is_none());

    writer.set(Reading(20));
    h.install::<Watching>(&()).unwrap();
    assert_eq!(h.state::<Seen>().0, [20]);

    writer.set(Reading(21));
    assert_eq!(readings.get().unwrap().0, 21);
    assert_eq!(h.state::<Seen>().0, [20, 21]);
}
```
<!-- /shown -->

## Источник: запись новых снимков

Предыдущий тест пишет через возвращённый хендл напрямую. В приложении
этот вызов делает источник данных. Вот другой вариант плагина термометра:
он оставляет writer в обработчике таймера и возвращает только хендл чтения.

<!-- shown: feeds: a producer writes readings -->
```rust
plugin! {
    pub PollingThermometer {
        feeds { Reading }
    }
}

#[builds]
fn polling_thermometer(app: &mut PluginBuilder) -> anyhow::Result<PollingThermometer> {
    let writer = app.feed::<Reading>();
    let readings = writer.feed();
    let mut temperature = 20;

    app.repeat(Duration::from_secs(1), move || {
        writer.set(Reading(temperature));
        temperature += 1;
    });

    Ok(PollingThermometer(readings))
}
```
<!-- /shown -->

Раз в секунду обработчик вызывает `writer.set(...)`. Возрастающие числа
заменяют показания датчика: пример выдаёт `20`, `21` и далее, не обращаясь
к настоящему устройству. Обработчик датчика или актор могут в этом же месте
публиковать измеренное значение.

`PollingThermometer` устанавливается вместо `Thermometer`: вместе
в одном скоупе их устанавливать нельзя, поскольку оба открывают фид
`Reading`. Фичу `Watching` менять не нужно. Следующий тест проверяет
весь путь, сам не записывая значения в фид:

<!-- shown: feeds: produced readings reach a consumer -->
```rust
#[guinea::test]
fn the_producer_writes_and_the_consumer_receives(h: &mut Harness) {
    let PollingThermometer(readings) = h.plugin::<PollingThermometer>(()).unwrap();
    h.install::<Watching>(&()).unwrap();
    assert!(readings.get().is_none());
    assert!(h.state::<Seen>().0.is_empty());

    h.advance(Duration::from_secs(1));
    assert_eq!(readings.get().unwrap().0, 20);
    assert_eq!(h.state::<Seen>().0, [20]);

    h.advance(Duration::from_secs(1));
    assert_eq!(readings.get().unwrap().0, 21);
    assert_eq!(h.state::<Seen>().0, [20, 21]);
}
```
<!-- /shown -->

Обработчик владеет writer, а скоуп приложения — таймером. Удаление
приложения останавливает источник вместе с удалением фида. Если бы
источник был внешним, его очистку тоже пришлось бы связать со скоупом.

## Чтение без подписки

Для разового чтения `feed.get()` возвращает `Option<Arc<T>>` без подписки.
`None` означает, что значение ещё не задано или скоуп источника уже удалён.
Аргумент `Option<Feed<T>>` делает необязательной саму зависимость от фида.
Это не то же самое, что существующий фид без значения.

## Сон и пробуждение

Сохранённая страница может заснуть, не удаляясь. Её подписки остаются
зарегистрированными, но во сне снимки ей не доставляются. Если фид
изменился, при пробуждении придёт только последнее значение — один раз:

<!-- shown: feeds: waking catches up once -->
```rust
#[guinea::test]
fn a_sleeping_page_catches_up_with_the_latest_reading(h: &mut Harness) {
    let Thermometer(_, writer) = h.plugin::<Thermometer>(()).unwrap();
    writer.set(Reading(20));
    let page = h.child();
    page.install::<Watching>(&()).unwrap();

    page.scope().sleep();
    writer.set(Reading(21));
    writer.set(Reading(22));
    assert_eq!(page.state::<Seen>().0, [20]);

    page.scope().wake();
    assert_eq!(page.state::<Seen>().0, [20, 22]);

    page.scope().sleep();
    page.scope().wake();
    assert_eq!(page.state::<Seen>().0, [20, 22]);
}
```
<!-- /shown -->

Промежуточное показание `21` не воспроизводится. Повторный сон
и пробуждение без новой записи не вызывают повторную доставку.
Тест вызывает `scope().sleep()` и `wake()` напрямую; жизненным циклом
смонтированной страницы обычно управляет роутер.

## Снимок — не очередь событий

Несколько записей за один UI-такт могут объединиться в одно уведомление
с последним значением. Записи из другого потока тоже объединяются:
серия записей ставит одну доставку в UI-поток, где потребители получают
последнее доступное значение. Фид не подходит, если нужно обработать
или воспроизвести каждый элемент.

Фид не сравнивает содержимое через `PartialEq`. Повторная установка того
же экземпляра `Arc` игнорируется, но новый снимок с равным содержимым
считается новой записью. Изменение содержимого уже сохранённого `Arc`
само по себе не уведомляет потребителей: публикуй новый снимок через `set`.

Выбирай механизм по нужному контракту:

- **Состояние редьюсера** хранит модель фичи и меняется через её обновления.
  Страница может прочитать его и получить диспетчер фичи-владельца.
- **Фид** передаёт последний снимок источника другим потребителям,
  с подписками по времени жизни скоупа и актуализацией после сна.
  Он не даёт диспетчер источника или право записи в него.
- **Событие шины** сообщает о том, что произошло. У него нет сохранённого
  текущего значения для потребителя, который появился позже.

Снимок фида можно преобразовать в локальное состояние редьюсера, как
делает `Watching`. От этого потребитель не становится владельцем
исходных данных.

## Производить данные только при наличии читателей

Секция, открывшая фид, может обрабатывать `Followed<T>`. Она получает
`true` при появлении первого бодрствующего подписчика и `false`, когда
последний уходит или засыпает. Источник может использовать это, чтобы
решить, нужен ли сейчас опрос данных.

Следующий плагин только записывает эти переходы. Он не запускает
и не останавливает настоящий датчик:

<!-- shown: feeds: noticing demand -->
```rust
plugin! {
    pub Demand {
        feeds { Reading }
    }
}

#[builds]
fn demand(app: &mut PluginBuilder, changes: Rc<RefCell<Vec<bool>>>) -> anyhow::Result<Demand> {
    app.answers(move |followed: Followed<Reading>| changes.borrow_mut().push(followed.0));
    let writer = app.feed::<Reading>();
    Ok(Demand(writer.feed()))
}
```
<!-- /shown -->

<!-- shown: feeds: only awake followers count -->
```rust
#[guinea::test]
fn demand_ends_only_when_the_last_awake_follower_goes(h: &mut Harness) {
    let changes = Rc::new(RefCell::new(Vec::new()));
    h.plugin::<Demand>(changes.clone()).unwrap();

    let first = h.child();
    let second = h.child();
    first.install::<Watching>(&()).unwrap();
    second.install::<Watching>(&()).unwrap();
    assert_eq!(*changes.borrow(), [true]);

    first.scope().sleep();
    assert_eq!(*changes.borrow(), [true]);
    second.leave();
    assert_eq!(*changes.borrow(), [true, false]);

    first.scope().wake();
    assert_eq!(*changes.borrow(), [true, false, true]);
    first.leave();
    assert_eq!(*changes.borrow(), [true, false, true, false]);
}
```
<!-- /shown -->

Два активных потребителя означают один `true`, а не два запроса на запуск.
Уход одного не прекращает спрос, пока другой бодрствует. Пробуждение
оставшегося потребителя снова создаёт спрос. `Followed` доставляется
в секцию, открывшую фид, а не публикуется событием в шину. Его может
обрабатывать и актор, зарегистрированный обработчиком этой секции.

Само уведомление ничего не останавливает. Обработчик должен запустить
или отменить работу источника через механизмы из главы о
[фоновых задачах](../tasks/) или через API внешней библиотеки.

## Владение и другие потоки

`Feed` реализует `Copy`, но его копия не удерживает скоуп в живых.
Уже полученный снимок в `Arc` принадлежит читателю отдельно и может
пережить скоуп источника:

<!-- shown: feeds: handles do not keep the scope alive -->
```rust
#[test]
fn a_feed_handle_does_not_keep_its_owner_alive() {
    let mut app = TestApp::new();
    let Thermometer(readings, writer) = app.plugin::<Thermometer>(()).unwrap();
    writer.set(Reading(20));
    let snapshot = readings.get().unwrap();

    drop(app);

    assert!(readings.get().is_none());
    assert_eq!(snapshot.0, 20);
}
```
<!-- /shown -->

`FeedWriter` можно клонировать и передавать в фоновую задачу или поток.
Клоны пишут в один фид: они не создают независимые источники и не
продлевают жизнь скоупа. После удаления скоупа источника запись
из UI-потока приводит к панике. Поздняя запись из другого потока
вместо этого отбрасывается с предупреждением.

Удаление фида само по себе не останавливает произвольные внешние потоки.
Их завершение тоже нужно связать со скоупом — через отмену задачи или
[`on_teardown`](../scopes/#ресурс-другой-библиотеки-на-время-жизни-страницы).
