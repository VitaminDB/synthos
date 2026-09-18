//! Material Icons codepoints used by Synthos.
//!
//! Connect the icon font in `run_desktop()` via
//! `.with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)`.

#![allow(dead_code)]

pub const MI_NOTIFICATIONS: &str       = "\u{E7F4}";
pub const MI_ARTICLE: &str             = "\u{EF42}";
pub const MI_ASPECT_RATIO: &str        = "\u{E85B}";
pub const MI_REMOVE_CIRCLE_OUTLINE: &str = "\u{E15D}";
pub const MI_WB_SHADE: &str            = "\u{E43E}";
pub const MI_APPS: &str                = "\u{E5C3}";
pub const MI_CONTACTS: &str            = "\u{E0BA}";
pub const MI_BAR_CHART: &str           = "\u{E26B}";
pub const MI_SETTINGS: &str            = "\u{E8B8}";
pub const MI_HEADSET_MIC: &str         = "\u{E311}";
pub const MI_SEARCH: &str              = "\u{E8B6}";
pub const MI_HELP_OUTLINE: &str        = "\u{E8FD}";
pub const MI_AUTORENEW: &str           = "\u{E863}";
/// Material Icons «compress» — стрелки сжатия, для autocompact-маркера и
/// кнопки «Compact now» в правой панели.
pub const MI_COMPRESS: &str            = "\u{E94D}";
pub const MI_CONTENT_COPY: &str        = "\u{E14D}";
pub const MI_CONTENT_CUT: &str         = "\u{E14E}";
pub const MI_SELECT_ALL: &str          = "\u{E162}";
pub const MI_EXPAND_MORE: &str         = "\u{E5CF}";
pub const MI_EXPAND_LESS: &str         = "\u{E5CE}";
pub const MI_FILTER_LIST: &str         = "\u{E152}";
pub const MI_MORE_HORIZ: &str          = "\u{E5D3}";
pub const MI_REPORT: &str              = "\u{E160}";
pub const MI_DONE_ALL: &str            = "\u{E877}";
pub const MI_EDIT_NOTE: &str           = "\u{E745}";
pub const MI_AUTO_AWESOME: &str        = "\u{E65F}";
pub const MI_ATTACH_FILE: &str         = "\u{E226}";
pub const MI_FORMAT_BOLD: &str         = "\u{E238}";
pub const MI_FORMAT_ITALIC: &str       = "\u{E23F}";
pub const MI_FORMAT_UNDERLINED: &str   = "\u{E249}";
pub const MI_FORMAT_LIST_BULLETED: &str = "\u{E241}";
pub const MI_FORMAT_LIST_NUMBERED: &str = "\u{E242}";
pub const MI_FORMAT_ALIGN_LEFT: &str   = "\u{E236}";
pub const MI_FORMAT_ALIGN_CENTER: &str = "\u{E234}";
pub const MI_FORMAT_ALIGN_RIGHT: &str  = "\u{E237}";
/// Material Icons «format_indent_increase/decrease» — вложить блок в
/// соседа сверху и вынуть обратно (меню блока в заметках).
pub const MI_FORMAT_INDENT_INCREASE: &str = "\u{E23E}";
pub const MI_FORMAT_INDENT_DECREASE: &str = "\u{E23D}";
pub const MI_LAYERS: &str              = "\u{E53B}";
pub const MI_PUSH_PIN: &str            = "\u{F10D}";
pub const MI_TABLE_ROWS: &str          = "\u{F101}";
pub const MI_VIEW_COLUMN: &str         = "\u{E8EC}";
pub const MI_MIC: &str                 = "\u{E029}";
pub const MI_VIDEOCAM: &str            = "\u{E04B}";
pub const MI_SEND: &str                = "\u{E163}";
pub const MI_EMAIL: &str               = "\u{E0BE}";
pub const MI_PHONE: &str               = "\u{E0CD}";
pub const MI_LANGUAGE: &str            = "\u{E894}";
pub const MI_PUBLIC: &str              = "\u{E80B}";
// Globe-with-magnifier — единая иконка для tool `web` (search + read).
pub const MI_TRAVEL_EXPLORE: &str      = "\u{E2C7}";
pub const MI_DESKTOP_WINDOWS: &str     = "\u{E30C}";
pub const MI_ROUTER: &str              = "\u{E328}";
pub const MI_CLOSE: &str               = "\u{E5CD}";
pub const MI_REMOVE: &str              = "\u{E15B}";
pub const MI_CROP_SQUARE: &str         = "\u{E3B6}";
pub const MI_CIRCLE: &str              = "\u{EF4A}";
pub const MI_KEYBOARD_COMMAND_KEY: &str = "\u{EAE7}";
/// «keyboard_return» — клавиша Enter в подсказках панели поиска.
pub const MI_KEYBOARD_RETURN: &str     = "\u{E166}";
/// «arrow_upward» / «arrow_downward» — стрелки навигации по выдаче поиска.
pub const MI_ARROW_UPWARD: &str        = "\u{E5D8}";
/// «arrow_forward» — стрелка «стало» в парах «до → после» (размер после
/// квантования). Символ U+2192 брать нельзя: его нет в шрифте интерфейса,
/// и вместо стрелки выходит пробел.
pub const MI_ARROW_FORWARD: &str       = "\u{E5C8}";
pub const MI_ARROW_DOWNWARD: &str      = "\u{E5DB}";
/// «search_off» — пустая выдача поиска.
pub const MI_SEARCH_OFF: &str          = "\u{EA76}";

