//! Тесты инструмента `notes`: действия целиком через `dispatch` на
//! собственном `NotesCtx`, без main-thread.

use super::*;
use crate::config::AppConfig;
use crate::pages::notes::project;

/// Контекст заметок над временным проектом. Каталоги строк — русские:
/// названия колонок шаблона и корня журнала берутся через `tr!`.
fn ctx() -> NotesCtx {
    syngui::i18n::register_catalogs(&crate::i18n::CATALOGS);
    syngui::i18n::set_language(syngui::i18n::Lang::new("ru"));
    let dir = std::env::temp_dir().join(format!("synthos-notes-tool-{}", project::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = AppConfig {
        notes_project_path: dir.join("p.syn").display().to_string(),
        notes_vault_path: dir.join("no-vault").display().to_string(),
        ..AppConfig::default()
    };
    NotesCtx::new_or_restore(&cfg)
}

fn call(ctx: NotesCtx, action: &str, args: serde_json::Value) -> String {
    dispatch(ctx, action, &args).unwrap_or_else(|e| panic!("{action} {args}: {e}"))
}

fn page_id(out: &str) -> String {
    let line = out.lines().find(|l| l.starts_with("page: ")).expect("page line");
    line[6..18].to_string()
}

/// Живой чат MyLife (08.09.2026): модель дважды собирала `add_card` без
/// `op` и с текстом карточки в `card`. Ошибка без `op` называет сигнатуры
/// операций, а `card` у `add_card` принимается как заголовок.
#[test]
fn kanban_add_card_tolerates_card_as_title_and_explains_ops() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Доска"})));
    call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "columns": ["К выполнению", "Готово"]}));

    let err = dispatch(ctx, "kanban", &serde_json::json!({"page": &page, "column": "К выполнению", "card": "Порядок страниц"})).unwrap_err();
    assert!(err.starts_with("missing \"op\". kanban ops"), "{err}");
    assert!(err.contains("add_card {title = the new card's text, column, md, priority, tags, due, duration, start, end, repeat, before}"), "{err}");

    let out = call(
        ctx,
        "kanban",
        serde_json::json!({"op": "add_card", "page": &page, "column": "К выполнению", "card": "Порядок страниц", "priority": "medium", "tags": ["Баг", "Заметки"]}),
    );
    assert!(out.contains("added card to \"К выполнению\"") && out.contains("\"Порядок страниц\"") && out.contains("tags: Баг, Заметки"), "{out}");
    // Явный title важнее: "card" тогда — просто лишнее поле.
    let out = call(ctx, "kanban", serde_json::json!({"op": "add_card", "page": &page, "title": "Заголовок", "card": "Не заголовок"}));
    assert!(out.contains("\"Заголовок\"") && !out.contains("\"Не заголовок\""), "{out}");

    let err = dispatch(ctx, "kanban", &serde_json::json!({"op": "add_card", "page": &page})).unwrap_err();
    assert!(err.starts_with("missing \"title\"") && err.contains("existing card"), "{err}");
    let err = dispatch(ctx, "kanban", &serde_json::json!({"op": "add_kard", "page": &page, "title": "x"})).unwrap_err();
    assert!(err.starts_with("unknown kanban op \"add_kard\". kanban ops") && err.contains("update_card {card, title"), "{err}");
}

/// Повторный `create` с тем же названием обязан сказать, что страница
/// уже есть: молчаливое переименование в «… 2» выглядит как успех, и
/// агент, не увидевший свою же прошлую страницу, зацикливается.
#[test]
fn create_reports_a_title_clash() {
    let ctx = ctx();
    let first = call(ctx, "create", serde_json::json!({"title": "Туду — канбан"}));
    assert!(!first.contains("already exists"), "первой странице ругаться не на что: {first}");
    let second = call(ctx, "create", serde_json::json!({"title": "Туду — канбан"}));
    assert!(second.contains("already exists"), "{second}");
    assert!(second.contains(&page_id(&first)), "нужен id существующей страницы: {second}");
    assert!(second.contains("Туду — канбан 2"), "{second}");
    // Одноимённые страницы у разных родителей законны — там не ругаемся.
    let parent = page_id(&call(ctx, "create", serde_json::json!({"title": "Работа"})));
    let child = call(
        ctx,
        "create",
        serde_json::json!({"title": "Туду — канбан", "parent": parent}),
    );
    assert!(!child.contains("already exists"), "{child}");
}

/// Агент видит страницу без служебного хвоста, а после его правки
/// нетронутые блоки сохраняют координаты холста.
#[test]
fn plain_markdown_hides_geometry_and_write_keeps_it() {
    let md = "# Заголовок\n\nАбзац\n\n- пункт\n\n```doc-layout\n0 40 300 200\n1 400 60 200\n```\n";
    let plain = plain_markdown(md);
    assert!(!plain.contains("doc-layout"), "{plain}");
    assert!(plain.contains("# Заголовок"));

    // Абзац переписан, заголовок и пункт остались — координаты у них на месте.
    let edited = plain.replace("Абзац", "Новый абзац");
    let merged = with_sidecar(md, &edited);
    assert!(merged.contains("Новый абзац"), "{merged}");
    assert!(merged.contains("0 {w=200 x=40 y=300}"), "заголовок потерял координаты: {merged}");
    assert!(!merged.contains("x=400"), "переписанный абзац не должен наследовать координаты: {merged}");
    // Вставка блока перед заголовком не сбивает его координаты.
    let shifted = format!("Преамбула\n\n{plain}");
    let merged = with_sidecar(md, &shifted);
    assert!(merged.contains("1 {w=200 x=40 y=300}"), "{merged}");
    assert!(merged.contains("2 {w=200 x=400 y=60}"), "{merged}");
}

/// Стили абзаца живут в том же хвосте, что и координаты: агент их не
/// видит, а нетронутый абзац их не теряет.
#[test]
fn plain_markdown_strips_style_sidecar_and_write_keeps_it() {
    let md = "Первый\n\nВторой\n\n```doc-layout\n1 {bg=#243149 color=#FF8800 x=40 y=60}\n```\n";
    let plain = plain_markdown(md);
    assert!(!plain.contains("doc-layout") && !plain.contains("bg="), "{plain}");
    let merged = with_sidecar(md, &format!("Нулевой\n\n{plain}"));
    assert!(merged.contains("2 {bg=#243149 color=#FF8800 x=40 y=60}"), "{merged}");
    // Правка через find/replace идёт по блоку — атрибуты остаются.
    let mut model = parse_document(md);
    let n = replace_in_blocks(&mut model, "Второй", "Второй и главный", false).unwrap().n;
    assert_eq!(n, 1);
    let out = serialize_document(&model);
    assert!(out.contains("Второй и главный") && out.contains("1 {bg=#243149 color=#FF8800 x=40 y=60}"), "{out}");
    // Фрагмент через два блока тоже заменяется; стиль второго остаётся при нём.
    let n = replace_in_blocks(&mut model, "Первый\n\nВторой и главный", "Первый\n\nВторой, итог", false).unwrap().n;
    assert_eq!(n, 1);
    let out = serialize_document(&model);
    assert!(out.contains("Второй, итог") && out.contains("1 {bg=#243149 color=#FF8800 x=40 y=60}"), "{out}");
    assert!(replace_in_blocks(&mut model, "нет такого", "x", false).unwrap_err().contains("not found"));
}

/// find/replace сверяется с тем же текстом, что агент видит в read: строки
/// списка идут без пустой строки, фрагмент может задеть несколько блоков,
/// пробелы и знаки разметки могут расходиться со скопированным. 14.09.2026
/// три строки «Текущая роль» не находились, модель ушла в set_markdown.
#[test]
fn find_replace_matches_what_read_shows() {
    let md = "# Резюме\n\nТекущая роль:\n\n- Компания: Альфа\n- Должность: **инженер**\n- Стаж: 3 года\n\nИтог\n\n```doc-layout\n2 40 120 300\n3 40 150 300\n4 40 180 300\n```\n";
    let mut model = parse_document(md);
    let (text, _) = page_spans(&model);
    assert_eq!(format!("{text}\n"), plain_markdown(md), "сверка идёт по тексту из read");

    // Три пункта списка разом — как их копирует модель.
    let n = replace_in_blocks(
        &mut model,
        "- Компания: Альфа\n- Должность: **инженер**\n- Стаж: 3 года",
        "- Компания: Бета\n- Должность: **ведущий инженер**\n- Стаж: 5 лет",
        false,
    )
    .unwrap()
    .n;
    assert_eq!(n, 1);
    let out = serialize_document(&model);
    assert!(out.contains("- Компания: Бета\n- Должность: **ведущий инженер**\n- Стаж: 5 лет"), "{out}");
    assert!(out.contains("2 {w=300 x=40 y=120}") && out.contains("4 {w=300 x=40 y=180}"), "пункты остались на своих местах холста: {out}");

    // Пробелы и перевод строки расходятся.
    replace_in_blocks(&mut model, "Текущая   роль:\n", "Прошлая роль:", false).unwrap();
    // Скопирован отрисованный текст, без `**`: закрывающие не повисают.
    replace_in_blocks(&mut model, "Должность: ведущий инженер", "Должность: техлид", false).unwrap();
    let out = serialize_document(&model);
    assert!(out.contains("Прошлая роль:") && out.contains("- Должность: техлид\n"), "{out}");
    assert!(!out.contains("**"), "{out}");

    // Значение другое, слов мало — не догадка: ошибка с ближайшим текстом дословно.
    let err = replace_in_blocks(&mut model, "Прошлая роль:\n\n- Компания: Гамма", "x", false).unwrap_err();
    assert!(err.contains("closest text is in block #1") && err.contains("Прошлая роль:\n\n- Компания: Бета"), "{err}");
}

/// Модель пересказала строку по памяти (14.09.2026, MyLife: «трекеры,
/// ритуалы, правила» вместо «трекеры, системы, ритуалы» — дважды «not
/// found», пока не перечитала страницу): единственная уверенно похожая
/// строка заменяется и называется в результате; сомнительное — ошибка.
#[test]
fn find_replace_takes_the_one_close_line_and_names_it() {
    let md = "## Разделы жизни\n\n- [[Здоровье и энергия]] — сон, спорт, питание, привычки\n- [[Отношения и семья]] — близкие, друзья, ритуалы\n- [[Привычки и дисциплина]] — трекеры, системы, ритуалы\n- [[Идеи и заметки]] — бред, инсайты, список идей\n";
    let mut model = parse_document(md);
    let r = replace_in_blocks(&mut model, "- [[Привычки и дисциплина]] — трекеры, ритуалы, правила\n", "", false).unwrap();
    assert_eq!(r.n, 1);
    assert_eq!(r.approx.as_deref(), Some("- [[Привычки и дисциплина]] — трекеры, системы, ритуалы"));
    let out = serialize_document(&model);
    assert!(!out.contains("Привычки и дисциплина"), "{out}");
    assert!(out.contains("- [[Отношения и семья]] — близкие, друзья, ритуалы\n- [[Идеи и заметки]]"), "соседи на месте: {out}");

    // Мало слов — ничего не трогаем, показываем ближайшее.
    let err = replace_in_blocks(&mut model, "- [[Идеи]] — мысли, инсайты", "x", false).unwrap_err();
    assert!(err.contains("- [[Идеи и заметки]] — бред, инсайты, список идей"), "{err}");
    // Две строки одинаково похожи — тоже не догадка.
    let mut twins = parse_document("- план на неделю спорт сон еда\n- план на неделю спорт сон вода\n");
    let err = replace_in_blocks(&mut twins, "- план на неделю спорт сон чай", "x", false).unwrap_err();
    assert!(err.contains("closest text"), "{err}");
    // Фрагмент из середины строки (длина не та) — тоже нет.
    let mut line = parse_document("Сегодня купить молоко хлеб сыр масло яйца и забрать посылку на почте\n");
    assert!(replace_in_blocks(&mut line, "купить молоко хлеб сыр масло чай", "x", false).is_err());
}

