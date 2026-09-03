//! Каталог инструментов — чистые описания, без логики исполнения.

use serde_json::json;

use crate::icons::{
    MI_ACCOUNT_TREE, MI_BOLT, MI_EDIT_NOTE, MI_MEMORY, MI_PSYCHOLOGY, MI_SEARCH, MI_TERMINAL,
    MI_TRAVEL_EXPLORE,
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
pub const KEY_NOTES: &str = "notes";

/// Строит полный список известных инструментов. Вызывается один раз
/// (кэш в `Tool::all()` через `OnceLock`).
pub(super) fn build_all() -> Vec<Tool> {
    vec![
        Tool {
            key: KEY_BASH,
            label: "bash",
            icon: MI_TERMINAL,
            description: "Run a shell command via bash -lc. \
                Returns stdout, stderr, and the exit code. \
                Use only for fast, deterministic commands.",
            schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command for bash -lc. \
                            A single line; if you need pipes, quote them properly."
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
            description: "Find relevant fragments in the active knowledge bases \
                (RAG hybrid search: BM25 + cosine). Returns the top-K fragments \
                with source citations. Use it when you need specific facts \
                from the user's documents; don't use it for general \
                questions the model already knows.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural-language search query."
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "How many fragments to return (1..=20). Default 5.",
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
            description: "Search and read web pages in real time. \
                ALWAYS CALL this tool when the question needs current or \
                fresh data that can't be in your weights: weather, currency \
                and crypto rates, quotes and prices, news and events, sports \
                results, schedules (flights, trains, movies), service \
                status, software releases, charts, league tables, elections, \
                facts after your knowledge cutoff, and anything like \
                \"now / today / this week\". Do NOT refuse citing the \
                cutoff and don't send the user to third-party sites — you \
                have `web`, go get the answer. \
                action=search — search via DuckDuckGo, returns the top-N \
                results (title, url, snippet); use a short query with \
                place/date/units (\"weather Almaty today\", \
                \"USD KZT exchange rate\", \"Bitcoin price USD\"). \
                action=read — fetch a page by URL and extract the \
                article content as Markdown (Mozilla Readability + htmd; \
                nav/footer/ads are filtered automatically). \
                Typical workflow: action=search first, then action=read \
                for the 1-2 most relevant URLs from the results. Don't \
                guess URLs — always search first.",
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["search", "read"],
                        "description": "What to do: `search` — search DDG; \
                            `read` — read a specific URL and return Markdown."
                    },
                    "query": {
                        "type": "string",
                        "description": "Required for action=search. \
                            Natural-language search query."
                    },
                    "url": {
                        "type": "string",
                        "description": "Required for action=read. \
                            HTTP(S) URL of the page to read."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Only for action=search. \
                            How many results to return (1..=20). Default 10.",
                        "minimum": 1,
                        "maximum": 20,
                        "default": 10
                    },
                    "lang": {
                        "type": "string",
                        "description": "Only for action=search. \
                            Preferred language (e.g. \"ru\", \"en\"). Default \"ru\".",
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
            description: "Get the user's markdown instruction (skill) by its id. \
                The full list of available skills (id and short description) is \
                inserted into this description dynamically on every request. When \
                the user's question matches the topic of one of the skills — call \
                `autoskill` with its id, get the markdown, and follow the instruction.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Slug id of the skill (see the list in this tool's description)."
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
            description: "Node-based generation pipelines (video LTX/MiniMax-H3, \
                music ACE-Step, TTS, ASR, LLM node) in the editor's service tab — \
                one per chat, the user can open it from the chat and watch. \
                Typical workflow: 1) action=list — templates (built-in and \
                custom) and chat attachments; 2) action=open with \
                template=<id> — load a template; 3) action=nodes with \
                filter=<kind> — the exact node schema (ports + example \
                state JSON); 4) action=apply — fill it in: \
                set_state=[{node,state}] sets prompts/parameters/paths, \
                connect/disconnect edit links, graph (mode=replace|merge) \
                replaces/extends the whole graph with a Template JSON; \
                5) action=run — start the run (before a heavy run check \
                system status; free_vram=true unloads the chat LLM for the \
                duration of the run and reloads it after). \
                In state strings the attachment:<name|sha|last> reference \
                works — it substitutes the path of a chat attachment (e.g. a \
                ref image in LtxImage.image_path). action=graph — a snapshot \
                of the graph; action=save_template with name — save the \
                graph as a custom template for the user.",
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["list", "nodes", "open", "graph", "apply", "save_template", "run"],
                        "description": "What to do (see the tool description)."
                    },
                    "template": {
                        "type": "string",
                        "description": "action=open: template id from action=list."
                    },
                    "filter": {
                        "type": "string",
                        "description": "action=nodes: a substring matched against kind/name/category. \
                            Without filter — a compact list of all node kinds."
                    },
                    "graph": {
                        "type": "object",
                        "description": "action=apply: Template JSON {nodes:[{id?,kind,pos?,state?,enabled?}], \
                            connections:[{from_node,from_port,to_node,to_port}]}. \
                            id and pos can be omitted — they'll be assigned automatically."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["replace", "merge"],
                        "description": "action=apply with graph: replace — replace the graph (default), \
                            merge — append to the existing one (node ids get reassigned)."
                    },
                    "set_state": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "action=apply: [{node:<id>, state:{kind,data}}] — \
                            several nodes in one call, not one apply per node. \
                            See the state JSON in action=nodes (example with defaults) \
                            or action=graph (current values). data is a patch: only the \
                            listed fields change, the rest of the node's state is kept."
                    },
                    "connect": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "action=apply: [{from_node,from_port,to_node,to_port}] — add links."
                    },
                    "disconnect": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "action=apply: [{from_node,from_port,to_node,to_port}] — remove links."
                    },
                    "name": {
                        "type": "string",
                        "description": "action=save_template: name of the new custom template."
                    },
                    "description": {
                        "type": "string",
                        "description": "action=save_template: template description (optional)."
                    },
                    "free_vram": {
                        "type": "boolean",
                        "description": "action=run: unload the chat LLM for the duration of the run \
                            (needed for heavy pipelines — LTX/H3 won't fit alongside the LLM on 24 GB) \
                            and reload it afterward."
                    },
                    "run_id": {
                        "type": "string",
                        "description": "action=run: run label; when re-running \
                            the same graph, pass a NEW value (e.g. run-2)."
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
            description: "System and memory state. action=status — VRAM \
                (total / free / available accounting for pools), RAM, list \
                of models in memory: node-graph models (with id for \
                unloading) and the chat LLM. action=unload — unload \
                node-graph models from VRAM (by id from status, or all at \
                once with all=true). CALL status before starting a heavy \
                pipeline, to decide whether there's enough VRAM and what to \
                unload. The chat LLM (the one you're running on) is not \
                unloaded by this tool — use free_vram on pipelines action=run.",
            schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "unload"],
                        "description": "status — a snapshot of VRAM/RAM/models; \
                            unload — unload node-graph models."
                    },
                    "id": {
                        "type": "integer",
                        "description": "Only for action=unload: model id \
                            from the status output."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "Only for action=unload: true — \
                            unload all node-graph models."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_NOTES,
            label: "notes",
            icon: MI_EDIT_NOTE,
            description: "Read and edit the user's Notes (the Notes mode of \
                the app): a tree of markdown pages with kanban boards and \
                Gantt charts embedded in them. Changes appear in the UI at \
                once and are saved automatically. Actions: list (page tree \
                with ids and the boards/charts on each page), search \
                (titles + text), read (page markdown, its boards/charts with \
                card ids, links), create / update / move / delete / \
                duplicate (pages), open (show a page to the user), attach \
                (file or chat attachment → media block on a page), kanban \
                (op=create | read | add_column | update_column | \
                delete_column | add_card | update_card | move_card | \
                delete_card | delete), gantt (op=create | read | add_task | \
                update_task | delete_task | add_dep | delete_dep | delete). \
                Workflow: list → read the page → edit. Prefer update with \
                find/replace or mode=append over rewriting a whole page. \
                Pages are addressed by id (12 hex) or exact title; boards \
                and charts by id, or implicitly when the page has one.",
            schema: notes_schema(),
        },
        Tool {
            key: KEY_SUBAGENT,
            label: "subagent",
            icon: MI_BOLT,
            description: "Delegate a long research/local task to a nested \
                agent so it doesn't bloat your context. The subagent runs \
                its own loop with its own set of tools, performs the \
                needed actions, and returns ONLY a brief summary. \
                CALL subagent when: 1) you need to read a long web page \
                or a series of pages and extract 1-3 facts; 2) you need to \
                crawl the project with several `bash find/grep` calls; \
                3) the task needs 2+ tool calls and you don't want them \
                taking up your own context. \
                IMPORTANT ABOUT `task`: pass ONLY the narrow subtask. Don't \
                copy the whole user dialogue, don't restate your overall \
                goal — the subagent has a hard budget (24 tool calls), \
                after which it's forced to hand back a text summary. \
                Good: \"read https://… and return the list of mentioned \
                APIs in 1 paragraph\". Bad: \"help build app X\". \
                A nested subagent (subagent inside subagent) is forbidden. \
                The subagent has no access to your history — phrase the \
                task self-sufficiently. Inside the subagent, tool calls run \
                without user confirmation — the user only confirms the \
                subagent call itself, so phrase the task carefully. Always prefer using subagents.",
            schema: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "A NARROW subtask (not the user's whole \
                            overall goal!). Phrase it self-sufficiently: the \
                            subagent doesn't have your context — write what \
                            it needs for exactly this step, without a long \
                            backstory. End with the instruction \"return a \
                            brief summary in 1-3 paragraphs\". If the task \
                            sounds like \"build an app / implement a \
                            feature\" — it's too broad, break it down \
                            yourself and call subagent for separate \
                            research steps."
                    },
                    "system_prompt": {
                        "type": "string",
                        "description": "Optional custom system prompt for the \
                            subagent. If not set — the user's default system \
                            prompt is used."
                    },
                    "tools": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of tool keys the \
                            subagent is allowed to use (e.g. \
                            [\"bash\", \"web\"]). If omitted — all active \
                            tools of the current chat except \"subagent\". \
                            Don't include \"subagent\" — a nested subagent is forbidden."
                    }
                },
                "required": ["task"],
                "additionalProperties": false
            }),
        },
    ]
}