// --- Настройки и подстраницы
pub const MI_TUNE: &str                = "\u{E429}";
pub const MI_PALETTE: &str             = "\u{E40A}";
pub const MI_PSYCHOLOGY: &str          = "\u{EA4A}";
pub const MI_GROUPS: &str              = "\u{F233}";
pub const MI_LIGHTBULB: &str           = "\u{E0F0}";
pub const MI_DARK_MODE: &str           = "\u{E51C}";
pub const MI_LIGHT_MODE: &str          = "\u{E518}";
pub const MI_CHECK: &str               = "\u{E5CA}";
pub const MI_CODE: &str                = "\u{E86F}";
pub const MI_BOLT: &str                = "\u{EA0B}";
pub const MI_TRANSLATE: &str           = "\u{E8E2}";
pub const MI_VOLUME_UP: &str           = "\u{E050}";
pub const MI_NOTIFICATIONS_ACTIVE: &str = "\u{E7F7}";
pub const MI_MENU_BOOK: &str           = "\u{EA19}";
pub const MI_CAMPAIGN: &str            = "\u{EF49}";

// --- Раздел «Модели»
// smart_toy (U+EF8A) в MaterialIcons-Regular.ttf отсутствует — глиф
// рисовался пустым местом. Все его места переведены на MI_AUTO_AWESOME.
// Покрытие шрифта проверяет `tests/icons_font_coverage.rs`.
pub const MI_MEMORY: &str              = "\u{E322}";
pub const MI_FOLDER_OPEN: &str         = "\u{E2C8}";
pub const MI_IMAGE_ICON: &str          = "\u{E3F4}";
pub const MI_DELETE: &str              = "\u{E872}";
pub const MI_ADD: &str                 = "\u{E145}";
pub const MI_ADD_CIRCLE: &str          = "\u{E147}";
pub const MI_INFO: &str                = "\u{E88E}";

pub const MI_CHAT: &str                = "\u{E0B7}";
pub const MI_RECORD_VOICE_OVER: &str   = "\u{E63C}";
pub const MI_DOWNLOAD: &str            = "\u{F090}";
pub const MI_CLOUD_DOWNLOAD: &str      = "\u{E2C0}";
pub const MI_FAVORITE: &str            = "\u{E87D}";
pub const MI_TRENDING_UP: &str         = "\u{E8E5}";
pub const MI_VERIFIED_USER: &str       = "\u{E8E8}";
pub const MI_LOCK: &str                = "\u{E897}";

// --- Llama control / Support
pub const MI_PLAY_ARROW: &str          = "\u{E037}";
pub const MI_STOP: &str                = "\u{E047}";
pub const MI_PERSON: &str              = "\u{E7FD}";
pub const MI_DNS: &str                 = "\u{E875}";
pub const MI_LAN: &str                 = "\u{EB2F}";
pub const MI_TERMINAL: &str            = "\u{EB8E}";
pub const MI_BUG_REPORT: &str          = "\u{E868}"; // bug_report — индикатор debug/диагностики
pub const MI_CLEAR_ALL: &str           = "\u{E0B8}";
pub const MI_POWER_SETTINGS: &str      = "\u{E8AC}";