/// Блоки: список с геометрией, вставка в позицию, атрибуты, закрепление,
/// перенос и удаление — всё по индексам, с сохранением остального.
#[test]
fn blocks_ops_through_the_tool() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Холст", "content": "# Схема\n\nАбзац\n"})));
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("#0 heading1 \"Схема\" · flow h=~"), "{listed}");
    assert!(listed.contains("#1 paragraph \"Абзац\""), "{listed}");

    // Вставка с геометрией после заголовка.
    let out = call(ctx, "blocks", serde_json::json!({"op": "insert", "page": &page, "md": "Заметка", "after": 0, "x": 100, "y": 200, "w": 240}));
    assert!(out.contains("inserted #1 at block #1") && out.contains("x=100 y=200 w=240"), "{out}");
    assert!(ctx.page_markdown(&page).contains("1 {w=240 x=100 y=200}"), "{}", ctx.page_markdown(&page));

    // Атрибуты: стиль ставится и снимается; чужой ключ — ошибка.
    let out = call(ctx, "blocks", serde_json::json!({"op": "set_attrs", "page": &page, "block": "find:Заметка", "attrs": {"bg": "#243149", "align": "center", "size": 18}}));
    assert!(out.contains("align=center") && out.contains("bg=#243149"), "{out}");
    assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "set_attrs", "page": &page, "block": 1, "attrs": {"font": "x"}})).is_err());
    call(ctx, "blocks", serde_json::json!({"op": "set_attrs", "page": &page, "block": 1, "attrs": {"size": null}}));
    assert!(!ctx.page_markdown(&page).contains("size="));
    // Замена markdown блока сохраняет его координаты и стиль.
    call(ctx, "blocks", serde_json::json!({"op": "set_markdown", "page": &page, "block": 1, "md": "Заметка подробнее"}));
    let md = ctx.page_markdown(&page);
    assert!(md.contains("Заметка подробнее") && md.contains("align=center") && md.contains("x=100"), "{md}");

    // pin без координат — под нижним закреплённым; unpin — снова в потоке; move меняет порядок.
    let out = call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 2}));
    assert!(out.contains("pinned at x=100 y="), "{out}");
    call(ctx, "blocks", serde_json::json!({"op": "unpin", "page": &page, "block": 2}));
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("#2 paragraph \"Абзац\" · flow"), "{listed}");
    call(ctx, "blocks", serde_json::json!({"op": "move", "page": &page, "block": 2, "index": 0}));
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.starts_with("#0 paragraph \"Абзац\""), "{listed}");
    let out = call(ctx, "blocks", serde_json::json!({"op": "delete", "page": &page, "block": 0}));
    assert!(out.contains("2 blocks left"), "{out}");
    let read = call(ctx, "read", serde_json::json!({"page": &page, "blocks": true}));
    assert!(read.contains("--- Blocks") && read.contains("grid: dots step 20 · snap: on step 5"), "{read}");
    assert!(ctx.page(&page).unwrap().handle.history_state().get_untracked().0);
}

/// Живой случай (08.09.2026, страница «Долги и кредиты»): модель написала
/// `> [!toggle] Полная таблица графика` и таблицу отдельным блоком — toggle
/// сворачивал пустоту. `op=nest` убирает таблицу внутрь, `op=unnest`
/// возвращает; в `op=list` вложенное видно строкой `#0.0`.
#[test]
fn nest_puts_a_table_inside_a_toggle() {
    let ctx = ctx();
    let md = "> [!toggle] Полная таблица графика\n\n| Платёж | Дата |\n| --- | --- |\n| 1 | 2025-06 |\n";
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Долги", "content": md})));
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("#0 toggle \"Полная таблица графика\"") && listed.contains("#1 table"), "{listed}");

    // Без "into" блок уходит в соседа сверху — как раз этот случай.
    let out = call(ctx, "blocks", serde_json::json!({"op": "nest", "page": &page, "block": 1}));
    assert!(out.contains("nested #1 into #0 as #0.0") && out.contains("  #0.0 table"), "{out}");
    let stored = ctx.page_markdown(&page);
    assert!(stored.contains("> [!toggle]{open} Полная таблица графика") && stored.contains("> | Платёж | Дата |"), "{stored}");

    // Вложенный блок операциям верхнего уровня недоступен — ошибка учит,
    // как его достать.
    let err = dispatch(ctx, "blocks", &serde_json::json!({"op": "set_markdown", "page": &page, "block": "0.0", "md": "x"})).unwrap_err();
    assert!(err.contains("nested inside block #0") && err.contains("op=unnest block=0 child=0"), "{err}");

    // Таблица не контейнер; сама в себя тоже не вкладывается.
    call(ctx, "blocks", serde_json::json!({"op": "insert", "page": &page, "md": "| A |\n| --- |\n| 1 |"}));
    let err = dispatch(ctx, "blocks", &serde_json::json!({"op": "nest", "page": &page, "block": 1, "into": 1})).unwrap_err();
    assert!(err.contains("cannot be nested into itself"), "{err}");
    let err = dispatch(ctx, "blocks", &serde_json::json!({"op": "nest", "page": &page, "block": 0, "into": 1})).unwrap_err();
    assert!(err.contains("cannot hold other blocks"), "{err}");

    let out = call(ctx, "blocks", serde_json::json!({"op": "unnest", "page": &page, "block": "0.0"}));
    assert!(out.contains("took 1 block(s) out of #0") && out.contains("#1 table"), "{out}");
    assert!(!ctx.page_markdown(&page).contains("> |"), "{}", ctx.page_markdown(&page));
}

/// Переход поток → холст сохраняет расположение: блоки получают
/// координаты колонкой в порядке документа, подсказка про свободную
/// раскладку исчезает; `create layout=free` с контентом рождает страницу
/// уже разложенной (07.09.2026: страница «Тестовая» без этого легла кашей).
#[test]
fn layout_free_pins_blocks_in_a_column() {
    let ctx = ctx();
    // Новая страница — холст по умолчанию, но без закреплённых блоков
    // она документ: ни закрепления, ни подсказки.
    let out = call(ctx, "create", serde_json::json!({"title": "Переход", "content": "# Заголовок\n\nАбзац\n\n![[shape:rect]]\n"}));
    assert!(!out.contains("pinned") && !out.contains("!! free layout"), "{out}");
    let page = page_id(&out);
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("· flow") && !listed.contains("!! free layout"), "{listed}");
    // layout=free просит разложить незакреплённые блоки колонкой.
    let out = call(ctx, "update", serde_json::json!({"page": &page, "layout": "free"}));
    assert!(out.contains("3 blocks pinned in a column"), "{out}");
    assert!(!out.contains("!! free layout"), "{out}");
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("#0 heading1 \"Заголовок\" · x=40 y=40 w=520"), "{listed}");
    assert!(!listed.contains("· flow"), "{listed}");
    // Каждый следующий блок ниже предыдущего; фигура получила свою высоту.
    let model = load_model(ctx, &page);
    let rects: Vec<_> = model.blocks.iter().map(|b| block_rect(b).unwrap()).collect();
    assert!(rects[1].1 >= rects[0].1 + rects[0].3 + 24.0, "{rects:?}");
    assert!(rects[2].1 >= rects[1].1 + rects[1].3 + 24.0, "{rects:?}");
    assert!(free::height_of(&model.blocks[2].attrs).is_some(), "{}", ctx.page_markdown(&page));
    // Повторный вызов ничего не перекладывает — всё уже закреплено.
    let out = call(ctx, "update", serde_json::json!({"page": &page, "layout": "free"}));
    assert!(!out.contains("pinned in a column"), "{out}");

    // create layout=free + content — сразу колонкой.
    let out = call(ctx, "create", serde_json::json!({"title": "Сразу холст", "layout": "free", "content": "Один\n\nДва\n"}));
    assert!(out.contains("2 blocks pinned in a column") && !out.contains("!! free layout"), "{out}");

    // Смесь на странице по умолчанию: фигура закреплена, текст в потоке —
    // подсказка появляется и пропадает после arrange.
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Смесь", "content": "Текст\n"})));
    let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "rect", "x": 40, "y": 40, "w": 200, "h": 100}));
    assert!(out.contains("!! free layout: 1 of 2 blocks have no x/y (#0)") && out.contains("ends at y=140"), "{out}");
    let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page}));
    assert!(out.contains("arranged 1 blocks in a column at x=40") && out.contains("y=164") && !out.contains("!! free layout"), "{out}");
}

/// `only=all` не уносит в колонку поставленный календарь (14.09.2026, MyLife:
/// календарь слева, колонка текста справа — после arrange календарь уехал в
/// хвост колонки, модель перебирала y по кругу).
#[test]
fn arrange_all_keeps_placed_objects_in_place() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Дни рождения", "content": "# Дни рождения\n\nТекст\n"})));
    call(ctx, "calendar", serde_json::json!({"op": "create", "page": &page, "view": "month", "x": 40, "y": 160, "w": 940, "h": 700}));
    let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page, "only": "all", "x": 1040, "y": 160, "w": 860}));
    assert!(out.contains("arranged 2 blocks") && out.contains("kept in place"), "{out}");
    assert!(out.contains("x=40 y=160 w=940 h=700"), "календарь на месте: {out}");
    let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page, "only": "everything", "x": 1040, "y": 160, "w": 860}));
    assert!(out.contains("arranged 3 blocks") && !out.contains("kept in place"), "{out}");
}

