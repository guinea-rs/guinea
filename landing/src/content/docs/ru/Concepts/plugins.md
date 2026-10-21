---
title: Установка плагинов
sidebar:
  order: 4
---

Плагин объединяет то, что может пригодиться разным приложениям: настройки,
локализацию, сохранение положения окон или инструменты разработчика. Он
знает о guinea, но не о страницах конкретного приложения. Плагины — Rust-код,
с которым собирается приложение, а не загружаемые во время работы модули.

## Объяви выходы

Общая модель входов, выходов и доступности в скоупах описана в главе
[«Зависимости и сборка приложения»](../dependencies/).

Этот плагин создаёт хранилище в памяти. Секция `provides` перечисляет
возвращаемые зависимости:

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

Первый аргумент после `PluginBuilder` — явно переданная конфигурация
плагина. Аргументы после него — зависимости, передаваемые фреймворком.
Если плагину нужны зависимости, но не нужна конфигурация, используй `()`
в качестве параметров.

## Установи плагин

<!-- shown: plugins: application wiring -->
```rust
fn app(app: &mut FeatureBuilder) -> anyhow::Result<Store> {
    let MemoryStore(user) = app.plugin::<MemoryStore>("dark")?;
    Ok(user)
}
```
<!-- /shown -->

После успешного создания guinea автоматически публикует выходы из
`provides`. Ни ручная регистрация, ни поиск сервиса не нужны. Клоны `Store`
обращаются к одной карте. Второе значение того же типа на одной ветке
скоупов — ошибка композиции: для разных хранилищ используй разные типы-обёртки.

<!-- shown: plugins: declared services are published -->
```rust
#[guinea::test]
fn declared_services_are_published_automatically(h: &mut Harness) {
    h.plugin::<MemoryStore>("dark").unwrap();
    h.install::<Appearance>(&()).unwrap();
    assert_eq!(h.state::<Theme>().0, "dark");
}
```
<!-- /shown -->

## Получай зависимости аргументами

Фича перечисляет зависимости после контекста установки:

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

Фреймворк передаёт клон `Store`. Так же получают зависимости фичи уровня
приложения и методы `install`, обработанные `#[page]` и `#[layout]`.

Необязательная зависимость записывается как `Option<Store>`. Она получает
`None`, только если поставщик вообще не объявлен. Если поставщик объявлен,
но его выход недоступен, это ошибка композиции даже для необязательного
входа. Фреймворк паникует до входа в тело установщика.

`Appearance` копирует текущую тему в состояние, а не подписывается на
изменения. Новый экземпляр фичи читает актуальное значение:

<!-- shown: plugins: testing application wiring -->
```rust
#[guinea::test]
fn the_application_shares_its_declared_store_with_pages(h: &mut Harness) {
    let store = h.app(app).unwrap();
    h.install::<Appearance>(&()).unwrap();

    assert_eq!(h.state::<Theme>().0, "dark");
    assert_eq!(store.get("theme").as_deref(), Some("dark"));

    store.set("theme", "blue");
    let page = h.child();
    page.install::<Appearance>(&()).unwrap();
    assert_eq!(page.state::<Theme>().0, "blue");
}
```
<!-- /shown -->

## Выходы приложения

Фичи уровня приложения используют ту же секцию `provides`: возвращают
объявленные выходы вместо ручной регистрации значений.

<!-- shown: plugins: application outputs -->
```rust
#[derive(Clone)]
pub struct AppName(pub &'static str);

feature! {
    pub Application {
        provides { AppName }
    }
}

#[installs]
fn application(app: &mut FeatureBuilder) -> anyhow::Result<Application> {
    app.plugin::<MemoryStore>("dark")?;
    Ok(Application(AppName("guinea")))
}
```
<!-- /shown -->

`Application` публикует `AppName`, а установленный ею плагин `MemoryStore`
публикует `Store`. Установщики, которым они нужны, перечисляют их аргументами.

У трёх секций выходов разные задачи:

- `provides` публикует копируемые через `Clone` значения зависимостей.
- `exports` открывает состояние редьюсеров для чтения ниже по дереву.
- `feeds` открывает потоки значений для дочерних сегментов.

Зависимость принадлежит скоупу установщика, который её вернул, и доступна
в нём и его потомках, но не в соседних скоупах. При удалении скоупа
сохранённые им значения зависимостей освобождаются. Выходы приложения и
плагинов принадлежат скоупу приложения, поэтому доступны всем его окнам.

В графе конкретных деклараций у типа сервиса должен быть один объявленный
поставщик. Одного локального поставщика можно установить в разных соседних
скоупах: каждый экземпляр владеет своим значением. Подменять зависимость
другим значением ниже по той же ветке нельзя.

## Проверка деклараций

Макросы регистрируют контракты установщиков через `inventory`. До вызова
функции приложения бэкендом guinea проверяет конкретные декларации
итогового бинарника: отсутствие обязательных поставщиков, конфликтующие
поставщики сервисов, противоречивые входы, повторяющиеся выходы и циклы
зависимостей. Тела установщиков не выполняются, ресурсы не открываются,
вызовы внутри функций не анализируются. Эту же проверку можно запустить
явно через `guinea::app::validate_declarations()`.

Неверный контракт вызывает панику со структурированным деревом диагностики
`miette` и местами объявлений. Перед входом в каждый установщик отдельно
проверяется доступность входов: декларация ещё не означает, что поставщик
уже выполнился или фид доступен из этого скоупа. Реальные отказы, например
ошибка открытия файла, по-прежнему передаются через `Result`.

Одни декларации фидов не описывают дерево монтирования, поэтому начальная
проверка не устанавливает отношение предка и потомка. Обобщённые
установщики регистрируются как шаблоны; конкретные входы и выходы проверяются
при установке специализации. Если семейство обобщённых выходов может
предоставить недостающий вход, начальная проверка откладывает его проверку,
а не отвергает потенциально корректный граф. Запрошенная специализация
считается объявленным поставщиком даже после ошибки создания ресурса:
необязательный потребитель не может незаметно превратить этот отказ в `None`.
В тестах значения фикстур передаются через
плагины с объявленными выходами — по тому же пути установки, что в приложении.

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