// --- Details / metrics
pub const MI_SPEED: &str               = "\u{E9E4}";
pub const MI_BOLT_FILLED: &str         = "\u{EA0B}";
pub const MI_DEVELOPER_BOARD: &str     = "\u{E30D}";
pub const MI_STORAGE: &str             = "\u{E1DB}";
// monitor_heart (U+F154) в шрифте отсутствует; константа нигде не
// использовалась — удалена, чтобы не всплыла пустым глифом позже.
pub const MI_GRAPHIC_EQ: &str          = "\u{E1B8}";
pub const MI_FILTER_ALT: &str          = "\u{EF4F}"; // filter_alt
pub const MI_BLUR_ON: &str              = "\u{E3A5}"; // blur_on (для Reverb)
pub const MI_HOURGLASS_TOP: &str       = "\u{EF53}";
pub const MI_WARNING_AMBER: &str       = "\u{F083}";
pub const MI_THERMOSTAT: &str          = "\u{F076}";

// --- Code editor (file tree, file types, save)
pub const MI_FOLDER: &str              = "\u{E2C7}";
pub const MI_DESCRIPTION: &str         = "\u{E873}";
pub const MI_SAVE: &str                = "\u{E161}";
pub const MI_SAVE_AS: &str             = "\u{EB60}";
pub const MI_WRAP_TEXT: &str           = "\u{E25B}";
// File-type icons. Используются в [`pages::code_editor::file_icons::icon_for_path`]
// для визуального различения файлов в дереве и списке открытых вкладок.
// Отдельный значок для .rs/.go/.cpp/.py/.ts/.js/.kt/etc — `code` (общий).
// Для языков, где Material Icons имеет специфическую иконку — кладём
// её отдельной константой, fallback на MI_CODE.
pub const MI_DATA_OBJECT: &str         = "\u{EA64}"; // JSON / YAML — data_object
pub const MI_ARCHIVE: &str             = "\u{E149}"; // .zip/.tar/.gz — archive
pub const MI_PICTURE_AS_PDF: &str      = "\u{E415}"; // .pdf — picture_as_pdf
pub const MI_MOVIE: &str               = "\u{E02C}"; // .mp4/.mov/.mkv — movie
pub const MI_AUDIOTRACK: &str          = "\u{E3A1}"; // .mp3/.wav/.flac — audiotrack
pub const MI_FONT_DOWNLOAD: &str       = "\u{E167}"; // .ttf/.otf — font_download
pub const MI_INTEGRATION_INSTRUCTIONS: &str = "\u{EF54}"; // dotfiles, .env — integration_instructions
pub const MI_FOLDER_OPEN_FILLED: &str  = "\u{E2C8}"; // alias к MI_FOLDER_OPEN — раскрытая папка

// --- Code editor (context menu, ops)
pub const MI_DRIVE_FILE_RENAME_OUTLINE: &str = "\u{E9C5}"; // rename
pub const MI_CREATE_NEW_FOLDER: &str   = "\u{E2CC}"; // new folder
pub const MI_NOTE_ADD: &str            = "\u{E89C}"; // new file
pub const MI_LAUNCH: &str              = "\u{E895}"; // reveal in file manager
pub const MI_SYNC_PROBLEM: &str        = "\u{E629}"; // file changed externally — conflict

// --- Агентские инструменты
pub const MI_HANDYMAN: &str            = "\u{F10B}"; // autotools — пул инструментов по запросу
pub const MI_EXTENSION: &str           = "\u{E87B}"; // скилы в левой панели чата — кусочек пазла
/// «token» — карточка «Контекст» правой панели чата: окно в токенах.
pub const MI_TOKEN: &str               = "\u{EA25}";

// --- SynExplorer
/// Material Symbols «inventory_2» — иконка nav-rail страницы SynExplorer
/// (склад моделей-пакетов `.syn`).
pub const MI_INVENTORY_2: &str         = "\u{E1A1}";
/// Preview-таб «Содержимое пакета». Задуманный «deployed_code» (U+F510)
/// в MaterialIcons-Regular.ttf отсутствует, поэтому здесь «token» —
/// четыре квадрата, читается как «составные части пакета».
pub const MI_DEPLOYED_CODE: &str       = "\u{E9B0}";
/// «folder_zip» — карточка `.syn` пакета в списке.
pub const MI_FOLDER_ZIP: &str          = "\u{EB2C}";
/// «book» — преамбула / overview таб.
pub const MI_BOOK: &str                = "\u{E865}";
/// «list_alt» — files таб.
pub const MI_LIST_ALT: &str            = "\u{E0EE}";
/// «edit» — metadata таб.
pub const MI_EDIT: &str                = "\u{E3C9}";
/// «visibility» — preview таб.
pub const MI_VISIBILITY: &str          = "\u{E8F4}";
/// «bookmark_add» — кнопка «добавить закладку».
pub const MI_BOOKMARK_ADD: &str        = "\u{E598}";
/// «refresh» / «autorenew» — alias для reload-кнопки (MI_AUTORENEW уже есть).