/// `blocks op=arrange`: одна команда кладёт неприкреплённые блоки под
/// закреплённые; `only=all` перекладывает всё; предупреждение о наложении
/// появляется у pin/shape и не мешает линиям.
#[test]
fn arrange_stacks_blocks_and_overlaps_are_reported() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Раскладка-2", "layout": "free", "content": "Один\n\nДва\n"})));
    // Дописанный контент без геометрии → подсказка; arrange её снимает.
    call(ctx, "update", serde_json::json!({"page": &page, "content": "Три\n", "mode": "append"}));
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("!! free layout: 1 of 3"), "{listed}");
    let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page}));
    assert!(out.contains("arranged 1 blocks in a column at x=40"), "{out}");
    assert!(!out.contains("!! free layout") && !out.contains("!! overlaps"), "{out}");
    // Новый блок встал под нижним закреплённым.
    let model = load_model(ctx, &page);
    let r1 = block_rect(&model.blocks[1]).unwrap();
    let r2 = block_rect(&model.blocks[2]).unwrap();
    assert!(r2.1 >= r1.1 + r1.3 + 24.0, "{r1:?} {r2:?}");
    // Повтор — нечего раскладывать; only=all перекладывает всё заново.
    let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page}));
    assert!(out.contains("nothing to arrange"), "{out}");
    let out = call(ctx, "blocks", serde_json::json!({"op": "arrange", "page": &page, "only": "all", "x": 100, "y": 100, "w": 300, "gap": 0}));
    assert!(out.contains("arranged 3 blocks in a column at x=100 (gap 0)") && out.contains("x=100 y=100 w=300"), "{out}");
    assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "arrange", "page": &page, "only": "some"})).is_err());
    assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "arrange", "page": &page, "gap": 900})).is_err());

    // Наложение: фигура поверх первого блока — предупреждение с его рамкой.
    let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "rect", "x": 100, "y": 100, "w": 200, "h": 100}));
    assert!(out.contains("!! overlaps #0 (x=100 y=100 w=300 h="), "{out}");
    // Линия поверх тех же блоков — не в счёт.
    let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "arrow", "x1": 100, "y1": 100, "x2": 300, "y2": 200}));
    assert!(!out.contains("!! overlaps"), "{out}");
    // pin в свободное место — тихо; pin поверх фигуры — с предупреждением.
    let out = call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 0, "x": 1000, "y": 1000, "w": 200}));
    assert!(!out.contains("!! overlaps"), "{out}");
    let out = call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 0, "x": 150, "y": 150, "w": 200}));
    assert!(out.contains("!! overlaps") && out.contains("#3 (x=100 y=100 w=200 h=100)"), "{out}");
}

/// Фигуры: рамка из x y w h, линия из абсолютных концов (рамка считается
/// сама), connect между закреплёнными блоками, ошибка для незакреплённого.
#[test]
fn shapes_create_update_and_connect() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Фигуры", "content": "Старт\n\nФиниш\n"})));
    let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "rect", "x": 40, "y": 40, "w": 200, "h": 100, "fill": "blue", "sw": 3}));
    assert!(out.contains("#2 shape:rect · x=40 y=40 w=200 h=100") && out.contains("fill=#") && out.contains("sw=3"), "{out}");
    let md = ctx.page_markdown(&page);
    assert!(md.contains("![[shape:rect]]{fill=") && md.contains("2 {h=100 w=200 x=40 y=40}"), "{md}");

    // Линия по абсолютным концам: рамка — bbox с полем 12.
    let out = call(ctx, "shape", serde_json::json!({"op": "create", "page": &page, "kind": "arrow", "x1": 100, "y1": 300, "x2": 300, "y2": 340}));
    assert!(out.contains("shape:arrow · x=88 y=288 w=224 h=64 · from (100,300) to (300,340)"), "{out}");
    // update концов пересчитывает рамку; смена вида сохраняет оформление.
    let out = call(ctx, "shape", serde_json::json!({"op": "update", "page": &page, "block": 3, "x2": 500, "y2": 300, "kind": "line", "stroke": "red", "dash": 6}));
    assert!(out.contains("shape:line · x=88 y=288 w=424 h=24 · from (100,300) to (500,300)") && out.contains("dash=6"), "{out}");

    // connect: незакреплённые блоки — ошибка; закрепим и соединим.
    let err = dispatch(ctx, "shape", &serde_json::json!({"op": "connect", "page": &page, "from": 0, "to": 1})).unwrap_err();
    assert!(err.contains("no coordinates"), "{err}");
    call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 0, "x": 0, "y": 0, "w": 100, "h": 40}));
    call(ctx, "blocks", serde_json::json!({"op": "pin", "page": &page, "block": 1, "x": 400, "y": 0, "w": 100, "h": 40}));
    let out = call(ctx, "shape", serde_json::json!({"op": "connect", "page": &page, "from": "find:Старт", "to": "find:Финиш"}));
    assert!(out.contains("connected #0 → #1 with arrow") && out.contains("from (100,20) to (400,20)"), "{out}");
    let out = call(ctx, "shape", serde_json::json!({"op": "delete", "page": &page, "block": 4}));
    assert!(out.contains("deleted #4 shape:arrow"), "{out}");
    assert!(dispatch(ctx, "shape", &serde_json::json!({"op": "delete", "page": &page, "block": 0})).is_err());
}

/// Страница: сетка/привязка через update, объекты — в позицию с высотой,
/// стиль доски и зум диаграммы.
#[test]
fn layout_objects_style_and_zoom_through_the_tool() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Раскладка", "content": "Один\n\nДва\n"})));
    let out = call(ctx, "update", serde_json::json!({"page": &page, "grid": "lines", "grid_step": 32, "snap": false}));
    assert!(out.contains("grid: lines") && out.contains("snap: off"), "{out}");
    let l = ctx.page_layout(&page);
    assert_eq!((l.grid, l.grid_step, l.snap), (PageGrid::Lines, 32.0, false));
    assert!(dispatch(ctx, "update", &serde_json::json!({"page": &page, "snap_step": 0})).is_err());
    let read = call(ctx, "read", serde_json::json!({"page": &page}));
    assert!(read.contains("layout: canvas · grid: lines step 32 · snap: off · bg: theme"), "{read}");
    let out = call(ctx, "update", serde_json::json!({"page": &page, "bg": "#243149"}));
    assert!(out.contains("bg: #243149"), "{out}");
    assert_eq!(ctx.page_layout(&page).bg, "#243149");
    assert!(dispatch(ctx, "update", &serde_json::json!({"page": &page, "bg": "plaid"})).is_err());
    call(ctx, "update", serde_json::json!({"page": &page, "bg": "none"}));
    assert!(ctx.page_layout(&page).bg.is_empty());

    // append в позицию и attach-подобная вставка с геометрией.
    let out = call(ctx, "update", serde_json::json!({"page": &page, "content": "Между", "mode": "append", "after": 0, "x": 10, "y": 20}));
    assert!(out.contains("inserted 1 block at block #1 (#1)"), "{out}");
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("#1 paragraph \"Между\" · x=10 y=20"), "{listed}");

    // Доска в начале страницы со своей высотой, стиль и чтение.
    let out = call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "index": 0, "h": 500, "x": 0, "y": 600, "title": "Доска"}));
    assert!(out.contains("created board at block #0 (#0, #1)"), "{out}");
    let listed = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    assert!(listed.contains("#0 heading3 \"Доска\" · flow") && listed.contains("#1 embed:kanban:") && listed.contains("x=0 y=600 w=520 h=500"), "{listed}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "set_style", "page": &page, "column_width": 320, "lane_bg": "#4F8CFF33", "show_counts": false}));
    assert!(out.contains("column_width=320 lane_bg=#4F8CFF33 card_bg=theme counts=off"), "{out}");
    assert!(dispatch(ctx, "kanban", &serde_json::json!({"op": "set_style", "page": &page, "column_width": 10})).is_err());
    call(ctx, "kanban", serde_json::json!({"op": "add_card", "page": &page, "title": "А"}));
    call(ctx, "kanban", serde_json::json!({"op": "add_card", "page": &page, "title": "Б"}));
    call(ctx, "kanban", serde_json::json!({"op": "update_card", "page": &page, "card": "Б", "before": "А"}));
    let (_, board) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "kanban").unwrap();
    let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &board) else { panic!() };
    assert_eq!(handle.lock().cards.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Б", "А"]);

    // Диаграмма: зум в пределах, «сегодня».
    call(ctx, "gantt", serde_json::json!({"op": "create", "page": &page}));
    let out = call(ctx, "gantt", serde_json::json!({"op": "set_zoom", "page": &page, "zoom": 40}));
    assert!(out.contains("zoom: 40 px/day"), "{out}");
    assert!(dispatch(ctx, "gantt", &serde_json::json!({"op": "set_zoom", "page": &page, "zoom": 500})).is_err());
    assert!(call(ctx, "gantt", serde_json::json!({"op": "show_today", "page": &page})).contains("today"));
}

/// График через инструмент: создание из таблицы, ряды, точечные
/// значения, вид, оформление, «из блока-таблицы», удаление.
#[test]
fn chart_through_the_tool() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Отчёт"})));
    let out = call(
        ctx,
        "chart",
        serde_json::json!({
            "op": "create",
            "page": &page,
            "kind": "bar",
            "title": "Квартал",
            "table": "| Месяц | План | Факт |\n| --- | --- | --- |\n| Янв | 10 | 12 |\n| Фев | 20 | 18 |",
            "style": {"stacked": true, "legend": "top", "y_max": 40}
        }),
    );
    assert!(out.contains("kind: bar") && out.contains("series: 2") && out.contains("points: 2"), "{out}");
    assert!(out.contains("\"Квартал\"") && out.contains("stacked true") && out.contains("legend top"), "{out}");
    let (_, chart) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "chart").unwrap();
    let Some(LiveObject::Chart { handle, .. }) = ctx.object("chart", &chart) else { panic!() };
    assert_eq!(handle.lock().options.y_max, Some(40.0));

    // Ряд: добавление, переименование, цвет, замена значений.
    let out = call(ctx, "chart", serde_json::json!({"op": "add_series", "chart": &chart, "name": "Прогноз", "data": [11, 19], "color": "green"}));
    assert!(out.contains("Прогноз") && out.contains("series: 3"), "{out}");
    call(ctx, "chart", serde_json::json!({"op": "update_series", "chart": &chart, "series": "Прогноз", "data": [15, 25], "color": "none"}));
    {
        let doc = handle.lock();
        let s = doc.find_series("Прогноз").unwrap();
        assert_eq!(s.data, vec![15.0, 25.0]);
        assert!(s.color.is_empty(), "«none» снимает цвет: {s:?}");
    }
    // Точечная правка и подписи.
    call(ctx, "chart", serde_json::json!({"op": "set_data", "chart": &chart, "series": "План", "value": 30, "index": 1}));
    call(ctx, "chart", serde_json::json!({"op": "set_data", "chart": &chart, "categories": ["Янв", "Фев", "Мар"]}));
    {
        let doc = handle.lock();
        assert_eq!(doc.find_series("План").unwrap().at(1), 30.0);
        assert_eq!(doc.categories.len(), 3, "новая подпись — новая точка у всех рядов");
        assert_eq!(doc.series[0].data.len(), 3);
    }

    // Вид: у круговой один ряд, и лишние отбрасываются.
    let out = call(ctx, "chart", serde_json::json!({"op": "update", "chart": &chart, "kind": "pie"}));
    assert!(out.contains("kind: pie") && out.contains("series: 1"), "{out}");
    assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "add_series", "chart": &chart, "name": "Ещё"})).is_err());
    assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "update", "chart": &chart, "kind": "sausage"})).is_err());

    // Шкала: значение, границы, зоны и единица.
    call(ctx, "chart", serde_json::json!({"op": "update", "chart": &chart, "kind": "gauge", "value": 72,
        "style": {"gauge_max": 120, "unit": "%", "zones": "0-60 green, 60-120 red"}}));
    {
        let doc = handle.lock();
        assert_eq!(doc.gauge_value(), 72.0);
        assert_eq!(doc.options.gauge_max, 120.0);
        assert_eq!(doc.options.unit, "%");
        assert_eq!(doc.options.zones.len(), 2);
        assert_eq!(doc.options.zones[1].color, "#EE5E48");
    }
    assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "set_style", "chart": &chart, "style": {"nope": 1}})).is_err());
    assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "set_style", "chart": &chart, "style": {"legend": "sideways"}})).is_err());

    // График из блока-таблицы страницы: таблица заменяется врезкой.
    call(ctx, "update", serde_json::json!({"page": &page, "content": "| Город | Людей |\n| --- | --- |\n| Алматы | 2 |\n| Астана | 1 |\n", "mode": "append"}));
    let idx = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    let block = idx.lines().find(|l| l.contains("table")).unwrap()[1..2].to_string();
    let out = call(ctx, "chart", serde_json::json!({"op": "from_table", "page": &page, "block": &block, "kind": "line"}));
    assert!(out.contains("chart with 1 series over 2 points") && out.contains("kind: line"), "{out}");
    assert_eq!(object_refs(&ctx.page_markdown(&page)).iter().filter(|(k, _)| k == "chart").count(), 2);
    // Не таблица — понятная ошибка, а не пустой график.
    let idx = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    if let Some(line) = idx.lines().find(|l| l.contains("paragraph")) {
        let b = line[1..2].to_string();
        assert!(dispatch(ctx, "chart", &serde_json::json!({"op": "from_table", "page": &page, "block": &b})).is_err());
    }

    // Чтение отдаёт и таблицу значений, и страницу.
    let out = call(ctx, "chart", serde_json::json!({"op": "read", "chart": &chart}));
    assert!(out.contains("value 72") && out.contains(&page), "{out}");

    // Удаление: врезка уходит со страницы.
    let out = call(ctx, "chart", serde_json::json!({"op": "delete", "chart": &chart}));
    assert!(out.contains("deleted chart:"), "{out}");
    assert_eq!(object_refs(&ctx.page_markdown(&page)).iter().filter(|(k, _)| k == "chart").count(), 1);
}

