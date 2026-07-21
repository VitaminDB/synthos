# MarkdownView link click + syngui `links` feature (2026-05-11)

## Что добавлено в syngui

1. **Feature `links`** (syngui/Cargo.toml): тонкая обёртка над крейтом
   `webbrowser`. Активирует `pub fn syngui::open_url(&str) -> Result<(), String>`.
   Без `links` функция возвращает Err.
2. **Feature `markdown-links = ["markdown", "links"]`** — опт-ин для
   приложений, которые хотят, чтобы `MarkdownView` сам открывал ссылки
   в системном браузере без явного callback'а.
3. **`terminal` feature** теперь подтягивает `links` транзитивно (раньше
   тянула `webbrowser` напрямую). Поведение терминала не изменилось:
   OSC 8 hyperlinks по-прежнему открываются на click.
4. **`syngui/src/external_url.rs`** — реализация `open_url`. Под cfg
   `links` вызывает `webbrowser::open`, без — возвращает Err.
5. **Terminal** теперь использует `crate::open_url(&uri)` вместо прямого
   `webbrowser::open(&uri)` — единая точка для всех виджетов.

## Что добавлено в MarkdownView

- `MarkdownView::on_link_click(|url| ...)` — кастомный обработчик клика
  по `[label](url)`. Если задан, дефолтный fallback не срабатывает.
- Если callback не задан, `MarkdownView` сам пытается открыть URL через
  `syngui::open_url(...)`. Это работает, если в сборке включена feature
  `links` (обычно через `terminal` или `markdown-links`).
- В `selection_map.rs` `SelectableRun` появилось поле `link: Option<String>`.
  В `renderer.rs` `InlineStyle.link_url: Option<String>` каскадирует
  через вложенные стили (Bold внутри ссылки → текст всё ещё link). Поле
  пробрасывается в `FlatSpan.link` → `SelectableRun.link` через
  `MdRenderer::emit_selectable`.
- Hit-test в `widget.rs::link_url_at(pos)` — линейный обход runs.
  Используется для (а) `CursorIcon::Pointer` на hover, (б) захвата
  `pending_link: Option<(String, Point)>` на MouseDown.
- Drift-detection в `MouseMove`: если drag ушёл больше чем на
  `LINK_DRAG_THRESHOLD_PX = 3.0`, pending отменяется и стартует обычная
  text-selection с anchor=исходная позиция MouseDown.
- На `MouseUp`: если pending не сброшен и upMutation в пределах порога,
  вызывается `dispatch_link_click(url)` — кастомный callback или
  `crate::open_url`. Логирование при ошибке через `log::warn!`.

## synthos

`message_bubble.rs::bubble_markdown` НЕ требует правки — synthos
подтягивает `terminal` feature, поэтому транзитивно включается `links`,
и MarkdownView открывает HTTP-ссылки в дефолтном браузере «из коробки».
Если в будущем понадобится перехватить клик (например, для analytics или
чтобы внутренние `synthos://`-схемы открывались внутри приложения), —
добавляется `MarkdownView::on_link_click(|url| { ... })`.

## Что НЕ сделано (на будущее)

- Контекстное меню на ссылке («Открыть», «Копировать URL») — сейчас
  PopupMenu MarkdownView'а универсальный (Copy/Select All/Copy All) и не
  показывает URL-зависимых пунктов.
- Подсветка hover'а на ссылке (изменение `link_color` или underline-style
  при наведении) — пока статичная.
