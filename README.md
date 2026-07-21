# Synthos

Frameless messenger-style приложение на `syngui`.

## Быстрый старт

```bash
cargo run -p synthos --features desktop
```

## Архитектура

### Верхний уровень

```
titlebar
│
└── Row
    ├── nav_rail (реактивный; слушает current_route)
    └── RouterView
        ├── chat      → Row[chats_column, chat_pane, right_panel]
        ├── articles  → stub
        ├── apps      → stub
        ├── contacts  → stub
        ├── analytics → stub
        ├── settings  → Settings shell (отдельный вложенный RouterView)
        └── support   → live stdout `llama-server`
```

Правый сайдбар страницы чатов (`components/right_panel/`) — два настоящих таба:
* **Ламма** (`llama_control.rs`) — дропдаун моделей, сводка по выбранному
  пресету (filename + ctx + кол-во активных параметров), кнопки
  «Запустить» / «Остановить» / «Открыть консоль» и статус-пилюля,
  реактивная на `AppCtx.llama.status_signal`.
* **Детали** (`customer_details.rs`) — прежний customer-data блок.

### Глобальный контекст (`src/context.rs`)

`AppCtx` пробрасывается через `provide_context(..)` один раз в `run_desktop`.
Любой компонент читает его через `use_context::<AppCtx>()`.

Поля:

| Сигнал | Назначение |
| --- | --- |
| `theme_key: RwSignal<String>` | Ключ темы (id). Персистится в конфиг |
| `theme_mss: RwSignal<String>` | MSS-блок активной темы, подключён через `AppBuilder::with_dynamic_theme` |
| `router: Arc<Mutex<Router>>` | Верхний роутер (7 маршрутов) |
| `current_route: RwSignal<String>` | Ключ активного верхнего маршрута — дубликат `router.current()` для реактивной подписки |
| `settings_router: Arc<Mutex<Router>>` | Подроутер страницы настроек (`general`/`themes`/`skills`/`models`) |
| `selected_settings_tab: RwSignal<String>` | Ключ активной вкладки настроек |
| `selected_skill: RwSignal<Option<String>>` | Имя выбранного скила (`None` = приглашение-заглушка) |
| `skill_buffer: RwSignal<String>` | Текст, отображаемый в `MultilineTextEdit` для текущего скила |
| `general: GeneralCtx { .. }` | Сигналы раздела «Общие» — персистятся |
| `models: RwSignal<Vec<ModelConfig>>` | Пресеты llama.cpp-моделей — персистятся |
| `selected_model: RwSignal<Option<String>>` | Имя выбранного пресета |
| `right_panel_tab: RwSignal<usize>` | Активный таб правого сайдбара на странице «Чаты» |
| `llama: Arc<LlamaProcess>` | Контроль над процессом `llama-server` + реактивные сигналы логов и статуса |

### Persistent-конфиг (`src/config.rs`)

Конфиг хранится в `$HOME/.config/synthos/config.json` и содержит: активную тему, раздел «Общие», пресеты моделей.

* `AppConfig::load()` читает файл; при отсутствии — создаёт дефолт. Ошибки парсинга логируются (`eprintln!`), приложение не паникует.
* `AppConfig::save()` пишет `to_string_pretty`.
* Автосохранение: `install_config_autosave()` в `lib.rs` создаёт один `create_effect`, который подписывается на все persistent-сигналы и вызывает `save()` на каждое изменение.

### Настройки (`src/pages/settings/`)

```
Row [settings-sidebar | RouterView(content) | settings-right]
```

