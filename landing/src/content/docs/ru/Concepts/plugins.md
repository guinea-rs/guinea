---
title: Установка плагинов
sidebar:
  order: 4
---

Фича описывает часть твоего приложения. Плагин объединяет то, что может
пригодиться разным приложениям: хранилище настроек, локализацию, сохранение
положения окон или инструменты разработчика. Он знает о guinea, но не должен
знать о страницах и фичах конкретного приложения.

Плагины — это Rust-код, с которым собирается приложение, а не модули,
загружаемые во время работы. Добавь крейт плагина в зависимости, затем вызови
`app.plugin::<P>(params)?` в функции приложения. Параметры и возвращаемое
значение плагина определяют, как связать его с остальным кодом.

В этой главе небольшой плагин определён рядом с приложением, чтобы был виден
весь его контракт. Плагин из другого крейта подключается так же. Этот код
не зависит от GUI-бэкенда.

## Что возвращает установка

`MemoryStore` создаёт хранилище «ключ — значение» и возвращает его хендл.
Данные остаются только в памяти: пример не открывает файл и не сохраняет
настройки между запусками.

<!-- shown: plugins: a plugin and its service -->
```rust
#[derive(Clone, Debug, Default)]
pub struct Store(Rc<RefCell<BTreeMap<String, String>>>);

impl Store {
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.borrow().get(key).cloned()
    }

    pub fn set(&self, key: &str, value: &str) {
        self.0.borrow_mut().insert(key.to_owned(), value.to_owned());
    }
}

plugin! {
    pub MemoryStore {
        provides { Store }
    }
}

#[builds]
fn memory_store(_app: &mut PluginBuilder, theme: &'static str) -> anyhow::Result<MemoryStore> {
    let store = Store::default();
    store.set("theme", theme);
    Ok(MemoryStore(store))
}
```
<!-- /shown -->

`plugin!` объявляет возвращаемое поле `Store` в секции `provides`.
Атрибут `#[builds]` реализует создание плагина: аргумент после `PluginBuilder`
становится его параметрами. Здесь это начальная тема оформления.

Клоны `Store` обращаются к одной карте. Ещё один вызов установки `MemoryStore`
создаёт отдельную карту. Приложение может создать оба хранилища и выбрать,
какое из них будут получать страницы.

## Выбор делает приложение

<!-- shown: plugins: application wiring -->
```rust
fn app(app: &mut FeatureBuilder) -> anyhow::Result<Store> {
    let MemoryStore(user) = app.plugin::<MemoryStore>("dark")?;
    let MemoryStore(internal) = app.plugin::<MemoryStore>("light")?;
    app.provide(user);
    Ok(internal)
}
```
<!-- /shown -->

Каждый вызов `app.plugin` сразу создаёт новый экземпляр. Повторная установка
плагина того же типа не пропускается. Здесь `user` и `internal` — независимые
хранилища, хотя у них один Rust-тип.

Вызов `app.provide(user)` делает выбранный хендл доступным как сервис `Store`
приложения. Сам по себе возврат значения из плагина **не** вызывает `provide`:

<!-- shown: plugins: returned is not provided -->
```rust
#[test]
fn returning_a_service_does_not_publish_it() {
    let mut app = TestApp::new();
    let MemoryStore(store) = app.plugin::<MemoryStore>("dark").unwrap();

    assert!(app.service::<Store>().is_none());
    app.provide(store);
    assert_eq!(
        app.service::<Store>().unwrap().get("theme").as_deref(),
        Some("dark")
    );
}
```
<!-- /shown -->

На каждый Rust-тип в приложении приходится один предоставленный сервис.
Второй вызов `provide` для `Store` — ошибка сборки приложения, которая
приводит к панике; создать второе хранилище при этом можно. Если разным
потребителям нужны разные хранилища, передавай хендлы явно или используй
отдельные типы-обёртки.

Функция `app` в примере возвращает внутреннее хранилище, чтобы тест мог его
проверить. В GUI-приложении такой код располагается до создания и возврата
`Window` выбранного бэкенда, как в главе о [страницах и лейаутах](../pages/).

## Получение сервиса

Фича объявляет нужный сервис аргументом после контекста установки:

