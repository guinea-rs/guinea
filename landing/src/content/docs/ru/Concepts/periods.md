---
title: Изменяемый период таймера
sidebar:
  order: 12
---

Фиксированный таймер принимает `Duration`. Вариант
`Period::follows(source, map)` читает период из наблюдаемого значения.
Изменение значения прерывает текущее ожидание и начинает новый полный
период, а не сохраняет остаток старого. Пока значения нет, тиков нет.

## Период из amethystate

Для настроек приложения есть
[`guinea-plugin-store`](https://github.com/guinea-rs/guinea-plugins/tree/master/plugins/store).
Плагин открывает и закрывает хранилище. Его extension trait `StoreAccess`
предоставляет `settings::<S>()` и `try_settings::<S>()`: они открывают
объявленную структуру настроек над этим хранилищем. Создавать отдельные
поля вручную или обращаться к глобальному хранилищу amethystate не нужно.

Текущий плагин ещё использует прежний API приложения guinea.
Его расширение контекстов пока несовместимо с API этой книги.
До обновления интеграции исполняемый пример ниже открывает настройки
верхнеуровневым конструктором amethystate `new_with(&store)`.
За работающий пример установки плагина эта версия не выдаётся.

### Объявление настройки

Допустим, пользователь выбирает частоту обновления списка процессов.
Объяви интервал в миллисекундах в структуре настроек:

<!-- shown: periods: declaring refresh settings -->
```rust
use amethystate::{Field, amethystate};

#[amethystate(prefix = "refresh")]
pub struct RefreshSettings {
    #[amestate(default = 1000u64)]
    pub interval_ms: u64,
}
```
<!-- /shown -->

`settings.interval_ms()` возвращает хендл поля. Ручной путь поля, UUID
и тестовые функции для этого не нужны. Передай хендл в фичу:

<!-- shown: periods: installing an amethystate timer -->
```rust
feature! {
    pub ConfiguredPolling {
        exports { Count }
    }
}

#[installs]
fn configured_polling(cx: &Install<Field<u64>>) -> anyhow::Result<ConfiguredPolling> {
    let count = cx.state::<Count>().plain();
    let period = Period::follows(Milliseconds(cx.params().clone()), Duration::from_millis);
    cx.repeat(period, move || count.push(()));
    Ok(ConfiguredPolling(count))
}
```
<!-- /shown -->

`Count` — редьюсер, определённый ниже. Запись через аксессор настройки
начинает ожидание таймера заново; переустанавливать фичу не нужно.

### Проверка изменения интервала

Тест открывает хранилище в памяти публичным API `StoreBuilder`,
создаёт `RefreshSettings` и использует сгенерированный аксессор.
Пример использует amethystate 0.26 с фичей `memory`:

<!-- shown: periods: changing an amethystate field rearms the timer -->
```rust
#[guinea::test]
fn changing_an_amethystate_field_restarts_the_timer(h: &mut Harness) {
    let store = amethystate::StoreBuilder::in_memory().build().unwrap();
    let settings = RefreshSettings::new_with(&store);
    let interval = settings.interval_ms();
    h.install::<ConfiguredPolling>(&interval).unwrap();
    h.settled();
    h.advance(Duration::from_millis(500));

    interval.set(2000);
    h.settled();
    h.advance(Duration::from_millis(1500));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 1);
    assert_eq!(interval.get(), 2000);
}
```
<!-- /shown -->

Поле меняется через половину исходной секунды. Тик не приходит
в прежний срок: таймер ждёт две полные секунды после изменения.
Тест проверяет этот порядок, а не сохранение настроек на диск.

### Переходник к текущему API таймера

Отдельный адаптер `amethystate-guinea` тоже пока использует прежний
`guinea_app::timers::Changing`. Его `.changing()` нельзя без изменений
подставить вместо текущего `guinea::sources::Changing`. Поэтому в примере
используется небольшой локальный переходник:

<!-- shown: periods: an amethystate field adapter -->
```rust
pub struct Milliseconds(Field<u64>);

impl Changing for Milliseconds {
    type Value = u64;

    fn now(&self) -> Option<u64> {
        Some(self.0.get())
    }

    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        Box::new(self.0.subscribe(move |_| changed()))
    }
}
```
<!-- /shown -->

`now` читает значение поля — оно всегда есть. `watch` возвращает guard
подписки amethystate: таймер владеет им, а удаление guard снимает подписку.
Остальные импорты — `std::any::Any`, `std::time::Duration`,
`guinea::sources::Changing` и `guinea::prelude::*`.

## Своя наблюдаемая настройка

Источник реализует `guinea::sources::Changing`: чтение текущего значения
и регистрацию потокобезопасного обработчика изменений. Возвращённый
из `watch` объект должен снимать эту подписку при удалении.
`Period` хранит его на время жизни таймера.

Вот небольшая настройка в памяти, не зависящая от библиотеки хранения:

<!-- shown: periods: an observable interval -->
```rust
type Changed = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct IntervalState {
    value: Option<Duration>,
    next: u64,
    watchers: BTreeMap<u64, Changed>,
}

#[derive(Clone, Default)]
pub struct Interval(Arc<Mutex<IntervalState>>);

impl Interval {
    pub fn set(&self, value: Option<Duration>) {
        let watchers: Vec<_> = {
            let mut state = self.0.lock().unwrap();
            state.value = value;
            state.watchers.values().cloned().collect()
        };
        for changed in watchers {
            changed();
        }
    }
}

struct Unwatch(Interval, u64);

impl Drop for Unwatch {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().watchers.remove(&self.1);
    }
}

impl Changing for Interval {
    type Value = Duration;

    fn now(&self) -> Option<Duration> {
        self.0.lock().unwrap().value
    }

    fn watch(&self, changed: Box<dyn Fn() + Send + Sync>) -> Box<dyn Any + Send> {
        let mut state = self.0.lock().unwrap();
        state.next += 1;
        let id = state.next;
        state.watchers.insert(id, Arc::from(changed));
        Box::new(Unwatch(self.clone(), id))
    }
}
```
<!-- /shown -->

Импорты: `std::any::Any`, `std::collections::BTreeMap`,
`std::sync::{Arc, Mutex}`, `std::time::Duration` и
`guinea::sources::Changing`. Перед уведомлением обработчики копируются
из-под блокировки: их вызов не требует удерживать мьютекс настройки.
Возвращаемый guard `Unwatch` удаляет именно свою регистрацию.

Если у реактивной библиотеки уже есть адаптер guinea, используй его
реализацию `Changing`, а не пиши ещё одну для того же типа.

## Установка таймера

<!-- shown: periods: installing a dynamic timer -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

#[reducer]
fn count(this: &mut Count, _: ()) {
    this.0 += 1;
}

feature! {
    pub Polling {
        exports { Count }
    }
}

#[installs]
fn polling(cx: &Install<Interval>) -> anyhow::Result<Polling> {
    let count = cx.state::<Count>().plain();
    let period = Period::follows(cx.params().clone(), std::convert::identity);
    cx.repeat(period, move || count.push(()));
    Ok(Polling(count))
}
```
<!-- /shown -->

Здесь используется `guinea::prelude::*`. Настройка `Interval` уже хранит
`Duration`, поэтому достаточно `identity`. Для числового значения
в миллисекундах функцией преобразования могла бы быть
`Duration::from_millis`.

`Period` — не публичный enum для разбора по вариантам. Создавай его
через `Period::follows` или передавай фиксированный `Duration` туда,
где принимается `Into<Period>`.

<!-- shown: periods: a change restarts the wait -->
```rust
#[guinea::test]
fn a_period_change_starts_a_new_wait_instead_of_finishing_the_old_one(h: &mut Harness) {
    let interval = Interval::default();
    h.install::<Polling>(&interval).unwrap();
    h.advance(Duration::from_secs(10));
    assert_eq!(h.state::<Count>().0, 0);

    interval.set(Some(Duration::from_secs(1)));
    h.settled();
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 0);
    interval.set(Some(Duration::from_secs(2)));
    h.settled();
    h.advance(Duration::from_millis(1500));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 1);
}
```
<!-- /shown -->

Изначально значения нет, поэтому даже за десять секунд тика не происходит.
Установка одной секунды начинает первое ожидание; изменение посередине
этой секунды на две секунды начинает его заново. Следующий тик произойдёт
через две секунды после изменения, а не в исходный момент и не немедленно.

Удаление значения также прерывает уже начатое ожидание; восстановление
начинает новое:

<!-- shown: periods: removing the interval -->
```rust
#[guinea::test]
fn removing_the_period_value_disarms_the_current_wait(h: &mut Harness) {
    let interval = Interval::default();
    interval.set(Some(Duration::from_secs(1)));
    h.install::<Polling>(&interval).unwrap();
    h.settled();
    h.advance(Duration::from_millis(500));
    interval.set(None);
    h.advance(Duration::from_secs(10));
    assert_eq!(h.state::<Count>().0, 0);
    interval.set(Some(Duration::from_secs(1)));
    h.settled();
    h.advance(Duration::from_secs(1));
    assert_eq!(h.state::<Count>().0, 1);
}
```
<!-- /shown -->

## Сон и изменение настройки

Изменение настройки само по себе не возобновляет вызовы для спящего
потребителя. Уже начатое извлечение тика может просрочиться во сне.
При пробуждении приходит один удержанный тик, затем ожидания идут
с актуальным периодом:

<!-- shown: periods: changes while asleep -->
```rust
#[guinea::test]
fn a_changed_period_during_sleep_does_not_replay_missed_ticks(h: &mut Harness) {
    let interval = Interval::default();
    interval.set(Some(Duration::from_secs(1)));
    let page = h.child();
    page.install::<Polling>(&interval).unwrap();
    h.settled();
    page.scope().sleep();
    interval.set(Some(Duration::from_millis(500)));
    h.advance(Duration::from_secs(5));
    assert_eq!(page.state::<Count>().0, 0);

    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Count>().0, 1);
    h.advance(Duration::from_millis(500));
    assert_eq!(page.state::<Count>().0, 2);

    page.leave();
    h.settled();
    assert!(interval.0.lock().unwrap().watchers.is_empty());
}
```
<!-- /shown -->

Этот тест начинает ожидание до сна, меняет период и спит достаточно долго,
чтобы тик просрочился. Он получает один тик, а не десять пропущенных
полусекундных интервалов. Это не обещание тика при каждом пробуждении:
если ожидание ещё не закончилось, оно должно закончиться. Удаление
страницы отменяет источник и снимает подписку настройки. Сама настройка
может жить дольше.

Пробуждение до срока ожидания само по себе не создаёт тик:

<!-- shown: periods: waking before the deadline -->
```rust
#[guinea::test]
fn waking_before_a_deadline_does_not_create_an_immediate_tick(h: &mut Harness) {
    let interval = Interval::default();
    interval.set(Some(Duration::from_secs(1)));
    let page = h.child();
    page.install::<Polling>(&interval).unwrap();
    h.settled();
    h.advance(Duration::from_millis(250));
    page.scope().sleep();
    h.advance(Duration::from_millis(250));
    page.scope().wake();
    h.settled();
    assert_eq!(page.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(page.state::<Count>().0, 1);
}
```
<!-- /shown -->