/// Интеллект-карта через инструмент: создание из списка, узлы, перенос,
/// кросс-ссылки, раскладка, стиль, «из блока».
#[test]
fn mindmap_through_the_tool() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Идеи"})));
    let out = call(
        ctx,
        "mindmap",
        serde_json::json!({"op": "create", "page": &page, "title": "Проект", "outline": "# Проект\n\n- Идеи\n  - Первая\n- Сроки\n", "direction": "both"}),
    );
    assert!(out.contains("nodes: 4") && out.contains("direction: both"), "{out}");
    assert!(out.contains("(#0)"), "карта — один блок, title идёт в корень: {out}");
    assert!(out.contains("\"Проект\"") && out.contains("\"Первая\""), "{out}");
    let (_, map) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "mindmap").unwrap();
    let Some(LiveObject::Mindmap { handle, .. }) = ctx.object("mindmap", &map) else { panic!() };

    // Узел: добавление под родителем по тексту, поля, свёрнутость.
    let out = call(ctx, "mindmap", serde_json::json!({"op": "add_node", "map": &map, "parent": "Сроки", "text": "Дедлайн", "color": "red", "icon": "🔥"}));
    assert!(out.contains("Дедлайн") && out.contains("icon 🔥"), "{out}");
    call(ctx, "mindmap", serde_json::json!({"op": "update_node", "page": &page, "node": "Дедлайн", "note": "- [ ] проверить", "link": "Идеи", "shape": "pill", "collapsed": true}));
    {
        let doc = handle.lock();
        let n = doc.nodes.iter().find(|n| n.text == "Дедлайн").unwrap();
        assert_eq!(n.link.as_deref(), Some(page.as_str()));
        assert_eq!(n.shape.key(), "pill");
        assert!(n.collapsed && n.note.contains("проверить"));
        assert_eq!(n.color, PALETTE[5]);
    }
    // Перенос: в своё поддерево нельзя, корень не переносится.
    assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "move_node", "map": &map, "node": "root", "parent": "Идеи"})).is_err());
    assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "move_node", "map": &map, "node": "Идеи", "parent": "Первая"})).is_err());
    call(ctx, "mindmap", serde_json::json!({"op": "move_node", "map": &map, "node": "Дедлайн", "parent": "Идеи", "index": 0}));
    {
        // Один захват мьютекса на выражение: вложенный lock() — дедлок.
        let doc = handle.lock();
        let ideas = doc.nodes.iter().find(|n| n.text == "Идеи").unwrap().id.clone();
        assert_eq!(doc.children_of(&ideas)[0].text, "Дедлайн");
    }

    // Кросс-ссылка, раскладка, стиль.
    let out = call(ctx, "mindmap", serde_json::json!({"op": "add_link", "map": &map, "from": "Первая", "to": "Сроки", "label": "см."}));
    assert!(out.contains("link") && out.contains("\"см.\""), "{out}");
    assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "add_link", "map": &map, "from": "Первая", "to": "Сроки"})).is_err());
    call(ctx, "mindmap", serde_json::json!({"op": "set_layout", "map": &map, "direction": "down", "curve": "elbow", "h_gap": 60}));
    let l = handle.layout();
    assert_eq!((l.direction.key(), l.curve.key(), l.h_gap), ("down", "elbow", 60.0));
    call(ctx, "mindmap", serde_json::json!({"op": "set_style", "map": &map, "style": {"palette": "rainbow", "font_size": 15, "show_icons": false}}));
    let st = handle.style();
    assert_eq!((st.font_size, st.show_icons, st.palette[0].as_str()), (15.0, false, "#FF5A5F"));
    assert!(dispatch(ctx, "mindmap", &serde_json::json!({"op": "set_style", "map": &map, "style": {"nope": 1}})).is_err());

    // Удаление узла с поддеревом и карта из блока страницы.
    let out = call(ctx, "mindmap", serde_json::json!({"op": "delete_node", "map": &map, "node": "Идеи"}));
    assert!(out.contains("deleted 3 nodes"), "перенесённый «Дедлайн» уходит с поддеревом: {out}");
    call(ctx, "update", serde_json::json!({"page": &page, "content": "- Альфа\n  - А1\n- Бета\n", "mode": "append"}));
    let idx = call(ctx, "blocks", serde_json::json!({"op": "list", "page": &page}));
    let block = idx.lines().find(|l| l.contains("bullet")).unwrap()[1..2].to_string();
    let out = call(ctx, "mindmap", serde_json::json!({"op": "from_list", "page": &page, "block": &block, "title": "Список"}));
    // Каждый пункт верхнего уровня — свой блок: берётся «Альфа» с «А1».
    assert!(out.contains("mind map with 3 nodes") && out.contains("\"А1\""), "{out}");
    assert_eq!(object_refs(&ctx.page_markdown(&page)).iter().filter(|(k, _)| k == "mindmap").count(), 2);

    let out = call(ctx, "mindmap", serde_json::json!({"op": "delete", "map": &map}));
    assert!(out.contains("embeds removed from 1 page"), "{out}");
    assert!(!ctx.page_markdown(&page).contains(&format!("mindmap:{map}")));
}

/// Календарь: события в общем хранилище, повторы, диапазон, виджет.
#[test]
fn calendar_through_the_tool() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "План"})));
    let out = call(ctx, "calendar", serde_json::json!({"op": "create", "page": &page, "view": "week", "anchor": "2026-09-03"}));
    assert!(out.contains("(week view)"), "{out}");
    let (_, widget) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "calendar").unwrap();

    // Событие со временем и повтором; список за диапазон.
    let out = call(
        ctx,
        "calendar",
        serde_json::json!({"op": "add_event", "title": "Стендап", "date": "2026-09-01", "start_time": "09:30", "end_time": "10:00", "repeat": "weekly", "until": "2026-09-30", "color": "blue"}),
    );
    assert!(out.contains("09:30–10:00") && out.contains("repeat weekly until 2026-09-30"), "{out}");
    call(ctx, "calendar", serde_json::json!({"op": "add_event", "title": "Отпуск", "date": "2026-09-10", "end_date": "2026-09-12"}));

    // Будни одним событием: repeat=weekdays вместо дюжины отдельных дат.
    let out = call(
        ctx,
        "calendar",
        serde_json::json!({"op": "add_event", "title": "Зарядка", "date": "2026-09-14", "repeat": "weekdays", "until": "2026-09-25"}),
    );
    assert!(out.contains("repeat daily on mon,tue,wed,thu,fri until 2026-09-25"), "{out}");
    // 19–20 сентября — суббота и воскресенье: вхождений нет.
    let weekend = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": "2026-09-19", "to": "2026-09-20"}));
    assert!(!weekend.contains("Зарядка"), "{weekend}");
    let monday = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": "2026-09-21", "to": "2026-09-21"}));
    assert!(monday.contains("Зарядка"), "{monday}");
    // «Сделано» у повтора — отметка одного дня: пятница не закрывает понедельник.
    let out = call(ctx, "calendar", serde_json::json!({"op": "complete", "event": "Зарядка", "on": "2026-09-18"}));
    assert!(out.contains("occurrence 2026-09-18 done") && out.contains("done on 2026-09-18"), "{out}");
    {
        let store = ctx.calendar_store();
        let s = store.lock();
        let e = s.events.iter().find(|e| e.title == "Зарядка").unwrap();
        let d = |iso: &str| crate::pages::notes::gantt::calendar::parse_days(iso).unwrap();
        assert!(e.done_at(d("2026-09-18")) && !e.done_at(d("2026-09-21")), "{:?}", e.done_on);
    }
    let err = dispatch(ctx, "calendar", &serde_json::json!({"op": "complete", "event": "Зарядка", "on": "2026-09-19"})).unwrap_err();
    assert!(err.contains("no occurrence on 2026-09-19") && err.contains("nearest: 2026-09-18, 2026-09-21"), "{err}");
    // skip_days считается от целой недели, а не от прежней маски.
    let out = call(ctx, "calendar", serde_json::json!({"op": "update_event", "event": "Зарядка", "skip_days": ["вс"]}));
    assert!(out.contains("repeat daily on mon,tue,wed,thu,fri,sat"), "{out}");
    let out = call(ctx, "calendar", serde_json::json!({"op": "update_event", "event": "Зарядка", "only_days": ["mon", "wed", "fri"]}));
    assert!(out.contains("repeat daily on mon,wed,fri"), "{out}");
    // Фильтр без повтора — ошибка с подсказкой, а не молчаливое «каждый день».
    let err = dispatch(ctx, "calendar", &serde_json::json!({"op": "add_event", "title": "Разово", "date": "2026-09-14", "only_days": ["mon"]})).unwrap_err();
    assert!(err.contains("need a repeat"), "{err}");
    let err = dispatch(ctx, "calendar", &serde_json::json!({"op": "update_event", "event": "Зарядка", "only_days": ["funday"]})).unwrap_err();
    assert!(err.contains("bad weekday"), "{err}");
    let out = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": "2026-09-01", "to": "2026-09-30"}));
    assert!(out.contains("Стендап") && out.contains("Отпуск") && out.contains("2026-09-10 → 2026-09-12"), "{out}");
    let narrow = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": "2026-09-11", "to": "2026-09-11"}));
    assert!(narrow.contains("Отпуск") && !narrow.contains("Стендап"), "{narrow}");

    // Календари: свой, фильтр виджета, переезд событий при удалении.
    let work = call(ctx, "calendar", serde_json::json!({"op": "add_calendar", "name": "Работа", "color": "green"}));
    assert!(work.contains("Работа"), "{work}");
    call(ctx, "calendar", serde_json::json!({"op": "update_event", "event": "Стендап", "calendar": "Работа"}));
    let store = ctx.calendar_store();
    assert_eq!(store.lock().calendars.len(), 2);
    call(ctx, "calendar", serde_json::json!({"op": "set_view", "calendar": &widget, "view": "month", "calendars": ["Работа"]}));
    {
        let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", &widget) else { panic!() };
        let d = handle.lock();
        assert_eq!(d.view.key(), "month");
        assert_eq!(d.calendars.len(), 1);
    }
    // Перенос, «сделано», удаление.
    call(ctx, "calendar", serde_json::json!({"op": "move_event", "event": "Отпуск", "date": "2026-09-17"}));
    {
        let s = store.lock();
        let e = s.events.iter().find(|e| e.title == "Отпуск").unwrap();
        assert_eq!((e.date.as_str(), e.end_date.as_deref()), ("2026-09-17", Some("2026-09-19")), "многодневность сохраняется");
    }
    let out = call(ctx, "calendar", serde_json::json!({"op": "complete", "event": "Отпуск"}));
    assert!(out.contains("done"), "{out}");
    assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "move_event", "event": "Отпуск"})).is_err());
    call(ctx, "calendar", serde_json::json!({"op": "delete_event", "event": "Отпуск"}));
    assert_eq!(store.lock().events.len(), 2, "остались «Стендап» и «Зарядка»");

    // Стиль виджета и слои; удаление виджета не трогает события.
    call(ctx, "calendar", serde_json::json!({"op": "set_style", "calendar": &widget, "style": {"preset": "light", "hour_from": 7, "hour_to": 22, "slot_min": 15, "show_kanban_due": true}}));
    {
        let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", &widget) else { panic!() };
        let st = handle.style();
        assert_eq!((st.from_min, st.to_min, st.slot_min, st.show_kanban_due), (7 * 60, 22 * 60, 15, true), "старые ключи часов — это минуты окна");
        assert_eq!(st.preset, "light");
    }
    // Окно суток со сдвигом: «HH:MM» и полные сутки от начала.
    call(ctx, "calendar", serde_json::json!({"op": "set_style", "calendar": &widget, "style": {"day_from": "06:30", "full_day": true}}));
    {
        let Some(LiveObject::Calendar { handle, .. }) = ctx.object("calendar", &widget) else { panic!() };
        let st = handle.style();
        assert_eq!((st.from_min, st.full_day), (6 * 60 + 30, true));
        assert_eq!(st.window(), (6 * 60 + 30, 24 * 60));
    }
    assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "set_style", "calendar": &widget, "style": {"nope": 1}})).is_err());
    let first = store.lock().calendars[0].id.clone();
    assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "delete_calendar", "calendar": &first})).is_ok());
    assert!(dispatch(ctx, "calendar", &serde_json::json!({"op": "delete_calendar", "calendar": "Работа"})).is_err(), "последний календарь");
    let out = call(ctx, "calendar", serde_json::json!({"op": "delete", "calendar": &widget}));
    assert!(out.contains("the events stay"), "{out}");
    assert_eq!(store.lock().events.len(), 2, "события остаются в проекте");
}

