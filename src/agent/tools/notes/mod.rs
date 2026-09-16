//! Tool `notes` — полный доступ агента Syn-чата к режиму «Заметки».
//!
//! Агент работает с тем же проектом `.syn` и теми же ручками, что и UI
//! (`NotesCtx`): дерево страниц, markdown страниц, канбан-доски и диаграммы
//! Ганта, вложения. Правки видны в интерфейсе сразу (перестройка по
//! `doc_epoch`/`revision`) и уходят на диск автосейвом; замена текста
//! страницы записывается в историю редактора — Ctrl+Z у пользователя
//! возвращает прежний текст.
//!
//! Действия:
//! - `list` — дерево страниц (id, название, объём, объекты на странице);
//!   `search` — поиск по названиям и тексту; `read` — страница целиком
//!   (markdown, доски и диаграммы на ней, связи) или сразу пачка:
//!   `pages` (список), `depth` (подстраницы), `page="all"` (весь проект)
//!   — сколько влезает в живой бюджет контекста ([`ReadBudget`]),
//!   остальное перечислено по id.
//!   `blocks op=read` так же берёт `all` / «0,2,5-7» / массив.
//! - `create` / `update` / `move` / `delete` / `duplicate` — страницы.
//!   `update` умеет переименовать, сменить иконку и раскладку, заменить
//!   текст целиком (`content`, `mode=replace|append|prepend`) и точечно
//!   (`find`/`replace`).
//! - `open` — показать страницу пользователю; `attach` — файл с диска или
//!   вложение чата → вложение проекта + медиа-блок на странице.
//! - `kanban` / `gantt` — объекты-примитивы: создать на странице, прочитать,
//!   колонки/карточки и задачи/зависимости, стиль доски и масштаб
//!   диаграммы, удалить (врезки убираются со страниц, файл — из бандла).
//! - `chart` — график (линии, столбцы, круговая, радар, шкала): создать на
//!   странице или из блока-таблицы, прочитать данными, менять вид, подписи,
//!   ряды и оформление, удалить.
//! - `blocks` — блоки страницы как структура: список с индексами,
//!   геометрией и атрибутами; вставка, замена, перенос, удаление одного
//!   блока; любые атрибуты (стиль текста, координаты и размеры, параметры
//!   фигур); закрепление на холсте и снятие с него.
//! - `shape` — примитивы: создать фигуру или линию (концы — в абсолютных
//!   координатах холста), изменить, удалить, `connect` — стрелка между
//!   двумя закреплёнными блоками.
//!
//! Адресация: страница — id (12 hex) либо название (без регистра; при
//! совпадениях — путь «Родитель / Страница» или id); доска/диаграмма — id
//! объекта либо страница, на которой объект один; колонка — id или
//! название; карточка/задача — id или заголовок; блок — индекс верхнего
//! уровня из `blocks op=list` либо `find:<фрагмент>` с единственным
//! вхождением.
//!
//! **Служебный хвост.** Агент видит и правит «плоский» markdown без хвоста
//! ```` ```doc-layout ````: там по индексу блока лежат координаты и —
//! у блоков без места под инлайн-атрибуты (абзац, списки, код, таблица)
//! — все их свойства. При записи блоки, чей markdown не изменился, получают
//! свои прежние атрибуты ([`with_sidecar`]) — перестановка абзаца агентом
//! не сбивает ни холст, ни оформление. Точечные правки (`find`/`replace`,
//! `blocks`) идут по модели документа и атрибуты блока не теряют.
//!
//! Все сигналы — main-thread: действие целиком исполняется в
//! `run_on_main_thread`-замыкании, результат уходит через oneshot
//! (паттерн `pipelines`). Формат результата — секционный plain-text с
//! шапкой `now:` ([`now_line`]) — локальными датой и временем на момент
//! вызова.
//!
//! **Устройство модуля.** Здесь только точка входа [`run`] и диспетчер
//! [`dispatch`]; общие `use` этого файла подмодули берут через
//! `use super::*`. Действия разложены по файлам:
//! [`pages`] (`list`/`search`/`read` и жизненный цикл страниц),
//! [`blocks`], [`shapes`] (`shape`), [`kanban`], [`gantt`], [`mindmap`],
//! [`calendar`], [`chart`], [`life`] (`agenda`/`tasks`/`log`/`journal`).
//! Общее — в утилитах: [`args`] (разбор полей JSON), [`refs`] (адресация
//! страниц и объектов), [`md`] (плоский markdown и служебный хвост),
//! [`geom`] (геометрия блоков и раскладка колонкой), [`attrs`] (значения
//! атрибутов блока).

use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;
use syngui::tr;
use syngui::widgets::input::document_editor::attrs::{parse_attr_block, serialize_attrs};
use syngui::widgets::input::document_editor::serialize::block_markdown;
use syngui::widgets::input::document_editor::{
    free, parse_document, props, serialize_document, shape, Attrs, BlockKind, DocBlock, DocModel, ShapeKind,
};

