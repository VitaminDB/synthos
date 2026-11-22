//! Что нужно, чтобы поднять вкладку терминала заново, если её процесс
//! потерян (хранитель терминалов убит вместе с synthos, перезагрузка):
//! рабочий каталог оболочки и сессия Claude Code, если она в вкладке шла.
//!
//! Сессию Claude узнаём по реестру самого Claude Code —
//! `~/.claude/sessions/<pid>.json` (`pid`, `sessionId`, `cwd`): процесс
//! `pid` должен быть потомком оболочки вкладки (идём вверх по `PPid`).
//! Семплер ([`super::terminal_activity`]) раз в несколько секунд обновляет
//! [`ResumeInfo`] вкладки; оно сохраняется в конфиг рядом с ключом вкладки.
//! При восстановлении вкладка без живого терминала открывается в прежнем
//! каталоге и набирает `claude … --resume <id>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Снимок вкладки для восстановления.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResumeInfo {
    /// Текущий каталог оболочки.
    pub cwd: Option<String>,
    /// `sessionId` Claude Code, запущенного в вкладке.
    pub claude_session: Option<String>,
    /// Аргументы того запуска Claude (без программы), чтобы вернуть те же
    /// флаги (`--permission-mode …`, `--model …`).
    pub claude_args: Vec<String>,
}

/// Живые процессы Claude Code: pid → (sessionId, аргументы).
pub fn claude_sessions() -> HashMap<u32, (String, Vec<String>)> {
    let mut out = HashMap::new();
    let Some(dir) = home().map(|h| h.join(".claude/sessions")) else {
        return out;
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let (Some(pid), Some(id)) = (v["pid"].as_u64(), v["sessionId"].as_str()) else {
            continue;
        };
        let pid = pid as u32;
        let Some(args) = cmdline(pid) else { continue }; // процесс уже умер
        out.insert(pid, (id.to_string(), args.into_iter().skip(1).collect()));
    }
    out
}

/// Снимок вкладки с оболочкой `shell_pid`.
pub fn probe(shell_pid: u32, claude: &HashMap<u32, (String, Vec<String>)>) -> ResumeInfo {
    let cwd = std::fs::read_link(format!("/proc/{shell_pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let found = claude
        .iter()
        .filter(|(pid, _)| is_descendant(**pid, shell_pid))
        .max_by_key(|(pid, _)| **pid);
    match found {
        Some((_, (id, args))) => ResumeInfo {
            cwd,
            claude_session: Some(id.clone()),
            claude_args: args.clone(),
        },
        None => ResumeInfo { cwd, ..Default::default() },
    }
}

/// Строка для оболочки: `claude <прежние флаги> --resume <id>`.
pub fn resume_command(session: &str, args: &[String]) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut it = args.iter().map(String::as_str);
    while let Some(a) = it.next() {
        match a {
            // Флаги выбора сессии со значением — выкинуть вместе со значением.
            "--resume" | "-r" | "--session-id" | "--fork-session" => {
                if a != "--fork-session" {
                    let _ = it.next();
                }
            }
            "--continue" | "-c" => {}
            _ if a.starts_with("--resume=") || a.starts_with("--session-id=") => {}
            // Первый позиционный аргумент — начальный промпт; повторять его нельзя.
            _ if !a.starts_with('-') && kept.last().is_none_or(|p| !takes_value(p)) => {}
            _ => kept.push(a),
        }
    }
    let mut cmd = String::from("claude");
    for a in kept {
        cmd.push(' ');
        cmd.push_str(&shell_quote(a));
    }
    cmd.push_str(" --resume ");
    cmd.push_str(&shell_quote(session));
    cmd
}

/// Флаги Claude Code со значением (чтобы не принять значение за промпт).
fn takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "--permission-mode"
            | "--model"
            | "--fallback-model"
            | "--add-dir"
            | "--allowedTools"
            | "--allowed-tools"
            | "--disallowedTools"
            | "--disallowed-tools"
            | "--mcp-config"
            | "--settings"
            | "--agent"
            | "--append-system-prompt"
            | "--system-prompt"
            | "--name"
            | "-n"
    )
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_=./:,@+".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn is_descendant(mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..64 {
        if pid == ancestor {
            return true;
        }
        match ppid(pid) {
            Some(p) if p > 1 => pid = p,
            _ => return false,
        }
    }
    false
}

fn ppid(pid: u32) -> Option<u32> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // После «(comm)»: state ppid …
    let rest = &s[s.rfind(')')? + 2..];
    rest.split_whitespace().nth(1)?.parse().ok()
}

fn cmdline(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    Some(
        raw.split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
    )
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p: &PathBuf| Path::new(p).is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resume_keeps_flags_drops_session_selection() {
        assert_eq!(
            resume_command("abc-1", &v(&["--permission-mode", "bypassPermissions"])),
            "claude --permission-mode bypassPermissions --resume abc-1"
        );
        assert_eq!(
            resume_command("new", &v(&["--resume", "old", "--model", "opus", "-c"])),
            "claude --model opus --resume new"
        );
        assert_eq!(resume_command("x", &v(&["fix the bug"])), "claude --resume x");
        assert_eq!(
            resume_command("x", &v(&["--append-system-prompt", "a b"])),
            "claude --append-system-prompt 'a b' --resume x"
        );
    }

    #[test]
    fn descendant_of_self_chain() {
        let me = std::process::id();
        assert!(is_descendant(me, me));
        let parent = ppid(me).unwrap();
        assert!(is_descendant(me, parent));
        assert!(!is_descendant(parent, me));
    }
}

/// Живая проверка на реальной вкладке: `PROBE_SHELL=<pid оболочки> cargo test
/// --lib probe_live -- --ignored --nocapture`.
#[cfg(test)]
#[test]
#[ignore]
fn probe_live() {
    let pid: u32 = std::env::var("PROBE_SHELL").unwrap().parse().unwrap();
    let info = probe(pid, &claude_sessions());
    println!("{info:?}");
    if let Some(id) = &info.claude_session {
        println!("{}", resume_command(id, &info.claude_args));
    }
}