/// Каждый ключ, который читает инструмент, обязан быть в схеме — иначе
/// валидатор (`additionalProperties: false`) его отрежет.
#[test]
fn schema_covers_every_argument() {
    let schema = crate::agent::tools::catalog::notes_schema();
    let props = schema["properties"].as_object().unwrap();
    // Каждый модуль инструмента — свой файл; читаем их все, иначе новый
    // аргумент в новом модуле проехал бы мимо проверки.
    let body = [
        include_str!("mod.rs"),
        include_str!("args.rs"),
        include_str!("refs.rs"),
        include_str!("md.rs"),
        include_str!("geom.rs"),
        include_str!("attrs.rs"),
        include_str!("pages.rs"),
        include_str!("blocks.rs"),
        include_str!("shapes.rs"),
        include_str!("kanban.rs"),
        include_str!("gantt.rs"),
        include_str!("mindmap.rs"),
        include_str!("calendar.rs"),
        include_str!("chart.rs"),
        include_str!("life.rs"),
    ]
    .join("\n");
    let body = body.as_str();
    let mut missing = Vec::new();
    for pat in [
        "str_field(v, \"",
        "ref_field(v, \"",
        "raw_string(v, \"",
        "bool_field(v, \"",
        "usize_field(v, \"",
        "f32_field(v, \"",
        "f64_field(v, \"",
        "data_field(v, \"",
        "list_field(v, \"",
    ] {
        for (i, _) in body.match_indices(pat) {
            let rest = &body[i + pat.len()..];
            let key = rest.split('"').next().unwrap();
            if !props.contains_key(key) && !missing.contains(&key) {
                missing.push(key);
            }
        }
    }
    for key in ["x", "y", "w", "h", "attrs"] {
        assert!(props.contains_key(key), "schema lacks {key}");
    }
    assert!(missing.is_empty(), "schema lacks {missing:?}");
}

#[test]
fn colors_priorities_and_dates_parse() {
    assert_eq!(parse_color("red").unwrap(), PALETTE[5]);
    assert_eq!(parse_color("#abc").unwrap(), "#AABBCC");
    assert_eq!(parse_color("none").unwrap(), "");
    assert!(parse_color("plaid").is_err());
    assert_eq!(parse_priority("High").unwrap(), Some(Priority::High));
    assert_eq!(parse_priority("none").unwrap(), None);
    assert!(parse_priority("meh").is_err());
    assert_eq!(parse_due("2026-09-10").unwrap().as_deref(), Some("2026-09-10"));
    assert!(parse_due("вчера").is_err());
    assert_eq!(fnum(12.0), "12");
    assert_eq!(fnum(12.34), "12.3");
    assert_eq!(validate_attr("size", "22").unwrap().as_deref(), Some("22"));
    assert!(validate_attr("size", "500").is_err());
    assert_eq!(validate_attr("stroke", "none").unwrap().as_deref(), Some("none"));
    assert_eq!(validate_attr("bg", "").unwrap(), None);
    assert_eq!(validate_attr("fill", "#4F8CFF80").unwrap().as_deref(), Some("#4F8CFF80"));
    assert!(validate_attr("font", "x").is_err());
    let v = serde_json::json!({"tags": "ui, дизайн", "n": "3", "b": "yes"});
    assert_eq!(list_field(&v, "tags").unwrap(), vec!["ui", "дизайн"]);
    assert_eq!(usize_field(&v, "n"), Some(3));
    assert_eq!(bool_field(&v, "b"), Some(true));
}

#[test]
fn pages_lifecycle_through_the_tool() {
    let ctx = ctx();
    let out = call(ctx, "create", serde_json::json!({"title": "Проект", "content": "# Проект\n\nВступление\n"}));
    let root = page_id(&out);
    let out = call(ctx, "create", serde_json::json!({"title": "Идеи", "parent": "Проект", "icon": "💡"}));
    let child = page_id(&out);
    assert_eq!(ctx.tree.get_untracked().parent_of(&child).as_deref(), Some(root.as_str()));

    // Адресация: id, название, путь; неоднозначность — ошибка.
    assert_eq!(resolve_page_ref(ctx, "идеи").unwrap(), child);
    assert_eq!(resolve_page_ref(ctx, "Проект / Идеи").unwrap(), child);
    call(ctx, "create", serde_json::json!({"title": "Идеи"}));
    assert!(resolve_page_ref(ctx, "Идеи").unwrap_err().contains("matches 2"));
    assert_eq!(resolve_page_ref(ctx, "Проект/Идеи").unwrap(), child);

    // list и read.
    let listed = call(ctx, "list", serde_json::json!({}));
    assert!(listed.contains(&format!("  {child} \"Идеи\" 💡")), "{listed}");
    let read = call(ctx, "read", serde_json::json!({"page": &root}));
    assert!(read.contains("Вступление"), "{read}");
    assert!(read.contains("children: 1"), "{read}");

    // update: append, find/replace, переименование, раскладка.
    call(ctx, "update", serde_json::json!({"page": &root, "content": "- [ ] задача", "mode": "append"}));
    let md = ctx.page_markdown(&root);
    assert!(md.contains("Вступление\n\n- [ ] задача"), "{md}");
    let err = dispatch(ctx, "update", &serde_json::json!({"page": &root, "find": "нет такого", "replace": "x"}))
        .unwrap_err();
    assert!(err.contains("not found"), "{err}");
    call(ctx, "update", serde_json::json!({"page": &root, "find": "- [ ] задача", "replace": "- [x] задача", "title": "План"}));
    assert!(ctx.page_markdown(&root).contains("- [x] задача"));
    assert_eq!(ctx.title_of(&root), "План");
    // Режима «поток» больше нет — просьбу переключиться отклоняем.
    let err = dispatch(ctx, "update", &serde_json::json!({"page": &root, "layout": "flow"}))
        .unwrap_err();
    assert!(err.contains("canvas-only"), "{err}");
    assert!(ctx.page(&root).unwrap().handle.history_state().get_untracked().0, "правка агента должна отменяться");

    // search, duplicate, move, delete.
    let found = call(ctx, "search", serde_json::json!({"query": "ЗАДАЧА"}));
    assert!(found.contains(&root), "{found}");
    let copy = page_id(&call(ctx, "duplicate", serde_json::json!({"page": &root})));
    assert_ne!(copy, root);
    assert_eq!(ctx.title_of(&copy), "План (копия)");
    call(ctx, "move", serde_json::json!({"page": &child, "parent": "root", "index": 0}));
    assert_eq!(ctx.tree.get_untracked().parent_of(&child), None);
    assert!(dispatch(ctx, "move", &serde_json::json!({"page": &root, "parent": &copy})).is_ok());
    assert!(dispatch(ctx, "move", &serde_json::json!({"page": &copy, "parent": &root})).is_err());
    // Копия сделана до переноса «Идей» в корень — в ней своя копия
    // «Идей», плюс перенесённый внутрь оригинал: три страницы.
    let out = call(ctx, "delete", serde_json::json!({"page": &copy}));
    assert!(out.starts_with("deleted 3 pages"), "{out}");
    assert!(ctx.tree.get_untracked().find(&root).is_none());
}

