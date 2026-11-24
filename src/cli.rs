//! Командная строка synthos.
//!
//! ```text
//! synthos                                     открыть окно (уже открыто — поднять его)
//! synthos --open-terminal <папка> [--cmd <команда>] [--title <заголовок>]
//!                                             вкладка терминала в проекте папки
//! synthos --list-terminals                  терминалы хранителя
//! synthos --screenshot <png> [--template <id>|demo] [--size ШxВ] [--scale N]
//!                                             снимок редактора нод без окна
//! synthos --help | --version
//! synthos --pty-holder <сокет>                служебное: хранитель терминалов
//! ```
//!
//! Второй запуск окно не открывает: аргументы уходят работающему synthos
//! (`syngui::app::forward_to_running_instance`), и он выполняет их у себя —
//! так другие программы (агенты в терминалах) открывают вкладки сами.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use syngui::prelude::*;

use crate::context::AppCtx;
use crate::pages::code_editor::state::{self, CodeEditorCtx, NewTerminal};

/// Что попросили в командной строке.
#[derive(Debug, Default, PartialEq)]
pub struct Cli {
    pub help: bool,
    pub version: bool,
    pub pty_holder: Option<PathBuf>,
    pub list_terminals: bool,
    pub open_terminals: Vec<OpenTerminal>,
    /// `--screenshot <png> [--template <id>|demo] [--size ШxВ] [--scale N]`.
    pub screenshot: Option<crate::shot::Shot>,
}

/// `--open-terminal <папка> [--cmd …] [--title …]`.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenTerminal {
    pub dir: PathBuf,
    pub cmd: Option<String>,
    pub title: Option<String>,
}

pub const USAGE: &str = "\
Использование:
  synthos                                   открыть окно (если открыто — поднять его)
  synthos --open-terminal <папка> [--cmd <команда>] [--title <заголовок>]
                                            открыть вкладку терминала в проекте папки;
                                            --cmd набирается в оболочке вкладки
  synthos --list-terminals                  терминалы, которые держит хранитель
                                            (ключ, pid, подключена ли вкладка, папка, заголовок)
  synthos --screenshot <png> [--template <id>|demo] [--size ШxВ] [--scale N]
                                            снимок редактора нод без окна
  synthos --help | --version

Если synthos уже запущен, аргументы передаются ему, второе окно не открывается.";

/// Разбор аргументов (без имени программы).
pub fn parse(args: &[String]) -> std::result::Result<Cli, String> {
    let mut cli = Cli::default();
    let mut it = args.iter().peekable();
    let value = |it: &mut std::iter::Peekable<std::slice::Iter<String>>, flag: &str| {
        it.next().cloned().ok_or_else(|| format!("{flag}: нужно значение"))
    };
    while let Some(arg) = it.next() {
        // `--flag=value` наравне с `--flag value`.
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        let take = |it: &mut std::iter::Peekable<std::slice::Iter<String>>| match inline.clone() {
            Some(v) => Ok(v),
            None => value(it, flag),
        };
        match flag {
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "--list-terminals" => cli.list_terminals = true,
            "--pty-holder" => cli.pty_holder = Some(PathBuf::from(take(&mut it)?)),
            "--open-terminal" => cli.open_terminals.push(OpenTerminal {
                dir: PathBuf::from(take(&mut it)?),
                cmd: None,
                title: None,
            }),
            "--cmd" | "--title" => {
                let v = take(&mut it)?;
                let Some(last) = cli.open_terminals.last_mut() else {
                    return Err(format!("{flag} — только после --open-terminal <папка>"));
                };
                if flag == "--cmd" {
                    last.cmd = Some(v);
                } else {
                    last.title = Some(v);
                }
            }
            "--screenshot" => {
                cli.screenshot = Some(crate::shot::Shot {
                    out: take(&mut it)?,
                    template: "demo".into(),
                    size: (1600, 1000),
                    scale: 1.0,
                })
            }
            "--template" | "--size" | "--scale" => {
                let v = take(&mut it)?;
                let Some(s) = cli.screenshot.as_mut() else {
                    return Err(format!("{flag} — только после --screenshot <png>"));
                };
                match flag {
                    "--template" => s.template = v,
                    "--size" => {
                        let (w, h) = v.split_once('x').ok_or_else(|| format!("--size: ШxВ, а не {v}"))?;
                        s.size = (w.parse().map_err(|_| format!("--size: {v}"))?, h.parse().map_err(|_| format!("--size: {v}"))?);
                    }
                    _ => s.scale = v.parse().map_err(|_| format!("--scale: {v}"))?,
                }
            }
            other => return Err(format!("неизвестный аргумент: {other}")),
        }
    }
    Ok(cli)
}