<!-- shown: plugins: a feature takes a service -->
```rust
#[derive(Default, Clone, PartialEq, Debug)]
pub struct Theme(pub String);

#[reducer]
fn theme(this: &mut Theme, value: String) {
    this.0 = value;
}

feature! {
    pub Appearance {
        exports { Theme }
    }
}

#[installs]
fn appearance(cx: &Install, store: Store) -> anyhow::Result<Appearance> {
    let theme = cx
        .state::<Theme>()
        .seed(Theme(store.get("theme").unwrap_or_default()))
        .plain();
    Ok(Appearance(theme))
}
```
<!-- /shown -->

Атрибут `#[installs]` находит `Store` и передаёт его клон в функцию.
К этому моменту сервис уже должен быть предоставлен. Если он необязателен,
используй `Option<Store>`; отсутствие обязательного сервиса — ошибка
сборки приложения.

`Appearance` копирует сохранённую тему в начальное состояние. Это не
подписка: последующие изменения карты не обновляют установленный `Theme`.
Для реактивных изменений нужна явная подписка или другой источник обновлений.

Тестовый хёрнесс выполняет ту же функцию приложения и устанавливает фичу
в оконном скоупе, не открывая настоящее окно:

<!-- shown: plugins: testing application wiring -->
```rust
#[guinea::test]
fn the_application_chooses_which_store_its_pages_receive(h: &mut Harness) {
    let internal = h.app(app).unwrap();
    h.install::<Appearance>(&()).unwrap();

    assert_eq!(h.state::<Theme>().0, "dark");
    assert_eq!(internal.get("theme").as_deref(), Some("light"));

    internal.set("theme", "blue");
    let page = h.child();
    page.install::<Appearance>(&()).unwrap();
    assert_eq!(page.state::<Theme>().0, "dark");
}
```
<!-- /shown -->

Дочерний скоуп устанавливает новую `Appearance` уже после изменения
внутреннего хранилища. Она по-прежнему получает пользовательское хранилище.
Так тест проверяет выбор сервиса, а не только значение, скопированное раньше
в состояние родителя.

## Связь между плагинами

Если одному плагину нужен сервис другого, сначала установи поставщика,
а затем передай возвращённый хендл в параметрах потребителя. Приложение явно
выбирает экземпляр и порядок установки. guinea не строит граф зависимостей
и не устанавливает их автоматически.

У двух секций объявления разные задачи:

- `provides { Store }` добавляет хендл сервиса в возвращаемое значение плагина.
  Приложение решает, кому его передать и нужен ли вызов `provide`.
- `exports { SomeState }` после успешной установки делает состояние редьюсера,
  зарегистрированное плагином, доступным окнам и их потомкам. Оно находится
  по правилам из главы о [скоупах](../scopes/).

Сервис не становится состоянием редьюсера автоматически. Возвращённый хендл
тоже не становится общей зависимостью приложения без явного решения.

## Время жизни и ошибки

Плагин регистрирует акторы, таймеры и подписки в скоупе приложения.
Удаление возвращённого значения плагина их не отключает. Они живут вместе
с хостом приложения, а не со страницей, которая ими пользуется.

Если ресурсу нужна явная очистка, билдер предоставляет `on_cleanup`:

<!-- shown: plugins: registering cleanup -->
```rust
plugin! {
    Cleanup {}
}

#[builds]
fn cleanup(app: &mut PluginBuilder, cleaned: Rc<Cell<bool>>) -> anyhow::Result<Cleanup> {
    app.on_cleanup(move || {
        cleaned.set(true);
        Ok(())
    });
    Ok(Cleanup)
}
```
<!-- /shown -->

Здесь флаг нужен только для проверки момента очистки:

<!-- shown: plugins: cleanup belongs to the application -->
```rust
#[test]
fn a_plugins_cleanup_runs_when_the_application_ends() {
    let cleaned = Rc::new(Cell::new(false));
    let mut app = TestApp::new();
    app.plugin::<Cleanup>(cleaned.clone()).unwrap();

    assert!(!cleaned.get());
    drop(app);
    assert!(cleaned.get());
}
```
<!-- /shown -->

Обработчики очистки вызываются в обратном порядке регистрации; возвращённая
ими ошибка записывается в лог.

### Ошибка посередине установки

