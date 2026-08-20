//! Каталог инструментов — чистые описания, без логики исполнения.

use serde_json::json;

use crate::icons::{
    MI_ACCOUNT_TREE, MI_BOLT, MI_MEMORY, MI_PSYCHOLOGY, MI_SEARCH, MI_TERMINAL, MI_TRAVEL_EXPLORE,
};

use super::descriptor::Tool;

/// Ключи инструментов — чтобы не дублировать литералы в executor/UI.
pub const KEY_BASH: &str = "bash";
pub const KEY_KB_SEARCH: &str = "kb_search";
pub const KEY_WEB: &str = "web";
pub const KEY_AUTOSKILL: &str = "autoskill";
pub const KEY_SUBAGENT: &str = "subagent";
pub const KEY_SYSTEM: &str = "system";
pub const KEY_PIPELINES: &str = "pipelines";

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
            key: KEY_PIPELINES,
            label: "pipelines",
            icon: MI_ACCOUNT_TREE,
            description: "Нодовые пайплайны генерации (видео LTX/MiniMax-H3, \
                музыка ACE-Step, TTS, ASR, LLM-нода) в служебной вкладке \
                редактора — одна на чат, пользователь может открыть её из \
                чата и наблюдать. Типовой workflow: 1) action=list — шаблоны \
                (встроенные и кастомные) и вложения чата; 2) action=open с \
                template=<id> — загрузить шаблон; 3) action=nodes с \
                filter=<kind> — точная схема нод (порты + пример state-JSON); \
                4) action=apply — заполнить: set_state=[{node,state}] ставит \
                промпты/параметры/пути, connect/disconnect правят связи, \
                graph (mode=replace|merge) заменяет/дополняет граф целиком \
                Template-JSON'ом; 5) action=run — запустить прогон (перед \
                тяжёлым прогоном проверь system status; free_vram=true \
                выгрузит чат-LLM на время прогона и вернёт после). \
                В строках state работает ссылка attachment:<имя|sha|last> — \
                подставляет путь вложения из чата (например ref-картинка в \
                LtxImage.image_path). action=graph — снимок графа; \
                action=save_template с name — сохранить граф пользователю \
                как кастомный шаблон.",
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "nodes", "open", "graph", "apply", "save_template", "run"],
                        "description": "Что сделать (см. описание инструмента)."
                    },
                    "template": {
                        "type": "string",
                        "description": "action=open: id шаблона из action=list."
                    },
                    "filter": {
                        "type": "string",
                        "description": "action=nodes: подстрока по kind/названию/категории. \
                            Без filter — компактный список всех видов нод."
                    },
                    "graph": {
                        "type": "object",
                        "description": "action=apply: Template-JSON {nodes:[{id?,kind,pos?,state?,enabled?}], \
                            connections:[{from_node,from_port,to_node,to_port}]}. \
                            id и pos можно опустить — проставятся автоматически."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "merge"],
                        "description": "action=apply с graph: replace — заменить граф (по умолчанию), \
                            merge — дописать к существующему (id нод переназначатся)."
                    },
                    "set_state": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "action=apply: [{node:<id>, state:{kind,data}}] — \
                            state-JSON смотри в action=nodes (пример с дефолтами) \
                            или action=graph (текущие значения)."
                    },
                    "connect": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "action=apply: [{from_node,from_port,to_node,to_port}] — добавить связи."
                    },
                    "disconnect": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "action=apply: [{from_node,from_port,to_node,to_port}] — убрать связи."
                    },
                    "name": {
                        "type": "string",
                        "description": "action=save_template: имя нового кастомного шаблона."
                    },
                    "description": {
                        "type": "string",
                        "description": "action=save_template: описание шаблона (опционально)."
                    },
                    "free_vram": {
                        "type": "boolean",
                        "description": "action=run: выгрузить чат-LLM на время прогона \
                            (нужно тяжёлым пайплайнам — LTX/H3 на 24 ГБ рядом с LLM не влезут) \
                            и загрузить обратно после."
                    },
                    "run_id": {
                        "type": "string",
                        "description": "action=run: метка запуска; при повторном запуске \
                            того же графа передай НОВОЕ значение (например run-2)."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_SYSTEM,
            label: "system",
            icon: MI_MEMORY,
            description: "Состояние системы и памяти. action=status — VRAM \
                (всего / свободно / доступно с учётом пулов), RAM, список \
                моделей в памяти: нодовые (с id для выгрузки) и чат-LLM. \
                action=unload — выгрузить нодовые модели из VRAM (по id из \
                status или все сразу с all=true). ВЫЗЫВАЙ status перед \
                запуском тяжёлого пайплайна, чтобы решить, хватает ли VRAM \
                и что выгрузить. Чат-LLM (та, на которой работаешь ты) этим \
                инструментом не выгружается — используй free_vram у \
                pipelines action=run.",
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "unload"],
                        "description": "status — снимок VRAM/RAM/моделей; \
                            unload — выгрузка нодовых моделей."
                    },
                    "id": {
                        "type": "integer",
                        "description": "Только для action=unload: id модели \
                            из вывода status."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "Только для action=unload: true — \
                            выгрузить все нодовые модели."
                    }
                },
                "required": ["action"],
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