use crate::pages::notes::gantt::calendar::{days_to_iso, parse_days, today_days};
use crate::pages::notes::gantt::model::GanttDoc;
use crate::pages::notes::gantt::GanttHandle;
use crate::pages::notes::kanban::model::{
    fmt_duration, item_id, parse_duration, parse_tags, DropSpot, KanbanCard, KanbanColumn, Moment, Priority, PALETTE,
};
use crate::pages::notes::kanban::KanbanHandle;
use crate::pages::notes::calendar::model::{
    fmt_hm, parse_hm, CalEvent, CalView, CalendarStore, CalendarStyle, EventStyle, Repeat, Weekdays,
};
use crate::pages::notes::calendar::{CalendarHandle, CalendarStoreHandle, ExternalQuery};
use crate::pages::notes::chart::model::{self as chart_model, ChartDoc, ChartKind, GaugeZone, LegendPos, PieLabels};
use crate::pages::notes::chart::ChartHandle;
use crate::pages::notes::mindmap::model::{Curve, Direction, MindmapDoc, NodeShape};
use crate::pages::notes::mindmap::MindmapHandle;
use crate::pages::notes::project::{PageGrid, PageLayout};
use crate::pages::notes::state::{object_refs, LiveObject, NotesCtx};
use crate::pages::notes::{embeds, media};
use crate::syn_chat::attach::blobs;

use super::budget;
use super::executor::{ToolError, MAX_NOTES_OUTPUT_BYTES};

mod args;
mod attrs;
mod blocks;
mod calendar;
mod chart;
mod gantt;
mod geom;
mod kanban;
mod life;
mod md;
mod mindmap;
mod pages;
mod refs;
mod shapes;

#[cfg(test)]
mod tests;

use args::*;
use attrs::*;
use blocks::*;
use calendar::*;
use chart::*;
use gantt::*;
use geom::*;
use kanban::*;
use md::*;
use mindmap::*;
use pages::*;
use refs::*;
use shapes::*;

/// Главный entrypoint из `executor::execute`.
pub async fn run(args_json: &str) -> Result<String, ToolError> {
    let v: Json = serde_json::from_str(args_json).map_err(|e| ToolError::BadArgs(e.to_string()))?;
    let action = str_field(&v, "action")
        .ok_or(ToolError::MissingField("action"))?
        .to_string();

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    run_on_main_thread(move || {
        let ctx = use_context::<NotesCtx>();
        // Агент работает с активным проектом; если ни один не открыт —
        // открывается недавний (или дефолтный) и появляется его плитка.
        if let Err(e) = ctx.ensure_project() {
            let _ = tx.send(Err(format!("no notes project is open and none could be opened: {}", e.message())));
            return;
        }
        // Правки от имени агента — так они помечены в журнале проекта.
        let _agent = crate::pages::notes::activity::agent_scope();
        let _ = tx.send(dispatch(ctx, &action, &v));
    });
    rx.await
        .map_err(|e| ToolError::Spawn(e.to_string()))?
        .map(|out| format!("{}{out}", now_line()))
        .map_err(ToolError::Args)
}

/// Шапка ответа: локальные дата, время и день недели на момент вызова.
///
/// Системный промпт даёт агенту только дату: часы в его голове меняли бы
/// префикс контекста каждую минуту и обнуляли переиспользование
/// префикс-KV (см. `syn_chat::system_prompt`). Заметкам же время нужно
/// постоянно — сроки и напоминания карточек, события календаря, штампы в
/// журнале дня, — и здесь оно достаётся даром: результат вызова уходит в
/// хвост контекста, где ничего не кэшируется.
pub(super) fn now_line() -> String {
    let day = crate::agent::time::local_today_days();
    format!(
        "now: {} {} (local)\n",
        crate::agent::time::format_now(),
        life::WEEKDAYS[crate::pages::notes::gantt::calendar::weekday_of(day) as usize]
    )
}

/// Диспетчер действий; вынесен из `run`, чтобы тесты звали его с
/// собственным `NotesCtx` без main-thread.
pub fn dispatch(ctx: NotesCtx, action: &str, v: &Json) -> Result<String, String> {
    match action {
        "list" => list_impl(ctx),
        "search" => search_impl(ctx, v),
        "read" => read_impl(ctx, v),
        "create" => create_impl(ctx, v),
        "update" => update_impl(ctx, v),
        "move" => move_impl(ctx, v),
        "delete" => delete_impl(ctx, v),
        "duplicate" => duplicate_impl(ctx, v),
        "open" => open_impl(ctx, v),
        "attach" => attach_impl(ctx, v),
        "kanban" => kanban_impl(ctx, v),
        "gantt" => gantt_impl(ctx, v),
        "blocks" => blocks_impl(ctx, v),
        "shape" => shape_impl(ctx, v),
        "mindmap" => mindmap_impl(ctx, v),
        "calendar" => calendar_impl(ctx, v),
        "chart" => chart_impl(ctx, v),
        "agenda" => life::agenda_impl(ctx, v),
        "tasks" => life::tasks_impl(ctx, v),
        "log" => life::log_impl(ctx, v),
        "journal" => life::journal_impl(ctx, v),
        other => Err(format!(
            "unknown action \"{other}\" (list | search | read | create | update | move | delete | \
             duplicate | open | attach | blocks | shape | kanban | gantt | mindmap | calendar | chart | \
             agenda | tasks | log | journal)"
        )),
    }
}