impl Cli {
    /// Аргументы для передачи работающему synthos: папки — абсолютные (у
    /// него свой текущий каталог).
    pub fn to_forward_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        for t in &self.open_terminals {
            out.push("--open-terminal".into());
            out.push(absolute(&t.dir).display().to_string());
            if let Some(c) = &t.cmd {
                out.push("--cmd".into());
                out.push(c.clone());
            }
            if let Some(t) = &t.title {
                out.push("--title".into());
                out.push(t.clone());
            }
        }
        out
    }
}

/// Вывод `--list-terminals`: строка на терминал, поля через табуляцию.
pub fn print_terminals() {
    use crate::pages::code_editor::terminal_holder::{list, TerminalMeta};
    let all = list();
    if all.is_empty() {
        println!("хранитель терминалов не запущен или терминалов нет");
        return;
    }
    println!("ключ\tpid\tвкладка\tпапка\tзаголовок");
    for h in all {
        let m = TerminalMeta::decode(&h.meta);
        println!(
            "{}\t{}\t{}\t{}\t{}",
            h.key,
            h.pid,
            if h.attached { "подключена" } else { "отсоединена" },
            m.folder.unwrap_or_else(|| if m.terminal_only { "~ (Терминал)".into() } else { "-".into() }),
            m.title
        );
    }
}

fn absolute(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    })
}

static STARTUP_ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// Аргументы этого запуска — выполнить после построения интерфейса.
pub fn set_startup_args(args: Vec<String>) {
    let _ = STARTUP_ARGS.set(args);
}

/// Обработчик аргументов от повторных запусков и выполнение своих.
pub fn install(_ctx: &AppCtx) {
    syngui::app::on_instance_args(|args| execute(&args));
    if let Some(args) = STARTUP_ARGS.get().filter(|a| !a.is_empty()).cloned() {
        // После первого кадра: роутер и страницы к этому моменту собраны.
        syngui::async_runtime::run_on_main_thread(move || execute(&args));
    }
}

fn execute(args: &[String]) {
    match parse(args) {
        Ok(cli) => {
            for t in cli.open_terminals {
                open_terminal(&t);
            }
        }
        Err(e) => tracing::warn!(error = %e, ?args, "аргументы командной строки не разобраны"),
    }
}

/// Вкладка терминала в сессии проекта папки `t.dir`: сессия с этим корнем,
/// иначе ближайшая, чей корень содержит папку, иначе новая сессия проекта.
pub fn open_terminal(t: &OpenTerminal) {
    let code = use_context::<CodeEditorCtx>();
    let app = use_context::<AppCtx>();
    let dir = absolute(&t.dir);
    let sessions = code.sessions.get_untracked();
    let session = sessions
        .iter()
        .filter(|s| !s.terminal_only)
        .filter_map(|s| {
            let root = s.root_folder.get_untracked()?;
            dir.starts_with(&root).then(|| (root.components().count(), *s))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, s)| s);
    let session = match session {
        Some(s) => s,
        None => {
            let id = code.create_empty();
            let Some(s) = code.sessions.get_untracked().into_iter().find(|s| s.id == id) else {
                return;
            };
            if dir.is_dir() {
                state::set_root_folder(s, dir.clone());
            }
            s
        }
    };
    code.switch_to(session.id);
    let id = state::add_terminal_with(
        session,
        app,
        NewTerminal {
            cwd: Some(dir.clone()),
            title: t.title.clone(),
            input: t.cmd.as_ref().map(|c| format!("{c}\n")),
            ..Default::default()
        },
    );
    crate::rail::navigate("code");
    tracing::info!(dir = %dir.display(), cmd = ?t.cmd, tab = ?id, "открыта вкладка терминала по команде");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_open_terminal() {
        let cli = parse(&a(&["--open-terminal", "/tmp", "--cmd", "claude --resume x", "--title=T"])).unwrap();
        assert_eq!(
            cli.open_terminals,
            vec![OpenTerminal {
                dir: "/tmp".into(),
                cmd: Some("claude --resume x".into()),
                title: Some("T".into())
            }]
        );
        assert!(parse(&a(&["--cmd", "x"])).is_err());
        assert!(parse(&a(&["--bogus"])).is_err());
        assert!(parse(&a(&["--help"])).unwrap().help);
        assert_eq!(
            parse(&a(&["--pty-holder", "/run/x.sock"])).unwrap().pty_holder,
            Some("/run/x.sock".into())
        );
    }

    #[test]
    fn forward_args_are_absolute_and_reparse() {
        let cli = parse(&a(&["--open-terminal", ".", "--cmd", "ls"])).unwrap();
        let fwd = cli.to_forward_args();
        let back = parse(&fwd).unwrap();
        assert!(back.open_terminals[0].dir.is_absolute());
        assert_eq!(back.open_terminals[0].cmd.as_deref(), Some("ls"));
    }
}