#[test]
fn kanban_and_gantt_through_the_tool() {
    let ctx = ctx();
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Спринт"})));
    let out = call(
        ctx,
        "kanban",
        serde_json::json!({"op": "create", "page": "Спринт", "title": "Доска", "columns": ["Бэклог", "В работе", "Готово"]}),
    );
    assert!(out.contains("column"), "{out}");
    let md = ctx.page_markdown(&page);
    assert!(md.contains("### Доска") && md.contains("![[kanban:"), "{md}");
    let (_, board_id) = object_refs(&md).into_iter().next().unwrap();

    // Доска находится и по странице, и по id; карточки — по заголовку.
    let out = call(
        ctx,
        "kanban",
        serde_json::json!({"op": "add_card", "page": "Спринт", "column": "Бэклог", "title": "Импорт", "md": "- [ ] парсер\n- [x] тесты", "priority": "high", "tags": "core, io", "due": "2026-09-10"}),
    );
    assert!(out.contains("priority: high") && out.contains("checklist 1/2") && out.contains("due: 2026-09-10"), "{out}");
    call(ctx, "kanban", serde_json::json!({"op": "add_card", "board": &board_id, "column": 1, "title": "Экспорт"}));
    call(ctx, "kanban", serde_json::json!({"op": "move_card", "board": &board_id, "card": "Экспорт", "column": "Готово"}));
    call(ctx, "kanban", serde_json::json!({"op": "update_card", "board": &board_id, "card": "Импорт", "priority": "none", "column": "В работе"}));
    let Some(LiveObject::Kanban { handle, .. }) = ctx.object("kanban", &board_id) else { panic!() };
    {
        let doc = handle.lock();
        assert_eq!(doc.cards.len(), 2);
        let import = doc.cards.iter().find(|c| c.title == "Импорт").unwrap();
        assert_eq!(import.priority, None);
        assert_eq!(doc.columns.iter().find(|c| c.id == import.column).unwrap().name, "В работе");
        assert_eq!(doc.cards_of(&doc.columns[2].id).len(), 1);
    }
    let out = call(ctx, "kanban", serde_json::json!({"op": "read", "board": &board_id}));
    assert!(out.contains("| - [ ] парсер"), "{out}");
    assert!(out.contains(&format!("page: {page}")), "{out}");
    call(ctx, "kanban", serde_json::json!({"op": "add_column", "board": &board_id, "name": "Ревью", "color": "purple"}));
    call(ctx, "kanban", serde_json::json!({"op": "delete_column", "board": &board_id, "column": "Готово"}));
    assert_eq!(handle.lock().columns.len(), 3);
    assert_eq!(handle.lock().cards.len(), 2, "карточки удалённой колонки переезжают");
    call(ctx, "kanban", serde_json::json!({"op": "delete_card", "board": &board_id, "card": "Экспорт"}));
    assert_eq!(handle.lock().cards.len(), 1);

    // Диаграмма.
    call(ctx, "gantt", serde_json::json!({"op": "create", "page": &page}));
    let out = call(ctx, "gantt", serde_json::json!({"op": "add_task", "page": &page, "name": "Дизайн", "start": "2026-09-01", "end": "2026-09-03"}));
    assert!(out.contains("2026-09-01 → 2026-09-03 (3 days)"), "{out}");
    call(ctx, "gantt", serde_json::json!({"op": "add_task", "page": &page, "name": "Код", "start": "2026-09-04", "after": "Дизайн", "color": "green"}));
    let out = call(ctx, "gantt", serde_json::json!({"op": "read", "page": &page}));
    assert!(out.contains("dep") && out.contains("\"Дизайн\" →"), "{out}");
    call(ctx, "gantt", serde_json::json!({"op": "update_task", "page": &page, "task": "Код", "start": "2026-09-05"}));
    let (_, chart_id) = object_refs(&ctx.page_markdown(&page)).into_iter().find(|(k, _)| k == "gantt").unwrap();
    let Some(LiveObject::Gantt { handle: gh, .. }) = ctx.object("gantt", &chart_id) else { panic!() };
    let code = gh.lock().tasks.iter().find(|t| t.name == "Код").cloned().unwrap();
    assert_eq!((code.start.as_str(), code.end.as_str()), ("2026-09-05", "2026-09-07"), "длительность сохраняется");
    assert_eq!(code.color, PALETTE[2]);

    // Удаление объектов убирает врезки со страницы.
    let out = call(ctx, "gantt", serde_json::json!({"op": "delete", "chart": &chart_id}));
    assert!(out.contains("embeds removed from 1 page"), "{out}");
    assert!(!ctx.page_markdown(&page).contains("gantt:"), "{}", ctx.page_markdown(&page));
    call(ctx, "kanban", serde_json::json!({"op": "delete", "board": &board_id}));
    assert!(!ctx.page_markdown(&page).contains("kanban:"));
    assert!(ctx.page_markdown(&page).contains("### Доска"), "заголовок над доской остаётся");
    assert!(dispatch(ctx, "kanban", &serde_json::json!({"op": "read", "page": &page})).is_err());
}

/// Массовое чтение: страница с подстраницами, список страниц и весь
/// проект — одним вызовом. По странице за ход локальная модель платит
/// полным префиллом за каждую подстраницу.
#[test]
fn read_takes_many_pages_at_once() {
    let _serial = budget::test_serial();
    let ctx = ctx();
    let root = page_id(&call(ctx, "create", serde_json::json!({"title": "Проект", "content": "Корень\n"})));
    let a = page_id(&call(
        ctx,
        "create",
        serde_json::json!({"title": "Раздел А", "parent": &root, "content": "Текст А\n"}),
    ));
    let b = page_id(&call(
        ctx,
        "create",
        serde_json::json!({"title": "Раздел Б", "parent": &root, "content": "Текст Б\n"}),
    ));
    call(ctx, "create", serde_json::json!({"title": "Подраздел", "parent": &a, "content": "Глубокий текст\n"}));

    // Одна страница — прежний формат, без шапки пачки и без соседей.
    let one = call(ctx, "read", serde_json::json!({"page": &root}));
    assert!(one.starts_with("page: "), "{one}");
    assert!(!one.contains("Текст А"), "{one}");

    // Поддерево целиком.
    assert!(one.contains("--- Sub-pages (2)"), "одиночное чтение называет подстраницы: {one}");

    let tree = call(ctx, "read", serde_json::json!({"page": &root, "depth": "all"}));
    assert!(tree.starts_with("--- 4 pages"), "{tree}");
    for t in ["Корень", "Текст А", "Текст Б", "Глубокий текст"] {
        assert!(tree.contains(t), "нет «{t}»: {tree}");
    }

    // Один уровень — дети без внуков.
    let level1 = call(ctx, "read", serde_json::json!({"page": &root, "depth": 1}));
    assert!(level1.contains("Текст А") && !level1.contains("Глубокий текст"), "{level1}");

    // Явный список страниц и весь проект.
    let pair = call(ctx, "read", serde_json::json!({"pages": [&a, &b]}));
    assert!(pair.contains("Текст А") && pair.contains("Текст Б") && !pair.contains("Корень"), "{pair}");
    let all = call(ctx, "read", serde_json::json!({"page": "all"}));
    assert!(all.contains("=== page 4/4 ==="), "{all}");
    let two = call(ctx, "read", serde_json::json!({"page": "all", "limit": 2}));
    assert!(two.starts_with("--- 2 pages"), "{two}");
}

/// Пачка обязана делиться сама: иначе укладка выхлопа в окно
/// (`fit_for_prompt`) вырежет из ответа середину — целые страницы, о
/// пропаже которых модель узнаёт только по дыре в тексте.
#[test]
fn notes_answers_are_not_clipped_by_history() {
    // Клипа поверх `notes` нет: инструмент сам меряет ответ живым окном
    // и обрывает его на границе страницы. Статический клип вырезал бы
    // у честной пачки середину — целые страницы, о пропаже которых
    // модель узнаёт только по дыре в тексте.
    assert_eq!(
        super::super::executor::history_limit(super::super::catalog::KEY_NOTES),
        None
    );
    assert!(
        READ_MAX_BYTES < MAX_NOTES_OUTPUT_BYTES,
        "предохранитель исполнителя не оставил места шапке и списку недочитанного"
    );
}

/// Свободное окно — весь проект одним ответом: своего потолка у
/// инструмента больше нет, границу ставит только контекст.
#[test]
fn a_roomy_window_reads_the_whole_project_in_one_reply() {
    let _serial = budget::test_serial();
    let ctx = ctx();
    let long = "Строка текста для объёма.\n\n".repeat(110);
    for i in 1..=12 {
        call(ctx, "create", serde_json::json!({"title": format!("Стр {i}"), "content": &long}));
    }
    // Без бюджета те же страницы в один ответ не влезают (фолбэк —
    // 8000 токенов), с живым окном на 200k — влезают все.
    assert!(call(ctx, "read", serde_json::json!({"page": "all"})).contains("did not fit"));

    let _budget = budget::arm(200_000, std::sync::Arc::new(|s: &str| s.chars().count() / 3));
    let all = call(ctx, "read", serde_json::json!({"page": "all"}));
    assert!(all.contains("all of them in this reply"), "{}", &all[..160]);
    assert_eq!(all.matches("=== page ").count(), 12);
    assert!(all.contains("left in context"), "шапка называет цену ответа и остаток окна");
    assert!(!all.contains("Context is filling up"), "окно свободно — пугать нечем");
}

/// Одна страница больше бюджета: обрывается по строке, называет
/// остаток и способ его дочитать — а не уезжает под клип истории.
#[test]
fn a_huge_single_page_is_cut_with_a_way_to_finish_it() {
    let _serial = budget::test_serial();
    let ctx = ctx();
    let long = "Строка текста для объёма.\n\n".repeat(400);
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Полотно", "content": &long})));

    let _budget = budget::arm(2_000, std::sync::Arc::new(|s: &str| s.chars().count() / 3));
    let out = call(ctx, "read", serde_json::json!({"page": &page}));
    assert!(out.contains("--- Page truncated: "), "{}", &out[..200]);
    assert!(out.contains("blocks {\"op\": \"read\""), "нужен способ дочитать остаток");
    assert!(
        budget::count(&out) <= 1_000,
        "ответ на ~{} токенов больше гранта",
        budget::count(&out)
    );
    // Обрыв по границе строки: половинок строк в ответе нет.
    let body = out.split("--- Page truncated").next().unwrap();
    assert!(
        body.matches("Строка текста для объёма.").count() > 0
            && body.ends_with('\n'),
        "страница обязана обрываться на строке"
    );
}

/// Тесное окно опускает потолок ответа, и модель узнаёт об этом из
/// шапки — вместо молчаливой дыры в середине.
#[test]
fn a_tight_context_shrinks_the_reply_and_says_so() {
    let _serial = budget::test_serial();
    let ctx = ctx();
    let long = "Строка текста для объёма.\n\n".repeat(60);
    for i in 1..=12 {
        call(ctx, "create", serde_json::json!({"title": format!("Стр {i}"), "content": &long}));
    }
    let roomy = call(ctx, "read", serde_json::json!({"page": "all"}));

    // 4000 токенов окна → грант 2000 → страниц влезает меньше.
    let _budget = budget::arm(4_000, std::sync::Arc::new(|s: &str| s.chars().count() / 3));
    let tight = call(ctx, "read", serde_json::json!({"page": "all"}));
    let pages = |s: &str| s.matches("=== page ").count();
    assert!(
        pages(&tight) < pages(&roomy),
        "тесное окно ({} страниц) должно отдавать меньше свободного ({})",
        pages(&tight),
        pages(&roomy)
    );
    assert!(tight.contains("left in context"), "шапка обязана назвать остаток окна: {}", &tight[..120]);
    assert!(tight.contains("Context is filling up"), "модель должна узнать причину обрыва");
}

