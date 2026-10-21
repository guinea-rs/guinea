---
title: Изменяемый период таймера
sidebar:
  order: 12
---

Допустим, приложение обновляет список процессов раз в секунду, а пользователь
меняет этот интервал в настройках. Таймер должен подхватить новое значение
без переустановки фичи.

Для этого вместо фиксированного `Duration` передай в `cx.repeat`
`Period::follows(source, map)`: источник сообщает об изменениях, а функция
преобразует его значение в длительность.

## Настройка в amethystate

Объявим интервал в миллисекундах. Аксессор `interval_ms()` возвращает хендл
поля: через него можно и читать настройку, и менять её.

<!-- shown: periods: declaring refresh settings -->
```rust
use amethystate::{Field, Store, amethystate};

#[amethystate(prefix = "refresh")]
pub struct RefreshSettings {
    #[amestate(default = 1000u64)]
    pub interval_ms: u64,
}
```
<!-- /shown -->

Для наглядности каждый тик увеличивает счётчик. В приложении на этом месте
может быть отправка актору сообщения об обновлении списка.

<!-- shown: periods: counting refreshes -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Count(pub u32);

#[reducer]
fn count(this: &mut Count, _: ()) {
    this.0 += 1;
}
```
<!-- /shown -->

## Фича получает хранилище

Зависимость фичи — `Store`, а не отдельное поле настройки. Она объявлена
аргументом установщика; guinea передаёт хранилище от установленного
провайдера. Внутри фича открывает свои настройки и создаёт таймер:

<!-- shown: periods: installing an amethystate timer -->
```rust
feature! {
    pub ConfiguredPolling {
        exports { Count }
    }
}

#[installs]
fn configured_polling(cx: &Install, store: Store) -> anyhow::Result<ConfiguredPolling> {
    let settings = RefreshSettings::try_new_with(&store)?;
    let count = cx.state::<Count>().plain();
    let period = Period::follows(Milliseconds(settings.interval_ms()), Duration::from_millis);
    cx.repeat(period, move || count.push(()));
    Ok(ConfiguredPolling(count))
}
```
<!-- /shown -->

`Duration::from_millis` переводит значение поля в период. `Milliseconds`
связывает подписку amethystate с таймером; его реализация приведена ниже.
`try_new_with` возвращает ошибку открытия настроек через `?`. Отсутствующее
хранилище — другая ситуация: это ошибка сборки зависимостей, и guinea
паникует до вызова установщика.

В приложении хранилище может предоставлять
[`guinea-plugin-store`](https://github.com/guinea-rs/guinea-plugins/tree/master/plugins/store).
Фиче не нужен поиск сервиса в контексте или `cx.settings()`: достаточно
получить `Store` аргументом. Версии плагина и guinea должны быть совместимы.

Для исполняемого примера этой главы используем небольшой плагин,
открывающий настоящее хранилище amethystate в памяти. Он не сохраняет
настройки на диск и не заменяет настройку хранения в приложении:

<!-- shown: periods: a store for the example -->
```rust
plugin! {
    pub MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder) -> anyhow::Result<MemoryStore> {
    let store = amethystate::StoreBuilder::in_memory().build()?;
    Ok(MemoryStore(store))
}
```
<!-- /shown -->

`provides { Store }` объявляет выход плагина. После успешного создания
guinea автоматически делает его доступным установщикам.

## Что происходит при изменении

В тесте плагин и фича используют одно хранилище. Меняем настройку через
сгенерированный аксессор, как это сделал бы обработчик элемента интерфейса:

<!-- shown: periods: changing an amethystate field rearms the timer -->
```rust
#[guinea::test]
fn changing_an_amethystate_field_restarts_the_timer(h: &mut Harness) {
    let MemoryStore(store) = h.plugin::<MemoryStore>(()).unwrap();
    let settings = RefreshSettings::new_with(&store);
    let interval = settings.interval_ms();
    h.install::<ConfiguredPolling>(&()).unwrap();
    h.settled();
    h.advance(Duration::from_millis(500));

    interval.set(2000);
    h.settled();
    h.advance(Duration::from_millis(1500));
    assert_eq!(h.state::<Count>().0, 0);
    h.advance(Duration::from_millis(500));
    assert_eq!(h.state::<Count>().0, 1);
    assert_eq!(interval.get(), 2000);
    h.advance(Duration::from_millis(1999));
    assert_eq!(h.state::<Count>().0, 1);
    h.advance(Duration::from_millis(1));
    assert_eq!(h.state::<Count>().0, 2);
}
```
<!-- /shown -->

Изначально период равен секунде. Через полсекунды меняем его на две секунды.
Текущее ожидание отменяется: следующий тик приходит через **две полные
секунды после изменения**, а не в прежний срок и не немедленно. Последующие
тики используют новый период. Тест проверяет время с помощью `Harness`,
без реального ожидания и без проверки записи на диск.

## Подписка на поле

Таймеру нужен источник `guinea::sources::Changing`. В этом примере локальная
обёртка над `Field<u64>` читает значение и подписывается на изменения:

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

`watch` возвращает guard подписки amethystate. Таймер хранит его; при удалении
таймера guard удаляется и подписка снимается. Если используешь совместимый
адаптер `amethystate-guinea`, у него уже есть `IntoChanging::changing()` —
повторять эту обёртку в приложении не нужно.

Импорты примера: `std::any::Any`, `std::time::Duration`,
`guinea::prelude::*`, `guinea::sources::Changing`; для теста —
`guinea::app::Harness`. Пример проверяется с amethystate 0.27 и фичей `memory`.

## Границы поведения

- Если `Changing::now()` возвращает `None`, таймер не тикает. Удаление
  значения прерывает текущее ожидание; появление значения начинает новое.
  У поля `u64` в этом примере значение есть всегда.
- Изменение периода не будит спящий скоуп. Если начатое ожидание успело
  закончиться во сне, после пробуждения может прийти один удержанный тик,
  но не очередь всех пропущенных тиков. Если срок ещё не наступил,
  пробуждение само по себе тик не создаёт.
- Удаление скоупа отменяет таймер и снимает его подписку. Само хранилище
  при этом может продолжать жить.
