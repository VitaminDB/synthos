//! Каталог инструментов — чистые описания, без логики исполнения.

use serde_json::json;

use crate::icons::{
    MI_ACCOUNT_TREE, MI_BOLT, MI_EDIT_NOTE, MI_HANDYMAN, MI_HELP_OUTLINE, MI_MEMORY,
    MI_PSYCHOLOGY, MI_SEARCH, MI_TERMINAL, MI_TRAVEL_EXPLORE, MI_VISIBILITY,
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
pub const KEY_WIZARD: &str = "wizard";
pub const KEY_VIEW_MEDIA: &str = "view_media";
pub const KEY_AUTOTOOLS: &str = "autotools";

/// Строит полный список известных инструментов. Вызывается один раз
/// (кэш в `Tool::all()` через `OnceLock`).
pub(super) fn build_all() -> Vec<Tool> {
    vec![
        Tool {
            key: KEY_BASH,
            label: "bash",
            icon: MI_TERMINAL,
            summary: "run a shell command: files, processes, builds, git, local scripts.",
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
            summary: "find facts in the user's local knowledge bases (their own documents).",
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
            summary: "search the web and read pages: news, prices, weather, anything fresh or after your cutoff.",
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
            summary: "get one of the user's markdown instructions (skills) when the question matches its topic.",
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
            key: KEY_AUTOTOOLS,
            label: "autotools",
            icon: MI_HANDYMAN,
            summary: "load the description and parameters of a tool that is not declared up front.",
            description: "Load a tool that is not declared up front. The tools \
                listed below exist and work, but their descriptions and \
                parameters are left out of your context to save room. When the \
                task needs one of them, call `autotools` with its id: the reply \
                is that tool's description, JSON parameters and rules. Then call \
                the tool itself directly by its name, exactly like a declared \
                tool. A tool loaded earlier in this dialogue stays usable — \
                don't load it again. The list is inserted into this description \
                dynamically.",
            schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Id of the tool to load (see the list in this tool's description)."
                    },
                    "ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Several tool ids at once, when the task clearly needs more than one."
                    }
                },
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_PIPELINES,
            label: "pipelines",
            icon: MI_ACCOUNT_TREE,
            summary: "any audio, music, voice or video work: generate, extract stems, TTS, transcribe, edit.",
            description: "Node-based media pipelines in the editor's service tab — \
                one per chat, the user can open it from the chat and watch. \
                THE way to do audio/music/voice/video work on this machine — \
                use it instead of looking for external tools (demucs, spleeter, \
                ffmpeg filters, pip packages). Built-in templates: music from \
                text (ACE-Step Text→Music), vocal / stem extraction from a song \
                (ACE-Step Extract: one stem per run — track_idx on the Generate \
                node picks it: vocals, backing_vocals, drums, bass, guitar, \
                keyboard, …; tags = the stem name; for vocals put the song's \
                lyrics into the lyrics node — without them the voice comes out \
                wordless, so transcribe with ASR first if the text is unknown; \
                there is no \"everything but the vocals\" stem), cover in a new style (ACE-Step Cover), regenerate a \
                region (Repaint), extend a track (Extend), morph toward a new \
                prompt (Edit), variation (Retake); speech: TTS and voice clone \
                (VoxCPM, OmniVoice, VibeVoice dialogue/podcast), ASR \
                (GigaAM), mixing/effects/recording; images and MiniMax-H3 run as stages \
                in the neural service (flow_* nodes, parameters in fields: set_state=[{node, \
                fields: {name: value}}]; flow_image_model.model / flow_h3_model.model = a \
                service model id from action=list): one chain for FLUX.1, FLUX.2, \
                Qwen-Image Edit / 2511, Qwen-Image 2.1 and SDXL — Image Model → Prompt \
                Encoder → (References: Image → flow_image_refs.images, chained via refs → \
                Prompt Encoder.refs + Sampler.refs) → Empty Latent or Image to Latent \
                (img2img) → Sampler → Image Decoder (builtin-flux-*, builtin-flux2-*, \
                builtin-qwen-image-*, builtin-qwen-image21-* — Qwen-Image 2.1 refers to \
                references as <image1>, <image2>…, builtin-sdxl-*; prompt format comes \
                with open); a generated frame straight into video (builtin-ltx-flux-keyframe, \
                builtin-h3-flux-keyframe: Image Decoder.image → LTX Frame / H3 \
                Encoder.first_frame); LTX-2.3 video with audio, also as service stages: LTX Model \
                (fields.lora: union / motion / lipdub or a file) → LTX Encoder (+ LTX NAG) → \
                LTX Stage1 (LTX Frame: frame 0 = first frame, >0 = keyframe) → LTX Upscale ×2 → \
                LTX Stage2, or LTX Retake / IC-LoRA (control none/canny/depth) / LipDub / \
                Audio to Video → LTX Video/Audio Decoder → Mux (builtin-ltx-*); \
                MiniMax-H3 video with native audio — H3 Model → H3 Video Size → H3 Encoder \
                (first_frame/last_frame from flow_file.path, or refs from a flow_list whose \
                fields.items = file paths, one per line — images, videos, audio the user \
                attached; template builtin-h3-ref2va) → H3 Sampler → H3 Video/Audio \
                Decoder → Mux; open/apply answer with the <Picture/Video/Audio N> tags \
                and the prompt format — follow it). \
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
                In state and fields strings the attachment:<name|sha|last> reference \
                works — it substitutes the path of a chat attachment (e.g. a \
                ref image in flow_file.path feeding LTX Frame). action=graph — a snapshot \
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
            summary: "VRAM/RAM status and loaded models; unload node-graph models.",
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
            key: KEY_WIZARD,
            label: "wizard",
            icon: MI_HELP_OUTLINE,
            summary: "ask the user one question with clickable answer options when the decision is theirs.",
            description: "Ask the user ONE question with clickable answer \
                options (single or multiple choice) and/or a free-text \
                field. A panel appears in the chat; the user's click is \
                sent as their next message (the chosen option labels, or \
                the typed text). THIS CALL ENDS YOUR TURN: write nothing \
                after it and call no other tool in the same turn — the \
                reply comes as the next user message. Use it for decisions \
                only the user can make (a period, a template, confirm vs \
                alternatives), not for questions you can answer yourself. \
                2–6 short options; value = machine-readable id when it \
                differs from the label; allow_free_text on an option = the \
                user types a custom value for it; required = the panel \
                cannot be skipped; timeout_sec = the panel folds without an \
                answer after that many seconds.",
            schema: json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question shown above the options. Markdown: **bold**, lists, and fenced code blocks with a language tag (```rust … ```) render with syntax highlighting."
                    },
                    "options": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string", "description": "Button text." },
                                "value": { "type": "string", "description": "Optional id; the user's message carries the label." },
                                "tooltip": { "type": "string", "description": "Hint shown on hover." },
                                "allow_free_text": { "type": "boolean", "description": "Choosing this option opens a text field; the answer is \"label: text\"." }
                            },
                            "required": ["label"],
                            "additionalProperties": false
                        },
                        "description": "2–6 answer buttons (max 12). Omit for a free-text question."
                    },
                    "allow_multiple": {
                        "type": "boolean",
                        "description": "true — checkboxes, several options at once (labels come comma-separated)."
                    },
                    "allow_free_text": {
                        "type": "boolean",
                        "description": "true — a free-text field next to the options."
                    },
                    "required": {
                        "type": "boolean",
                        "description": "true — the user cannot skip the question."
                    },
                    "timeout_sec": {
                        "type": "integer",
                        "description": "Seconds until the panel folds without an answer (0 = no timer)."
                    }
                },
                "required": ["question"],
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_VIEW_MEDIA,
            label: "view_media",
            icon: MI_VISIBILITY,
            summary: "look at local images or video and listen to audio files by path.",
            description: "Look at local files yourself. Images and video go \
                through your own vision encoder — you see them natively, exactly \
                as if the user had attached them; audio is transcribed by the \
                loaded speech-recognition model; text documents are inlined. \
                CALL it whenever the user points to a picture, photo, video or \
                recording on disk, or asks you to look at, compare or find \
                something in local media (e.g. find a person across a folder of \
                photos). It does not list directories: find the files first with \
                bash (`ls DIR`, `find DIR -type f -iname '*.jpg'`), then pass up \
                to 8 paths per call and call it again for the rest. Each file \
                comes under a `[file N: name]` header, followed by a list that \
                maps N to its full path; a file that could not be shown (no \
                vision encoder, video unsupported by this model, context full) \
                is listed with the reason.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute path (or ~/…) of one file to look at."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "maxItems": 8,
                        "description": "Several files at once (up to 8), shown in this order."
                    }
                },
                "additionalProperties": false
            }),
        },
        Tool {
            key: KEY_NOTES,
            label: "notes",
            icon: MI_EDIT_NOTE,
            summary: "the user's Notes: pages, kanban tasks, Gantt, calendar, mind maps, charts; agenda and journal.",
            description: "Read and edit the user's Notes (the Notes mode of \
                the app): a tree of markdown pages — a free canvas where \
                blocks have coordinates — with shapes/arrows, kanban boards \
                and Gantt charts embedded in them. Changes appear in the UI \
                at once and are saved automatically. LIFE MANAGEMENT: agenda \
                (the day at a glance: overdue, due today / tomorrow / soon, \
                events, recently done, board counts — ONE call, use it first \
                for \"what should I do\" questions), tasks (cards across all \
                boards filtered by due/tag/priority/done/board/column/query), \
                log (history: what was added, moved, done, archived, when and \
                by whom — the app records every change itself, never keep a \
                history page by hand), journal (the day's page: get or create \
                Journal / yyyy-mm / yyyy-mm-dd, append notes). Cards carry \
                created/done dates, repeat (daily…yearly: closing spawns the \
                next one), attachments (attach path|attachment → image \
                thumbnail or file with a paperclip), and every board has a \
                DONE column (done=true on a column): move_card there = task \
                done; done cards auto-archive after archive_after days. Due \
                dates already appear in the calendar — never copy them into \
                events. TIME: every reply opens with a `now:` line — the \
                local date, clock time and weekday at the moment of the \
                call; your system prompt carries the date alone. Take due \
                dates, event and reminder times and journal stamps from it \
                instead of guessing the clock, and when you need the time \
                before writing anything, read first (agenda is the cheapest \
                call). Actions: list (page \
                tree with ids and the boards/charts on each page), search \
                (titles + text), read (page markdown, layout, its \
                boards/charts with card ids, links; blocks=true lists blocks \
                with coordinates; pages=[…], depth=all or page=\"all\" read \
                many pages, a whole subtree or the whole project in ONE \
                call), create / update / move / delete / \
                duplicate (pages; update also sets grid/snap, shows or \
                hides the page's right Properties panel and inserts content \
                at a position), open (show a page to the user), \
                attach (file or chat attachment → media block), blocks \
                (op=list | read | insert | set_markdown | delete | move | \
                nest | unnest | set_attrs | pin | unpin — blocks by index, \
                coordinates x y w h, \
                text style color/bg/size/align/weight), shape (op=create | \
                update | delete | connect — rect/ellipse/triangle/diamond, \
                lines and arrows with absolute end points, connect draws an \
                arrow between two pinned blocks), kanban (op=create | read | \
                set_style | add_column | update_column | delete_column | \
                add_card | update_card | move_card | delete_card | delete), \
                gantt (op=create | read | add_task | update_task | \
                delete_task | add_dep | delete_dep | set_zoom | show_today | \
                delete), chart (op=create | read | update | set_data | \
                add_series | update_series | delete_series | set_style | \
                from_table | delete — a line, bar, pie, radar or gauge chart \
                drawn from labels + series of numbers; from_table turns a \
                markdown table on the page into one). Workflow: list → read the pages you need in one \
                call (pages=[…] / depth=all, blocks op=read block=all for \
                every block of a page) → edit. Reading one page or one \
                block per call is the slow way and wastes the user's time. \
                Prefer \
                update with find/replace, mode=append or blocks ops over \
                rewriting a whole page. Pages are addressed by id (12 hex) \
                or exact title; blocks by index from blocks op=list; boards \
                and charts by id, or implicitly when the page has one. \
                PROJECTS. Several projects (.syn files) can be open, a tile \
                each: list, search, agenda, tasks and log cover all of them; \
                any other action \
                works in the project shown in Notes unless you pass \
                project=<title or path>. \
                PROPERTIES PANEL. The right panel of the Notes mode \
                (Properties | Links tabs) is remembered by each page: \
                update props_panel=hidden|shown sets it, read shows it in \
                the layout line, and the screen follows at once when that \
                page is open. For every page at once: update page=\"all\" \
                props_panel=hidden — ONE call; pages=[…] takes a list. \
                NESTING. What a toggle hides — and what a callout, a quote or \
                a list item holds — are its nested blocks, written in markdown \
                as quote lines: EVERY line of the content, table rows \
                included, starts with \"> \". A \"> [!toggle] Payment \
                schedule\" line followed by a bare table is two blocks side by \
                side, and the toggle then collapses nothing. Write the whole \
                thing as quote lines (\"> [!toggle] Payment schedule\", \">\", \
                \"> | # | Date |\", \"> | --- | --- |\", \"> | 1 | 2025-06 |\"), \
                or fix a page that already has them apart with blocks op=nest \
                (the block goes inside the one above it). blocks op=list shows \
                nested blocks as \"#3.0\" under their container. \
                LAYOUT. A new page is a free canvas, but as long as none of \
                its blocks is pinned they flow in one centred column and \
                the page reads as a plain document — keep it that way and \
                pass no coordinates unless the user asked for a canvas. \
                The moment anything is pinned (a shape, a board, a chart, \
                blocks op=pin, insert with x/y) nothing arranges the rest \
                for you: EVERY block must then carry x, y and w (plus h for \
                shapes, media, boards, charts, mind maps and calendars), \
                otherwise the unplaced ones are drawn in the flow column ON \
                TOP of the pinned blocks and the page turns into a pile. \
                create layout=free with content, and update layout=free, \
                pin the unplaced blocks in a column where the flow had \
                them; content added later carries no geometry, so after \
                such an edit on a page with pinned blocks run blocks \
                op=arrange once (stacks every unplaced block below the \
                pinned ones) or pin them one by one (blocks op=pin x y w). \
                Plan the layout before writing it — e.g. one column at x=40 \
                with y growing by the block's height + 24, objects below \
                the text — and check the result with read blocks=true, \
                where h=~ marks a height the editor estimated; a reply line \
                starting with !! free layout lists blocks still unplaced on \
                a page with pinned ones, and !! overlaps names blocks whose \
                frames intersect. !! gap N px above it (in the block list) \
                marks empty space over a block: close it by moving that \
                block up — a heading brings the blocks of its section along.",
            schema: notes_schema(),
        },
        Tool {
            key: KEY_SUBAGENT,
            label: "subagent",
            icon: MI_BOLT,
            summary: "delegate a narrow research subtask to a nested agent that returns a short summary.",
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

/// Схема `notes` — из протокола заметок (общая с ассистентом synshell).
pub(crate) fn notes_schema() -> serde_json::Value {
    syn_notes_proto::schema::notes_schema()
}
