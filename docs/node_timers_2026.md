# Таймеры node-editor'а (2026-08-14)

Замер времени работы: секундомер у каждой ноды с явным запуском и один общий на весь прогон графа.

---

## 1. Модель

`pages::node_editor::timing::Stopwatch` — `Copy`-структура из трёх сигналов:

| поле | смысл |
|---|---|
| `started: RwSignal<Option<Instant>>` | момент старта; `None` — секундомер стоит |
| `live_ms: RwSignal<u64>` | «живое» показание идущего отсчёта |
| `last_ms: RwSignal<Option<u64>>` | итог последнего завершённого измерения |

API: `start` / `finish` / `reset` / `tick` / `is_running[_untracked]` / `display_ms`.

`display_ms` — то, что показывает бейдж: идущее показание, иначе прошлый итог, иначе `None` (замеров не было — бейджа нет).

Секундомеры не персистятся: измерение принадлежит сессии, а не графу, поэтому в `workspace.json` / шаблоны не попадает и в `persist::subscribe_node_signals` не подписывается (иначе автосейв срабатывал бы 10 раз в секунду).

## 2. Кто заводит отсчёт

**Ноды** — эффект `state::NodeEditorCtx::install_node_timers`, подписанный на `busy_signal` каждой ноды графа. Флип `false → true` вызывает `start()`, обратный — `finish()`. Наблюдение именно за busy (а не за `on_run`-хуком) даёт два бонуса: меряются и глобальный Run, и per-node Play; эффект живёт вместе с вкладкой, а не с её отрисованным canvas'ом, поэтому на неактивной вкладке замер не теряется.

**Прогон** — `run_controls`: `start()` на Run (вместе с `run_total` / `run_done`), `finish()` на Stop, на опустошение очереди и в legacy-ветке auto-Stop. Живёт в `EditorWorkspace` рядом с `run_state`, то есть общий на все вкладки — как и сам Run-pill.

Pause отсчёт не останавливает: cooperative-паузы у нод нет, работа продолжается — значит и время идёт.

## 3. Тик и отрисовка

Тик сделан на `Element::animate` (через `controls::ProgressTicker` / `node_progress_animator`), а не на фоновом потоке: `animate` вызывается движком на main-thread, поэтому секундомер читает свои сигналы напрямую (`RwSignal::get*` вне main-thread паникует). Возврат `true` из `tick` заставляет syngui запросить следующий кадр — цикл держится сам, пока идёт отсчёт, и гаснет вместе с ним.

Показание обновляется раз в 100 мс (`TICK_INTERVAL`) — ровно один разряд десятых долей в `fmt_elapsed`. Накопитель `dt` живёт в тик-элементе, поэтому тикер стоит рядом с текстом, а не внутри его `Reactive` — иначе пересобирался бы на каждом обновлении и терял накопленное.

Формат `fmt_elapsed`: `12.4 с` до минуты, `1:05.3` до часа, дальше `1:02:03`.

## 4. UI

**Бейдж ноды** — в шапке карточки слева от «×», только у нод с `busy_signal` (у реактивных Number/Add мерить нечего). Пока нода работает — зелёный с мягкой пульсацией, после — приглушённый итог.

**Бейдж прогона** — в Run-pill за вертикальным разделителем: время + счётчик `3/7` (счётчик только пока прогон идёт). Ширина колонки времени фиксирована (`min-width`), иначе pill дёргался бы при переходе `9.9 с → 10.0 с` — он центрируется в overlay-строке балансировкой краёв.

## 5. Файлы

- `src/pages/node_editor/timing.rs` — `Stopwatch`, `fmt_elapsed`, тикер, оба бейджа.
- `src/pages/node_editor/state.rs` — `install_node_timers`.
- `src/pages/node_editor/run_controls.rs` — старт/финиш прогона, `run_done` / `run_total`, бейдж в pill.
- `src/pages/node_editor/node_view.rs` — бейдж в шапке карточки.
- `src/pages/node_editor/tabs.rs` — `run_timer` / `run_done` / `run_total` в `EditorWorkspace`.
- `src/pages/node_editor/types.rs` — поле `NodeInstance.timing`.
- `styles/components/node_editor_timers.mss` — стили обоих бейджей.

## 6. Тесты

- `timing::tests::fmt_seconds_with_tenths`, `fmt_minutes_and_hours` — форматирование.
- `timing::tests::start_tick_finish_cycle` — старт → тик → финиш, повторный `finish` не затирает итог, `reset`.
- `timing::tests::node_stopwatch_follows_busy_signal` — связка «busy ноды → её секундомер» на реальном эффекте `NodeEditorCtx`.
- `timing::tests::ticker_stops_with_the_stopwatch` — тикер не держит кадры у стоящего секундомера.
- `timing::tests::badge_takes_space_only_after_first_run` — `#[ignore]`, требует `--features testing`: layout-проход `TestHarness`, проверяет, что бейдж не схлопывается в 0×0 и отсутствует до первого прогона. Отдельным запуском, потому что `TestHarness::new` занимает глобальный `MAIN_THREAD_ID` syngui и ломает чтение сигналов в остальных тест-потоках: `cargo test --features testing badge_takes_space -- --ignored`.
- `styles::tests::stylesheet_parses` — весь MSS парсится (сломанный селектор иначе всплыл бы только в рантайме).