// --- Voice FAB / Voice History
pub const MI_PAUSE: &str               = "\u{E034}";
pub const MI_HISTORY: &str             = "\u{E889}";
pub const MI_CONTENT_PASTE: &str       = "\u{E14F}";
/// Material «timer» — секундомер в бейджах таймеров node-editor'а
/// (шапка карточки ноды и Run-pill).
pub const MI_TIMER: &str               = "\u{E425}";

// --- Music / Audio
/// Material Symbols «library_music» — иконка категории Audio в node-editor
/// и кнопок-«в библиотеку» в смежных нодах.
pub const MI_LIBRARY_MUSIC: &str       = "\u{E030}";
/// Material `merge_type` — несколько потоков сходятся в один. Используется
/// в node-editor'е иконкой подменю «Аудио микшеры».
pub const MI_MERGE_TYPE: &str          = "\u{E252}";

// --- Node editor
/// «hub» — иконка nav-rail для редактора нод (центральная точка с ответвлениями).
pub const MI_HUB: &str                 = "\u{E9F4}";
/// «account_tree» — альтернативная иконка для узлового представления.
pub const MI_ACCOUNT_TREE: &str        = "\u{E97A}";
/// «schema» — интеллект-карта.
pub const MI_SCHEMA: &str              = "\u{E4FD}";
/// «subdirectory_arrow_right» — соседний узел карты.
pub const MI_SUBDIRECTORY_ARROW_RIGHT: &str = "\u{E5DA}";
/// «link» — ссылка узла на страницу.
pub const MI_LINK: &str                = "\u{E157}";
/// «zoom_in» / «zoom_out» / «fit_screen» — toolbar редактора нод.
pub const MI_ZOOM_IN: &str             = "\u{E8FF}";
pub const MI_ZOOM_OUT: &str            = "\u{E900}";
pub const MI_FIT_SCREEN: &str          = "\u{EA10}";
pub const MI_GRID_ON: &str             = "\u{E3EC}";
pub const MI_GRID_OFF: &str            = "\u{E3EB}";
/// «content_copy» уже есть выше; для дублирования ноды.
/// «settings_input_component» — иконка для портов / схемы данных.
pub const MI_SETTINGS_INPUT_COMPONENT: &str = "\u{E16A}";

// --- Вложения чата
/// «chevron_left» / «chevron_right» — листание вложений в просмотрщике.
pub const MI_CHEVRON_LEFT: &str        = "\u{E5CB}";
pub const MI_CHEVRON_RIGHT: &str       = "\u{E5CC}";
/// «open_in_new» — открыть вложение системным приложением.
pub const MI_OPEN_IN_NEW: &str         = "\u{E89E}";
/// «insert_drive_file» — универсальная иконка файла-вложения.
pub const MI_INSERT_DRIVE_FILE: &str   = "\u{E24D}";
/// «visibility» уже есть выше; «fullscreen» — раскрыть превью на всё окно.
pub const MI_FULLSCREEN: &str          = "\u{E5D0}";
/// «close_fullscreen» — вернуть оторванный чат на страницу.
pub const MI_CLOSE_FULLSCREEN: &str    = "\u{F1CF}";

// --- Видеоплеер просмотрщика (`components::video_player`)
pub const MI_FULLSCREEN_EXIT: &str     = "\u{E5D1}";
pub const MI_REPLAY: &str              = "\u{E042}";
pub const MI_REPLAY_10: &str           = "\u{E059}";
pub const MI_FORWARD_10: &str          = "\u{E056}";
pub const MI_VOLUME_OFF: &str          = "\u{E04F}";
pub const MI_VOLUME_DOWN: &str         = "\u{E04D}";