/// Массовое чтение обрывается на границе страницы и называет остаток.
#[test]
fn read_stops_on_a_page_boundary_and_lists_the_rest() {
    let _serial = budget::test_serial();
    let ctx = ctx();
    // Каждая страница — заметно больше десятой доли бюджета, так что
    // в один ответ они все не влезают.
    let long = "Строка текста для объёма.\n\n".repeat(110);
    for i in 1..=12 {
        call(ctx, "create", serde_json::json!({"title": format!("Стр {i}"), "content": &long}));
    }
    let all = call(ctx, "read", serde_json::json!({"page": "all"}));
    assert!(all.starts_with("--- ") && all.contains("did not fit in one reply"), "{}", &all[..200]);
    assert!(all.contains("--- Not read ("), "остаток должен быть перечислен по id");
    assert!(
        budget::count(&all) <= READ_FALLBACK_TOKENS && all.len() <= READ_MAX_BYTES,
        "ответ на ~{} токенов вышел за бюджет",
        budget::count(&all)
    );
    // Обрыв ровно на границе: сколько страниц названо в шапке, столько
    // их и в ответе — целиком, а не с оборванным хвостом у последней.
    let done: usize = all.split(' ').nth(1).and_then(|n| n.parse().ok()).expect("шапка пачки");
    assert!(done > 0 && done < 12, "прочитано {done} из 12");
    assert_eq!(all.matches("=== page ").count(), done);
    assert_eq!(
        all.matches("Строка текста для объёма.").count(),
        done * 110,
        "страница попала в ответ не целиком"
    );
}

/// Блоки страницы читаются пачкой: `all`, список и диапазон; одиночная
/// ссылка отвечает как прежде.
#[test]
fn blocks_read_takes_a_list_and_the_whole_page() {
    let _serial = budget::test_serial();
    let ctx = ctx();
    let page = page_id(&call(
        ctx,
        "create",
        serde_json::json!({"title": "Холст-чтение", "content": "# Схема\n\nПервый\n\nВторой\n\nТретий\n"}),
    ));
    let all = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "all"}));
    assert!(all.starts_with("--- 4 blocks ---"), "{all}");
    assert!(all.contains("# Схема") && all.contains("Первый") && all.contains("Третий"), "{all}");

    let some = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "1,3"}));
    assert!(some.starts_with("--- 2 blocks ---"), "{some}");
    assert!(some.contains("Первый") && some.contains("Третий") && !some.contains("Второй"), "{some}");

    let range = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "2-3"}));
    assert!(range.contains("Второй") && range.contains("Третий") && !range.contains("Первый"), "{range}");

    // Одиночная ссылка (индекс или find:) — прежний ответ без шапки.
    let single = call(ctx, "blocks", serde_json::json!({"op": "read", "page": &page, "block": "find:Второй"}));
    assert!(single.starts_with("#2 paragraph"), "{single}");
    assert!(dispatch(ctx, "blocks", &serde_json::json!({"op": "read", "page": &page, "block": "9"})).is_err());
}

/// Панель свойств помнится за страницей и уезжает в бандл: у каждой
/// страницы своё положение разделителя и свой «скрыта».
#[test]
fn props_panel_state_is_per_page() {
    let ctx = ctx();
    let a = page_id(&call(ctx, "create", serde_json::json!({"title": "Холст"})));
    let b = page_id(&call(ctx, "create", serde_json::json!({"title": "Заметка"})));
    assert_eq!(ctx.props_panel(&a), (None, false), "новая страница — без своих значений");

    ctx.set_props_panel(&a, 0.72, true);
    ctx.set_props_panel(&b, 0.5, false);
    assert_eq!(ctx.props_panel(&a), (Some(0.72), true));
    assert_eq!(ctx.props_panel(&b), (Some(0.5), false));

    // Положение квантуется — перетаскивание разделителя не поднимает
    // ревизию дерева на каждый пиксель.
    let rev = ctx.tree_rev.get_untracked();
    ctx.set_props_panel(&a, 0.7201, true);
    assert_eq!(ctx.tree_rev.get_untracked(), rev, "тот же квант — без правки дерева");
    ctx.set_props_panel(&a, 0.75, true);
    assert!(ctx.tree_rev.get_untracked() > rev);

    // Пережило запись и чтение проекта.
    let json = ctx.tree.get_untracked().serialize();
    let back = crate::pages::notes::project::ProjectTree::parse(&json).unwrap();
    assert_eq!(back.layout_of(&a).props_ratio, Some(0.75));
    assert!(back.layout_of(&a).props_hidden);
    assert!(!back.layout_of(&b).props_hidden);
}

/// Календарь по доскам (09.09.2026): оценка длительности плюс «в
/// календарь» дают карточке полосу; календарь видит её отрезком и
/// двигает, диаграмма Ганта — своей строкой, agenda кладёт её в
/// «Сегодня» даже без срока.
#[test]
fn board_cards_become_calendar_bars_and_gantt_rows() {
    use crate::pages::notes::calendar::{ExternalKind, ExternalQuery};
    let ctx = ctx();
    let today = today_days();
    let iso = |d: i64| days_to_iso(today + d);
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Спринт"})));
    let out = call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "columns": ["Бэклог", "Готово"]}));
    let board = out.lines().find_map(|l| l.strip_prefix("kanban:")).unwrap().split(' ').next().unwrap().to_string();
    call(ctx, "kanban", serde_json::json!({"op": "add_card", "board": &board, "column": "Бэклог", "title": "Импорт"}));

    // «Оценка 1 день» + «в календарь на завтра» — карточка получает полосу.
    let out = call(
        ctx,
        "kanban",
        serde_json::json!({"op": "schedule", "board": &board, "card": "Импорт", "date": "tomorrow", "duration": "1d"}),
    );
    assert!(out.contains("duration: 1d") && out.contains(&format!("planned: {}", iso(1))), "{out}");

    // Календарь видит её полосой; срок не выдуман — его нет.
    let env = embeds::calendar_env(ctx);
    let q = ExternalQuery { from: today, to: today + 7, boards: Vec::new(), due: true, spans: true, gantt: true };
    let items = (env.external)(&q);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!((items[0].day, items[0].end_day, items[0].source.kind), (today + 1, today + 1, ExternalKind::Card));

    // Перенос полосы в календаре двигает start/end карточки.
    assert!((env.shift_external)(&items[0].source, 2));
    let card = |ctx: NotesCtx| match ctx.object("kanban", &board) {
        Some(LiveObject::Kanban { handle, .. }) => handle.card(&items[0].source.item).unwrap(),
        _ => panic!("доска пропала"),
    };
    assert_eq!(card(ctx).start.as_deref(), Some(iso(3).as_str()));
    assert_eq!(card(ctx).end.as_deref(), Some(iso(3).as_str()));

    // Диаграмма Ганта показывает ту же карточку строкой и правит её даты.
    let g = call(ctx, "gantt", serde_json::json!({"op": "create", "page": &page}));
    let gid = g.lines().find_map(|l| l.strip_prefix("created chart gantt:")).unwrap().split(' ').next().unwrap().to_string();
    let out = call(ctx, "gantt", serde_json::json!({"op": "set_boards", "gantt": &gid, "boards": [&board]}));
    assert!(out.contains(&format!("boards: {board}")), "{out}");
    let genv = embeds::gantt_env(ctx);
    assert!((genv.cards)(&[]).is_empty(), "без выбранных досок диаграмма показывает только свои задачи");
    let rows = (genv.cards)(&[board.clone()]);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].start, rows[0].end, rows[0].name.as_str()), (today + 3, today + 3, "Импорт"));
    (genv.set_card_span)(&board, &rows[0].card, today, today + 1);
    assert_eq!(card(ctx).span_text(), format!("{} → {}", iso(0), iso(1)));

    // Часовая полоса: время попадает в отрезок дневной сетки.
    call(ctx, "kanban", serde_json::json!({"op": "add_card", "board": &board, "column": "Бэклог", "title": "Созвон"}));
    call(
        ctx,
        "kanban",
        serde_json::json!({"op": "schedule", "board": &board, "card": "Созвон", "date": "today", "time": "10:00", "duration": "2h"}),
    );
    let timed = (env.external)(&q);
    let call_item = timed.iter().find(|i| i.title == "Созвон").expect("полоса созвона");
    assert_eq!((call_item.day, call_item.time), (today, Some((600, 720))));

    // agenda: задача со start = сегодня в «Сегодня», хотя срока нет.
    let out = call(ctx, "agenda", serde_json::json!({}));
    assert!(out.contains("Today (2):") && out.contains("\"Импорт\"") && out.contains("\"Созвон\""), "{out}");

    // Снятие плана: полоса исчезает, карточка остаётся.
    call(ctx, "kanban", serde_json::json!({"op": "unschedule", "board": &board, "card": "Созвон"}));
    assert_eq!((env.external)(&q).len(), 1, "осталась только полоса «Импорта»");

    // Доска, встроенная ещё и на второй странице, считается один раз —
    // раньше каждая врезка добавляла в календарь свой экземпляр карточки.
    call(ctx, "create", serde_json::json!({"title": "Обзор", "content": format!("Доска спринта:\n\n![[kanban:{board}]]\n")}));
    let again = (env.external)(&q);
    assert_eq!(again.len(), 1, "полоса не задвоилась: {again:?}");
}