/// Схема `notes`: свойств много, и один `json!` на всё упирается в лимит
/// рекурсии макроса — собираем карту из мелких литералов.
fn notes_schema() -> serde_json::Value {
    let props: Vec<(&str, serde_json::Value)> = vec![
        ("action", json!({
            "type": "string",
            "enum": ["list", "search", "read", "create", "update", "move",
                     "delete", "duplicate", "open", "attach", "kanban", "gantt"],
            "description": "What to do. kanban and gantt take the sub-operation in op."
        })),
        ("page", json!({
            "type": "string",
            "description": "Page: id (12 hex, from list) or exact title; \
                for duplicate titles use \"Parent / Title\" or the id. \
                Required by read, update, move, delete, duplicate, attach \
                and by kanban/gantt op=create."
        })),
        ("title", json!({
            "type": "string",
            "description": "create: page title (made unique among siblings). \
                update: new title. kanban/gantt op=create: optional heading \
                above the object. kanban add_card/update_card: card title."
        })),
        ("content", json!({
            "type": "string",
            "description": "Page markdown for create and update (see mode). \
                Supported: # headings, - lists, 1. lists, - [ ] todos, > quotes, \
                > [!note] callouts, > [!toggle] toggles, | tables |, ``` code, \
                --- dividers, [[Page title]] wiki links, ![[Page title]] page embeds."
        })),
        ("mode", json!({
            "type": "string",
            "enum": ["replace", "append", "prepend"],
            "description": "update with content: replace the whole page \
                (default) or add the fragment at the end / the start."
        })),
        ("find", json!({
            "type": "string",
            "description": "update: exact markdown fragment to replace — copy \
                it from read. Must occur once unless all=true."
        })),
        ("replace", json!({
            "type": "string",
            "description": "update: replacement for find (empty string deletes \
                the fragment)."
        })),
        ("all", json!({
            "type": "boolean",
            "description": "update: replace every occurrence of find."
        })),
        ("parent", json!({
            "type": "string",
            "description": "create/move: parent page (id or title); omit or \
                \"root\" for the top level."
        })),
        ("index", json!({
            "type": "integer",
            "description": "create/move: 0-based position among siblings; omit \
                for the end."
        })),
        ("icon", json!({
            "type": "string",
            "description": "create/update: page icon — an emoji; \"none\" clears it."
        })),
        ("layout", json!({
            "type": "string",
            "enum": ["free", "flow"],
            "description": "create/update: free canvas (blocks keep their \
                coordinates) or a plain document flow."
        })),
        ("query", json!({
            "type": "string",
            "description": "search: case-insensitive text to find in titles and \
                page markdown."
        })),
        ("limit", json!({
            "type": "integer",
            "description": "search: max pages to return (default 20)."
        })),
        ("path", json!({
            "type": "string",
            "description": "attach: file on disk to put into the page (image, \
                svg, audio, video or any file)."
        })),
        ("attachment", json!({
            "type": "string",
            "description": "attach: a chat attachment instead of a path — file \
                name, sha256 prefix or \"last\"."
        })),
        ("caption", json!({
            "type": "string",
            "description": "attach: caption / alt text of the media block."
        })),
        ("open", json!({
            "type": "boolean",
            "description": "create: also show the new page to the user."
        })),
        ("op", json!({
            "type": "string",
            "description": "kanban: create | read | add_column | update_column | \
                delete_column | add_card | update_card | move_card | \
                delete_card | delete. gantt: create | read | add_task | \
                update_task | delete_task | add_dep | delete_dep | delete."
        })),
        ("board", json!({
            "type": "string",
            "description": "kanban: board id (kanban:<id> from list/read). Omit \
                when the page (or the whole project) has exactly one board."
        })),
        ("chart", json!({
            "type": "string",
            "description": "gantt: chart id (gantt:<id> from list/read). Omit \
                when the page (or the whole project) has exactly one chart."
        })),
        ("columns", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "kanban op=create: column names (default: To do / \
                In progress / Done)."
        })),
        ("column", json!({
            "type": "string",
            "description": "kanban: column by id, name or 1-based number — \
                target of add_card, subject of update_column/delete_column, \
                destination of move_card/update_card."
        })),
        ("name", json!({
            "type": "string",
            "description": "kanban add_column/update_column: column name. \
                gantt add_task/update_task: task name."
        })),
        ("color", json!({
            "type": "string",
            "description": "Column or task color: #rrggbb or gray | orange | \
                green | blue | purple | red | teal; \"none\" clears it."
        })),
        ("width", json!({
            "type": "number",
            "description": "kanban add_column/update_column: column width in \
                px (0 = the board default)."
        })),
        ("card", json!({
            "type": "string",
            "description": "kanban update_card/move_card/delete_card: card id \
                or exact title."
        })),
        ("md", json!({
            "type": "string",
            "description": "kanban add_card/update_card: card body markdown \
                (a `- [ ]` checklist shows progress on the card)."
        })),
        ("priority", json!({
            "type": "string",
            "description": "kanban add_card/update_card: low | medium | high | \
                urgent | none."
        })),
        ("tags", json!({
            "type": "array",
            "items": { "type": "string" },
            "description": "kanban add_card/update_card: tags (array or a \
                comma-separated string)."
        })),
        ("due", json!({
            "type": "string",
            "description": "kanban add_card/update_card: due date yyyy-mm-dd, \
                today, tomorrow or none."
        })),
        ("before", json!({
            "type": "string",
            "description": "kanban add_card/move_card: place the card before \
                this card (id or title) of the target column; omit for the end."
        })),
        ("to_board", json!({
            "type": "string",
            "description": "kanban move_card: move the card to another board (id)."
        })),
        ("task", json!({
            "type": "string",
            "description": "gantt update_task/delete_task: task id or exact name."
        })),
        ("start", json!({
            "type": "string",
            "description": "gantt add_task/update_task: start date yyyy-mm-dd \
                (default today)."
        })),
        ("end", json!({
            "type": "string",
            "description": "gantt add_task/update_task: end date yyyy-mm-dd, \
                inclusive (default start + 2 days; update_task keeps the \
                duration when only start is given)."
        })),
        ("after", json!({
            "type": "string",
            "description": "gantt add_task: make the new task depend on this \
                task (id or name)."
        })),
        ("from", json!({
            "type": "string",
            "description": "gantt add_dep/delete_dep: predecessor task (id or name)."
        })),
        ("to", json!({
            "type": "string",
            "description": "gantt add_dep/delete_dep: successor task (id or name)."
        })),
    ];
    let mut map = serde_json::Map::new();
    for (k, v) in props {
        map.insert(k.to_string(), v);
    }
    json!({
        "type": "object",
        "properties": map,
        "required": ["action"],
        "additionalProperties": false
    })
}