// --- Оболочка: плитки рейла, общая шапка, архив
/// «vertical_split» / «view_sidebar» — тогглы левой и правой панели в шапке.
pub const MI_VERTICAL_SPLIT: &str      = "\u{E949}";
pub const MI_VIEW_SIDEBAR: &str        = "\u{F114}";
pub const MI_VIEW_KANBAN: &str         = "\u{EB7F}";
pub const MI_VIEW_TIMELINE: &str       = "\u{EB85}";
pub const MI_TODAY: &str               = "\u{E8DF}";
/// Календарь: год / месяц / неделя / день.
pub const MI_DATE_RANGE: &str          = "\u{E916}";
pub const MI_CALENDAR_MONTH: &str      = "\u{EBCC}";
pub const MI_CALENDAR_VIEW_WEEK: &str  = "\u{EFE8}";
pub const MI_CALENDAR_VIEW_DAY: &str   = "\u{E936}";
/// Планирование карточки: «в календарь» (calendar_add_on), день меню
/// (event), оценка длительности (schedule).
pub const MI_CALENDAR_ADD: &str        = "\u{EFE9}";
pub const MI_EVENT: &str               = "\u{E878}";
pub const MI_SCHEDULE: &str            = "\u{E8B5}";
/// Графики заметок: линии / столбцы / круговая / радар (лучи из центра) /
/// шкала-спидометр. Столбцы — `MI_BAR_CHART`, шкала — `MI_SPEED`, радар —
/// `MI_HUB`.
pub const MI_SHOW_CHART: &str          = "\u{E6E1}";
pub const MI_PIE_CHART: &str           = "\u{E6C4}";
/// Чекбокс события календаря.
pub const MI_CHECK_BOX: &str           = "\u{E834}";
pub const MI_CHECK_BOX_OUTLINE_BLANK: &str = "\u{E835}";
/// «horizontal_rule» — пункт «Разделитель» в меню «+» рейла.
pub const MI_HORIZONTAL_RULE: &str     = "\u{F108}";
/// «dashboard_customize» — открыть окно шаблонов из шапки нод.
pub const MI_DASHBOARD_CUSTOMIZE: &str = "\u{E99B}";
/// «inbox» — раздел «Архив» в настройках; «unarchive» — вернуть чат;
/// «delete_forever» — удалить насовсем; «delete_sweep» — очистить архив.
pub const MI_INBOX: &str               = "\u{E156}";
pub const MI_UNARCHIVE: &str           = "\u{E169}";
pub const MI_DELETE_FOREVER: &str      = "\u{E92B}";
pub const MI_DELETE_SWEEP: &str        = "\u{E16C}";

// Векторные примитивы режима «Заметки» (контекстное меню и панель свойств).
pub const MI_CATEGORY: &str            = "\u{E574}"; // раздел «Примитивы»
pub const MI_CHANGE_HISTORY: &str      = "\u{E86B}"; // треугольник
pub const MI_DIAMOND: &str             = "\u{EAD5}"; // ромб
pub const MI_ARROW_RIGHT_ALT: &str     = "\u{E941}"; // стрелка
pub const MI_COMPARE_ARROWS: &str      = "\u{E0C3}"; // двойная стрелка
pub const MI_LINE_WEIGHT: &str         = "\u{E91A}"; // толщина обводки
pub const MI_LINE_STYLE: &str          = "\u{E919}"; // пунктир
pub const MI_ROUNDED_CORNER: &str      = "\u{E920}"; // скругление
pub const MI_OPACITY: &str             = "\u{E91C}"; // прозрачность
pub const MI_BRUSH: &str               = "\u{E3AE}"; // SVG-файл
pub const MI_FORMAT_COLOR_FILL: &str   = "\u{E23A}"; // заливка
pub const MI_HEIGHT: &str              = "\u{EA16}"; // высота блока
pub const MI_GESTURE: &str             = "\u{E155}"; // кривая Безье
pub const MI_REDO: &str                = "\u{E15A}"; // кривая со стрелкой
pub const MI_UNDO: &str                = "\u{E166}";
pub const MI_SWAP_CALLS: &str          = "\u{E15D}"; // двусторонняя кривая
pub const MI_INSERT_EMOTICON: &str     = "\u{E24E}"; // смайлик — панель эмодзи в чате
pub const MI_VIEW_LIST: &str           = "\u{E8EF}";
pub const MI_GRID_VIEW: &str           = "\u{E9B0}";
pub const MI_IMAGE: &str               = "\u{E3F4}";
