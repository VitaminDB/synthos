# Системное оформление — тема, кнопки окна и стекло

Приложение умеет подхватывать оформление рабочего стола: светлую/тёмную схему,
акцентный цвет, вид кнопок титлбара и размытие фона за окном. Всё включается
в **Настройки → Темы → Системное оформление** и переживает перезапуск.

Данные о системе приходят из syngui (`syngui::appearance`,
`syngui::window::backdrop`) — подробности источников в
`syngui/docs/14-system-appearance.md`. Здесь — как это устроено на стороне
synthos.

## Архитектура

```
AppCtx
 └─ appearance: AppearanceCtx
      ├─ system:                 RwSignal<SystemAppearance>  // наполняет syngui
      ├─ follow_system:          RwSignal<bool>              // светлая/тёмная по схеме DE
      ├─ theme_light:            RwSignal<String>            // тема для светлой схемы
      ├─ theme_dark:             RwSignal<String>            // тема для тёмной схемы
      ├─ use_system_accent:      RwSignal<bool>
      ├─ system_window_controls: RwSignal<bool>
      ├─ window_blur:            RwSignal<bool>
      ├─ window_opacity:         RwSignal<f32>               // 1.0 — сплошной фон
      └─ backdrop:               RwSignal<BackdropConfig>    // уходит в AppBuilder
```

Persist — `AppConfig`: `follow_system_theme`, `theme_light`, `theme_dark`,
`use_system_accent`, `system_window_controls`, `window_blur`, `window_opacity`.
Ручной выбор темы остаётся в `theme` и не теряется, пока включён системный
режим.

## Сборка палитры

`build_context` держит один эффект, который пересобирает `theme_mss` из трёх
блоков `:root` — переменные последнего блока перекрывают предыдущие, потому что
`StyleSheet` хранит их в `HashMap`:

1. **База** — `SynthosTheme::to_mss()` активной темы. В системном режиме тема
   выбирается `active_theme()`: светлая схема → `theme_light`, тёмная →
   `theme_dark` (`theme_data::find_or_default`).
2. **Акцент** — `theme_data::accent_override_mss(accent, is_dark)`, когда включён
   системный акцент и DE его сообщает. Переопределяет `--primary`,
   `--primary-hover`, `--primary-soft`, `--surface-selected`, `--on-primary`:
   портал отдаёт один цвет, производные считаются от него.
3. **Прозрачность** — `theme_data::surface_alpha_mss(&theme, opacity)` при
   `window_opacity < 1`. Переводит фоновые токены (`--bg-window`, `--bg-shell`,
   `--bg-rail`, `--bg-chats`, `--bg-chat`, `--bg-panel`, `--bg-search`) в
   `rgba(...)`. Текст, границы и акценты остаются плотными — иначе интерфейс
   не читается поверх чужих окон.

Схема и акцент меняются в системе на лету: syngui обновляет `system`, эффект
пересобирает MSS, приложение перекрашивается без перезапуска.

## Кнопки титлбара

`components/titlebar.rs` рисует либо встроенные кнопки, либо системные
(`SystemWindowControls`), в зависимости от `system_window_controls`. В
системном режиме берутся раскладка (`ButtonsOnLeft`/`ButtonsOnRight` из
`kwinrc`), размеры и выравнивание заголовка из темы декораций; на Aurorae-темах
кнопки растеризуются из SVG самой темы, включая hover/pressed/inactive.
Развёрнутость и фокус приходят из `window_state`: в развёрнутом окне кнопка
«развернуть» показывает иконку «восстановить», в неактивном окне кнопки
приглушены.

Снимок декораций кэшируется на процесс: файл читается один раз, а титлбар
пересобирается на каждый тик реактивного блока.

## Стекло

Окно всегда прозрачное (`.transparent(true)` + нулевой clear-color), поэтому
для «стекла» достаточно двух вещей: полупрозрачных фонов (пункт 3 выше) и
просьбы к композитору размыть то, что за окном — `window_blur` →
`BackdropConfig::frosted()` → `AppBuilder::with_backdrop`.

Область эффекта повторяет `.shell`: в восстановленном окне вокруг него 30px
прозрачного воздуха (`.window-backdrop { padding: 30px }`) и скругление
`--radius-shell`, в развёрнутом и то и другое обнуляется — отсюда зависимость
`backdrop_config(blur, maximized)`. Без этого композитор размывает всю
поверхность и вокруг окна висит мутный прямоугольник.

На KWin 6.7+ используется `ext-background-effect-v1`, на более старых —
`org_kde_kwin_blur`/`contrast`, на X11 — `_KDE_NET_WM_BLUR_BEHIND_REGION`. Если
композитор не умеет ничего из этого, окно просто остаётся полупрозрачным.