Допустим, плагин индексации создал lock-файл, зарегистрировал его удаление
при завершении приложения, запустил таймер опроса и записал лог запуска.
Затем он пытается прочитать `index.toml`. Если файла нет, установка
возвращает ошибку, но предыдущие действия уже выполнены.

В примере намеренно выбран такой порядок, чтобы увидеть частичную
установку. Вместо настоящей индексации таймер увеличивает счётчик:

<!-- shown: plugins: a partially built indexer -->
```rust
pub struct IndexingParams {
    pub directory: PathBuf,
    pub polls: Rc<Cell<u32>>,
}

plugin! {
    Indexing {}
}

#[builds]
fn indexing(app: &mut PluginBuilder, params: IndexingParams) -> anyhow::Result<Indexing> {
    let lock = params.directory.join("index.lock");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)?;
    app.on_cleanup(move || {
        std::fs::remove_file(lock)?;
        Ok(())
    });

    let polls = params.polls;
    app.repeat(Duration::from_secs(1), move || polls.set(polls.get() + 1));
    std::fs::write(params.directory.join("startup.log"), b"starting indexer\n")?;

    let _settings = std::fs::read_to_string(params.directory.join("index.toml"))?;
    Ok(Indexing)
}
```
<!-- /shown -->

Если вызывающий код перехватит ошибку и оставит приложение работать,
таймер продолжит срабатывать, а lock-файл останется на месте.
Зарегистрированная очистка принадлежит скоупу, а не неудачному вызову
функции:

<!-- shown: plugins: catching an error keeps the scope -->
```rust
#[guinea::test]
fn catching_an_indexer_error_leaves_its_timer_running(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let polls = Rc::new(Cell::new(0));
    let mut app = TestApp::new();
    let result = app.plugin::<Indexing>(IndexingParams {
        directory: directory.path().to_owned(),
        polls: polls.clone(),
    });

    assert!(result.is_err());
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 3);
    assert!(directory.path().join("index.lock").exists());

    drop(app);
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 3);
    assert!(!directory.path().join("index.lock").exists());
    assert!(directory.path().join("startup.log").exists());
}
```
<!-- /shown -->

Удаление приложения останавливает таймер и вызывает удаление lock-файла.
Но `startup.log` остаётся: действие для его удаления никто не регистрировал.
Ни ошибка установки, ни завершение приложения сами по себе не обращают
запись в файловую систему назад.

Для обязательного плагина передавай ошибку через `?` из функции запуска.
Следующий тест моделирует владение через `TestApp`: при ошибке локальное
приложение удаляется, а не возвращается вызывающему коду.

<!-- shown: plugins: propagating an error drops the host -->
```rust
#[guinea::test]
fn an_indexer_error_propagated_out_of_startup_releases_the_scope(h: &mut Harness) {
    let directory = tempfile::tempdir().unwrap();
    let polls = Rc::new(Cell::new(0));

    let result = (|| -> anyhow::Result<TestApp> {
        let mut app = TestApp::new();
        app.plugin::<Indexing>(IndexingParams {
            directory: directory.path().to_owned(),
            polls: polls.clone(),
        })?;
        Ok(app)
    })();

    assert!(result.is_err());
    h.advance(Duration::from_secs(3));
    assert_eq!(polls.get(), 0);
    assert!(!directory.path().join("index.lock").exists());
    assert!(directory.path().join("startup.log").exists());
}
```
<!-- /shown -->

Метод `run` бэкенда так же владеет хостом, пока вызывает функцию приложения.
Если та возвращает ошибку, хост удаляется вместе со всем, что было
установлено до неё.

В рабочем плагине по возможности проверяй настройки до запуска работы.
Если плагин необязателен и приложение должно продолжить запуск без него,
позаботься об очистке частичной установки до возврата ошибки. Не рассчитывай
на автоматический откат в `app.plugin`: при ошибке экспорты не публикуются,
владение состоянием и маршрутизация действий отзываются, но уже созданные
ресурсы остаются до удаления скоупа.

Готовые плагины находятся в отдельном
[репозитории guinea-plugins](https://github.com/guinea-rs/guinea-plugins).
Версия плагина должна быть совместима с версией guinea, с которой собирается
приложение.
