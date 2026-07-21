//! Каталог инструментов — чистые описания, без логики исполнения.

use serde_json::json;

use crate::icons::{MI_BOLT, MI_PSYCHOLOGY, MI_SEARCH, MI_TERMINAL, MI_TRAVEL_EXPLORE};

use super::descriptor::Tool;

/// Ключи инструментов — чтобы не дублировать литералы в executor/UI.
pub const KEY_BASH: &str = "bash";
pub const KEY_KB_SEARCH: &str = "kb_search";
pub const KEY_WEB: &str = "web";
pub const KEY_AUTOSKILL: &str = "autoskill";
pub const KEY_SUBAGENT: &str = "subagent";

/// Строит полный список известных инструментов. Вызывается один раз
/// (кэш в `Tool::all()` через `OnceLock`).
pub(super) fn build_all() -> Vec<Tool> {
    vec![
        Tool {
            key: KEY_BASH,
            label: "bash",
            icon: MI_TERMINAL,
            description: "Выполнить shell-команду через bash -lc. \
                Возвращает stdout, stderr и exit-код. \
                Используй только для быстрых, детерминированных команд.",
            schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell-команда для bash -lc. \
                            Одна строка; если нужны пайпы, экранируй кавычками."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_KB_SEARCH,
            label: "kb_search",
            icon: MI_SEARCH,
            description: "Найти релевантные фрагменты в активных базах знаний \
                (RAG hybrid-search: BM25 + cosine). Возвращает топ-K фрагментов \
                с цитированием источника. Используй когда нужны конкретные \
                факты из документов пользователя; не используй для генеральных \
                вопросов, которые модель и так знает.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Поисковый запрос на естественном языке."
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Сколько фрагментов вернуть (1..=20). По умолчанию 5.",
                        "minimum": 1,
                        "maximum": 20
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_WEB,
            label: "web",
            icon: MI_TRAVEL_EXPLORE,
            description: "Поиск и чтение страниц в интернете в реальном \
                времени. ВЫЗЫВАЙ этот tool всегда, когда вопрос требует \
                актуальных или свежих данных, которых не может быть в твоих \
                весах: погода, курсы валют и крипты, котировки и цены, \
                новости и события, спортивные результаты, расписания \
                (рейсы, поезда, кино), статус сервисов, релизы ПО, \
                чарты, таблицы лиг, выборы, факты после твоего knowledge \
                cutoff, а также любые «сейчас / сегодня / на этой неделе». \
                НЕ отказывай со ссылкой на cutoff и не отправляй \
                пользователя на сторонние сайты — у тебя есть `web`, \
                сходи и принеси ответ. \
                action=search — поиск через DuckDuckGo, возвращает топ-N \
                результатов (title, url, snippet); используй короткий \
                запрос с местом/датой/единицами (\"погода Алматы сегодня\", \
                \"USD KZT курс\", \"Bitcoin price USD\"). \
                action=read — загрузка страницы по URL и извлечение \
                article-контента в Markdown (Mozilla Readability + htmd; \
                nav/footer/реклама фильтруются автоматически). \
                Типовой workflow: сначала action=search, затем action=read \
                для 1-2 самых релевантных URL из выдачи. Не угадывай URL — \
                всегда сначала search.",
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["search", "read"],
                        "description": "Что делать: `search` — искать в DDG; \
                            `read` — прочитать конкретный URL и вернуть Markdown."
                    },
                    "query": {
                        "type": "string",
                        "description": "Обязателен при action=search. \
                            Поисковый запрос на естественном языке."
                    },
                    "url": {
                        "type": "string",
                        "description": "Обязателен при action=read. \
                            HTTP(S) URL страницы для чтения."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Только для action=search. \
                            Сколько результатов вернуть (1..=20). По умолчанию 10.",
                        "minimum": 1,
                        "maximum": 20,
                        "default": 10
                    },
                    "lang": {
                        "type": "string",
                        "description": "Только для action=search. \
                            Язык предпочтений (например \"ru\", \"en\"). По умолчанию \"ru\".",
                        "default": "ru"
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_AUTOSKILL,
            label: "autoskill",
            icon: MI_PSYCHOLOGY,
            description: "Получить markdown-инструкцию (skill) пользователя по её id. \
                Полный список доступных skill'ов (id и краткое описание) подставляется \
                в это описание динамически при каждом запросе. Когда вопрос пользователя \
                ложится на тематику одного из skill'ов — вызови `autoskill` с её id, \
                получи markdown и следуй инструкции.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Slug-id скила (см. список в описании этого tool'а)."
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_SUBAGENT,
            label: "subagent",
            icon: MI_BOLT,
            description: "Делегировать длинную исследовательскую/локальную задачу \
                вложенному агенту, чтобы не раздувать твой контекст. Субагент \
                запускает свой собственный цикл со своим набором tools, выполняет \
                нужные действия и возвращает ТОЛЬКО краткий итог. \
                ВЫЗЫВАЙ subagent, когда: 1) нужно прочитать длинную веб-страницу \
                или серию страниц и извлечь 1-3 факта; 2) нужно обойти проект \
                несколькими `bash find/grep`; 3) задача требует 2+ tool-вызовов \
                и ты не хочешь занимать ими свой контекст. \
                ВАЖНО ПРО `task`: передавай ТОЛЬКО узкую подзадачу. Не \
                копируй весь user-диалог, не пересказывай свою глобальную \
                цель — у субагента есть жёсткий бюджет (24 tool-вызовов), \
                после чего он принудительно сдаёт текстовый итог. \
                Хорошо: «прочти https://… и верни список упомянутых API \
                в 1 абзаце». Плохо: «помоги сделать приложение X». \
                Вложенный subagent (subagent внутри subagent) запрещён. \
                У субагента нет доступа к твоей истории — формулируй задачу \
                самодостаточно. Внутри субагента tool-вызовы исполняются \
                без подтверждения пользователя — пользователь подтверждает \
                только сам вызов subagent, поэтому формулируй task аккуратно. Всегда предпочитай использовать субагентов",
            schema: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "УЗКАЯ подзадача (не вся глобальная \
                            цель пользователя!). Самодостаточная формулировка: \
                            у субагента нет твоего контекста — пиши то, что \
                            ему нужно для конкретно этого шага, без длинной \
                            предыстории. Заверши инструкцией «верни краткий \
                            итог в 1-3 абзаца». Если задача звучит как \
                            «сделай приложение / реализуй фичу» — она слишком \
                            широкая, разбей её сам и зови subagent на \
                            отдельные исследовательские шаги."
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "Опциональный кастомный system-prompt для \
                            субагента. Если не задан — используется системный промпт \
                            пользователя по умолчанию."
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Опциональный список tool-ключей, которые \
                            субагенту разрешено использовать (например \
                            [\"bash\", \"web\"]). Если опущен — все активные \
                            инструменты текущего чата кроме \"subagent\". Не включай \
                            \"subagent\" — вложенный subagent запрещён."
                    }
                },
                "required": ["task"],
                "additionalProperties": false
            }),
        },
    ]
}