/// Ведение жизни: колонка «готово» по названию, штампы и повтор при
/// закрытии, журнал с актором, agenda/tasks по всем доскам, архив,
/// страница дня, вложения карточки, сроки в календаре по умолчанию.
#[test]
fn life_management_through_the_tool() {
    let ctx = ctx();
    let today = today_days();
    let iso = |d: i64| days_to_iso(today + d);
    let page = page_id(&call(ctx, "create", serde_json::json!({"title": "Работа"})));
    let out = call(ctx, "kanban", serde_json::json!({"op": "create", "page": &page, "columns": ["Бэклог", "В работе", "Готово"]}));
    assert!(out.contains("\"Готово\" · DONE column"), "колонка «Готово» узнаётся по названию: {out}");
    let board = out.lines().find_map(|l| l.strip_prefix("kanban:")).unwrap().split(' ').next().unwrap().to_string();
    let add = |title: &str, extra: serde_json::Value| {
        let mut args = serde_json::json!({"op": "add_card", "board": &board, "column": "Бэклог", "title": title});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        call(ctx, "kanban", args)
    };
    let out = add("Импорт", serde_json::json!({"due": iso(-3), "tags": "core"}));
    assert!(out.contains(&format!("created: {}", iso(0))), "штамп создания: {out}");
    add("Отчёт", serde_json::json!({"due": "today"}));
    add("Звонок", serde_json::json!({"due": "tomorrow", "priority": "high"}));
    add("План", serde_json::json!({"due": iso(5)}));
    add("Идея", serde_json::json!({"priority": "urgent"}));
    let out = add("Привычка", serde_json::json!({"due": "today", "repeat": "daily", "md": "- [x] шаг"}));
    assert!(out.contains("repeat: daily"), "{out}");
    call(ctx, "calendar", serde_json::json!({"op": "add_event", "title": "Стендап", "date": iso(1), "start_time": "10:00", "end_time": "10:30"}));

    // agenda: одним вызовом — просрочено, сегодня, завтра, скоро, важное без срока, события, доски.
    let out = call(ctx, "agenda", serde_json::json!({}));
    assert!(out.contains("Overdue (1):") && out.contains("\"Импорт\"") && out.contains("3 days late"), "{out}");
    assert!(out.contains("Today (2):") && out.contains("Tomorrow (1):") && out.contains("Next 7 days (1):"), "{out}");
    assert!(out.contains("Important without a due date (1):") && out.contains("\"Идея\""), "{out}");
    assert!(out.contains("\"Стендап\"") && out.contains("10:00–10:30"), "{out}");
    assert!(out.contains("Boards:") && out.contains("Бэклог 6") && out.contains("Готово ✓ 0"), "{out}");

    // tasks: фильтры.
    let out = call(ctx, "tasks", serde_json::json!({"due": "overdue"}));
    assert!(out.contains("1 card") && out.contains("\"Импорт\"") && !out.contains("\"Отчёт\""), "{out}");
    let out = call(ctx, "tasks", serde_json::json!({"tag": "core"}));
    assert!(out.contains("\"Импорт\"") && out.contains("1 card"), "{out}");
    let out = call(ctx, "tasks", serde_json::json!({"priority": "urgent", "due": "none"}));
    assert!(out.contains("\"Идея\"") && out.contains("1 card"), "{out}");
    let out = call(ctx, "tasks", serde_json::json!({"due": "week", "sort": "priority"}));
    let pos = |t: &str| out.find(t).unwrap_or(usize::MAX);
    assert!(pos("\"Звонок\"") < pos("\"Отчёт\""), "high раньше без приоритета: {out}");

    // Закрытие: перенос в «Готово» — штамп done и следующая карточка повтора; agenda видит «сделано».
    let out = call(ctx, "kanban", serde_json::json!({"op": "move_card", "board": &board, "card": "Привычка", "column": "Готово"}));
    assert!(out.contains(&format!("done: {}", iso(0))), "{out}");
    let doc = match ctx.object("kanban", &board) { Some(LiveObject::Kanban { handle, .. }) => handle.lock().clone(), _ => panic!() };
    let habits: Vec<&KanbanCard> = doc.cards.iter().filter(|c| c.title == "Привычка").collect();
    assert_eq!(habits.len(), 2, "{doc:?}");
    let next = habits.iter().find(|c| c.done.is_none()).unwrap();
    assert_eq!((next.due.as_deref(), next.md.as_str(), doc.column_name(&next.column).as_str()), (Some(iso(1).as_str()), "- [ ] шаг", "Бэклог"));
    let out = call(ctx, "agenda", serde_json::json!({}));
    assert!(out.contains("Done in the last 3 days (1):"), "{out}");
    let out = call(ctx, "tasks", serde_json::json!({"done": true}));
    assert!(out.contains("1 card") && out.contains("done: "), "{out}");
    let out = call(ctx, "tasks", serde_json::json!({"done": "any", "query": "привыч"}));
    assert!(out.contains("2 cards"), "{out}");

    // Журнал: добавления, DONE, повтор; актор — user вне агентского скоупа, agent внутри.
    let out = call(ctx, "log", serde_json::json!({"since": "today"}));
    assert!(out.contains("card \"Привычка\" DONE (\"Бэклог\" → \"Готово\")"), "{out}");
    assert!(out.contains("next repeat created") && out.contains("card \"Импорт\" added to \"Бэклог\""), "{out}");
    assert!(out.contains("event \"Стендап\" added on") && out.contains("page \"Работа\" created"), "{out}");
    assert!(out.lines().filter(|l| l.contains(" · user · ")).count() > 5 && !out.contains(" · agent · "), "{out}");
    {
        let _agent = crate::pages::notes::activity::agent_scope();
        call(ctx, "kanban", serde_json::json!({"op": "update_card", "board": &board, "card": "План", "due": iso(6)}));
    }
    let out = call(ctx, "log", serde_json::json!({"since": "today", "actor": "agent"}));
    assert!(out.contains("1 entry") && out.contains(&format!("card \"План\" due {} → {}", iso(5), iso(6))), "{out}");
    let out = call(ctx, "log", serde_json::json!({"since": "today", "kind": "event"}));
    assert!(out.contains("1 entry"), "{out}");
    let out = call(ctx, "log", serde_json::json!({"since": "today", "op": "done", "board": &board}));
    assert!(out.contains("1 entry") && out.contains("DONE"), "{out}");
    assert!(dispatch(ctx, "log", &serde_json::json!({"since": "позавчера"})).is_err());

    // Архив: archive_after=0 уносит закрытые сразу; read archived=true; unarchive по названию.
    let out = call(ctx, "kanban", serde_json::json!({"op": "set_style", "board": &board, "archive_after": 0}));
    assert!(out.contains("archive_after off"), "{out}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "set_style", "board": &board, "archive_after": "1"}));
    assert!(out.contains("archive_after 1 days"), "{out}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "archive", "board": &board, "card": "Отчёт"}));
    assert!(out.contains("archived card \"Отчёт\"") && out.contains("archived: 1"), "{out}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "read", "board": &board, "archived": true}));
    assert!(out.contains("--- Archive (1) ---") && out.contains("\"Отчёт\""), "{out}");
    let out = call(ctx, "tasks", serde_json::json!({"done": "any", "archived": true, "query": "отчёт"}));
    assert!(out.contains("· archived"), "{out}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "unarchive", "board": &board, "card": "Отчёт"}));
    assert!(out.contains("restored") && out.contains("archived: 0") || !out.contains("archived:"), "{out}");
    let doc = match ctx.object("kanban", &board) { Some(LiveObject::Kanban { handle, .. }) => handle.lock().clone(), _ => panic!() };
    let report = doc.cards.iter().find(|c| c.title == "Отчёт").unwrap();
    assert!(doc.is_done_column(&report.column) && report.done.is_some());
    let out = call(ctx, "log", serde_json::json!({"since": "today", "card": "Отчёт", "board": &board}));
    assert!(out.contains("archived (from") && out.contains("restored to \"Готово\""), "{out}");

    // Флаг колонки: снять и поставить.
    let out = call(ctx, "kanban", serde_json::json!({"op": "update_column", "board": &board, "column": "Готово", "done": false}));
    assert!(out.contains("!! no done column"), "{out}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "add_column", "board": &board, "name": "Сделано", "done": true}));
    assert!(out.contains("\"Сделано\" · DONE column") && !out.contains("!! no done column"), "{out}");

    // Страница дня.
    let out = call(ctx, "journal", serde_json::json!({"date": "2026-09-08", "content": "- сделал импорт"}));
    assert!(out.contains("journal page for 2026-09-08 (Tue) · created") && out.contains("1 block appended"), "{out}");
    assert!(out.contains("- сделал импорт"), "{out}");
    let day = page_id(&out);
    let out = call(ctx, "journal", serde_json::json!({"date": "2026-09-08"}));
    assert!(!out.contains("· created") && page_id(&out) == day, "{out}");
    let tree = ctx.tree.get_untracked();
    let path: Vec<String> = tree.path_of(&day).into_iter().map(|(_, t)| t).collect();
    assert_eq!(path, ["Журнал", "2026-09", "2026-09-08"]);
    assert_eq!(ctx.find_journal_page(parse_days("2026-09-08").unwrap()), Some(day.clone()));
    assert!(ctx.find_journal_page(parse_days("2026-09-09").unwrap()).is_none());
    assert_eq!(crate::pages::notes::state::date_title("2026-09-08"), parse_days("2026-09-08"));
    assert!(crate::pages::notes::state::date_title("Заметка").is_none());

    // Вложения: файл с диска → бандл; картинка — миниатюра, файл — скрепка; detach по имени.
    let dir = std::env::temp_dir().join(format!("synthos-notes-attach-{}", project::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let pic = dir.join("фото.png");
    std::fs::write(&pic, b"\x89PNG\r\n\x1a\nfake").unwrap();
    let doc_file = dir.join("договор.pdf");
    std::fs::write(&doc_file, b"%PDF-1.4 fake").unwrap();
    let out = call(ctx, "kanban", serde_json::json!({"op": "attach", "board": &board, "card": "Импорт", "path": pic.display().to_string()}));
    assert!(out.contains("as image (thumbnail on the card) \"фото.png\"") && out.contains("files: фото.png"), "{out}");
    let out = call(ctx, "kanban", serde_json::json!({"op": "attach", "board": &board, "card": "Импорт", "path": doc_file.display().to_string(), "name": "Договор"}));
    assert!(out.contains("as file (paperclip on the card) \"Договор\"") && out.contains("files: фото.png, Договор"), "{out}");
    let card = match ctx.object("kanban", &board) { Some(LiveObject::Kanban { handle, .. }) => handle.lock().cards.iter().find(|c| c.title == "Импорт").cloned().unwrap(), _ => panic!() };
    assert_eq!(card.files.len(), 2);
    assert!(card.files[0].is_image() && !card.files[1].is_image());
    assert!(crate::pages::notes::media::asset_file(&ctx.project_path.get_untracked(), &card.files[0].url).is_some_and(|p| p.is_file()));
    let out = call(ctx, "kanban", serde_json::json!({"op": "detach", "board": &board, "card": "Импорт", "file": "договор"}));
    assert!(out.contains("removed attachment") && !out.contains("Договор"), "{out}");
    assert!(dispatch(ctx, "kanban", &serde_json::json!({"op": "detach", "board": &board, "card": "Импорт", "file": "нет"})).is_err());

    // Календарь: сроки досок в выдаче по умолчанию, с пометкой «не событие».
    let out = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": iso(-3), "to": iso(7)}));
    assert!(out.contains("external:") && out.contains("\"Импорт\"") && out.contains("not an event"), "{out}");
    let out = call(ctx, "calendar", serde_json::json!({"op": "list_events", "from": iso(-3), "to": iso(7), "include_external": false}));
    assert!(!out.contains("external:"), "{out}");
}

/// Шапка `now:`: локальные дата, время и день недели на момент вызова —
/// системный промпт даёт агенту только дату, а срокам, событиям и штампам
/// журнала нужны часы.
#[test]
fn now_line_has_local_date_time_and_weekday() {
    crate::agent::time::override_offset_secs(Some(5 * 3600));
    let line = now_line();
    crate::agent::time::override_offset_secs(None);

    let rest = line.strip_prefix("now: ").expect("шапка now: {line}");
    let rest = rest.strip_suffix(" (local)\n").expect("пометка (local): {line}");
    let mut parts = rest.split(' ');
    let (date, time, weekday) = (
        parts.next().expect("дата"),
        parts.next().expect("время"),
        parts.next().expect("день недели"),
    );
    assert_eq!(parts.next(), None, "{line}");
    assert!(
        date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-',
        "{line}"
    );
    let (h, m) = time.split_once(':').expect("HH:MM");
    assert!(h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60), "{line}");
    assert!(life::WEEKDAYS.contains(&weekday), "{line}");

    // Дата в шапке — тот же день, что и у остальных действий инструмента.
    assert!(line.contains(&days_to_iso(today_days())), "{line}");
}

/// Описание инструмента объясняет, откуда брать время: без этого агент
/// шапку в ответе просто не замечает.
#[test]
fn tool_description_points_at_the_now_line() {
    let t = crate::agent::tools::descriptor::Tool::by_key("notes").expect("notes зарегистрирован");
    assert!(t.description.contains("`now:` line"), "{}", t.description);
}