* `sidebar.rs` — три вкладки с анимацией выделения.
* `general.rs` — форма с `Toggle`, `TextField`, `Dropdown`.
* `themes.rs` — карточки тем с swatches и кнопкой «Применить».
* `theme_data.rs` — 5 светлых + 5 тёмных тем; каждая вырабатывает `:root { --var: ... }` поверх базовых токенов из `styles/base/variables.mss`.
* `skills.rs` — редактор markdown-инструкций (`MultilineTextEdit::show_line_numbers(true)`).
* `skills_panel.rs` — правая колонка со списком скилов на вкладке «Скилы» (`ListView` c `SelectionMode::Single`).
* `right_hint.rs` — подсказка-заглушка в правой колонке на вкладках «Общие» и «Темы».
* `models/` — раздел «Модели»:
  * `mod.rs` — редактор пресета: имя, путь к `.gguf` и к `mmproj`, активные чипы с контролами (SpinBox/Toggle/TextField/Dropdown), divider и сетка доступных чипов, сгруппированных по категориям. ctx-size — всегда первый и без крестика.
  * `models_panel.rs` — правая колонка со списком моделей и кнопкой «+».
  * `llama_params.rs` — полный каталог параметров `llama-server` (~150 ключей, метаданные для Tooltip).

Полный справочник параметров llama-server: [`docs/llama-server-help.md`](docs/llama-server-help.md).

### Запуск llama-server (`src/llama/`)

`LlamaProcess::start(general, model)` запускает дочерний процесс через
`std::process::Command`:

1. Бинарь — `GeneralConfig.server_path` (пусто → просто `llama-server`
   из `$PATH`).
2. Пути — `--model <model_path>`, `--mmproj <mmproj_path>` (если задан).
3. `--ctx-size <model.ctx_size>`.
4. Каждый активный параметр → CLI-ключ из `llama_params::LLAMA_PARAMS`
   с сериализованным значением (Int/Float/Bool/Text/Enum).
5. `--host <general.server_host>` + `--port <general.server_port>`.

Stderr и stdout читаются в двух фоновых потоках построчно. Каждая строка
приходит в UI через `syngui::async_runtime::run_on_main_thread`, которое
безопасно обновляет `RwSignal<Vec<String>>` из главного UI-потока.
Буфер ограничен 5000 строками (head-drain). Статус
(`Stopped/Starting/Running/Error`) — тоже реактивный сигнал; переход
`Starting → Running` срабатывает по строке `"server is listening"`.

Глобальные настройки `server_path`, `server_host`, `server_port` правятся
в разделе **Настройки → Общие → Llama server** и сохраняются вместе с
остальным конфигом (`~/.config/synthos/config.json`).

Страница **Поддержка** (nav-rail footer) — живая консоль: `ListView`
со строками лога + статус-пилюля + кнопка «Очистить».

### Темизация

Базовые переменные определены в `styles/base/variables.mss`. При смене темы сигнал `theme_mss` (см. `theme_data::SynthosTheme::to_mss()`) перезаписывает их в рантайме через механизм `AppBuilder::with_dynamic_theme`. Все компонентные MSS-классы ссылаются исключительно на `var(--name)` — никаких хардкод-цветов в MSS.

### Логирование

В `run_desktop()` инициализируются:

```rust
tracing_log::LogTracer::init();        // log → tracing мост
tracing_subscriber::fmt().try_init();  // форматтер
```

Благодаря этому в консоли видны как собственные `tracing::info!` из `synthos`, так и `log::warn!` из `syngui`, `wgpu`, `sctk_adwaita`, `winit` и других зависимостей.

## Стили

Все `.mss` файлы лежат в `styles/` и подключаются в `src/styles.rs` через `include_str!`:

```
styles/
├── base/
│   ├── reset.mss
│   └── variables.mss
├── layout/
│   ├── shell.mss
│   └── settings.mss
└── components/
    ├── chat_header.mss
    ├── chats_column.mss
    ├── …
    ├── right_panel.mss       # контейнер правого сайдбара + customer rows
    ├── llama_control.mss     # таб «Ламма» + общие классы `.llama-pill*`
    ├── support_page.mss      # страница поддержки (live log)
    ├── settings_sidebar.mss
    ├── settings_form.mss
    ├── settings_themes.mss
    ├── settings_skills.mss
    ├── settings_models.mss
    └── stub_page.mss
```

Правила:

* Никаких inline-стилей с цветом — через `.class()` и MSS-переменные.
* Все анимации через `transition` в MSS с токенами `--duration-*` и `--ease-standard`.
* Material Icons — через `icons::MI_*` (codepoints).
