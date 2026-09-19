//! Хранение Syn-чатов на диске: `~/.config/synthos/syn_chats/{id}.json`.
//!
//! Формат файла — общий с основным llama.cpp-чатом
//! ([`crate::agent::storage::StoredChat`]), так что переключение между чатами
//! сохраняет историю как обычно. Отличается только директория, чтобы
//! пользователь видел Syn-чаты отдельным списком.
//!
//! Пишет файлы один фоновый поток ([`Writer`]): автосейв открытого чата
//! ([`save_async`]) и правки лент фоновых чатов ([`update_async`]) не держат
//! главный поток сериализацией и `fs::write`. Записи одного файла идут
//! строго в порядке постановки, подряд идущие снимки сворачиваются в
//! последний. Чтения ([`load`], [`list_meta`]) и [`delete`] сначала
//! дожидаются очереди своего файла — диск для них всегда актуален.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

pub use crate::agent::state::{ChatMeta, ChatMsg};
pub use crate::agent::storage::{preview_from_messages, truncate_chars, StoredChat};
use crate::syn_chat::chat_settings::ChatSettings;

#[cfg(test)]
thread_local! {
    /// Каталог чатов текущего тестового потока вместо `~/.config/…`.
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Направить файлы чатов этого тестового потока во временный каталог; с
/// дропом ручки — обратно. Через `HOME` нельзя: его меняют и читают
/// параллельные тесты.
#[cfg(test)]
pub(crate) fn test_dir(dir: &Path) -> TestDir {
    TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.to_path_buf()));
    TestDir
}

#[cfg(test)]
pub(crate) struct TestDir;

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        TEST_DIR.with(|d| *d.borrow_mut() = None);
    }
}

/// Каталог, где лежат JSON-файлы Syn-чатов. Создаётся при первом save.
pub fn syn_chats_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|d| d.borrow().clone()) {
        return dir;
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/synthos/syn_chats")
}

fn chat_path(id: &str) -> PathBuf {
    syn_chats_dir().join(format!("{}.json", id))
}

/// Прежний дефолтный заголовок чата. Раздел назывался «Syn-чаты», сейчас —
/// просто «Чаты», поэтому старые файлы приводим к новому имени.
const LEGACY_DEFAULT_TITLE: &str = "Новый Syn-чат";
/// Текущий дефолтный заголовок (см. `registry::create_new`).
const DEFAULT_TITLE: &str = "Новый чат";

/// Одноразовая миграция заголовка: `true`, если чат был переименован и его
/// нужно перезаписать на диск. Пользовательские названия не трогаем —
/// только точное совпадение со старым дефолтом.
fn migrate_legacy_title(chat: &mut StoredChat) -> bool {
    if chat.title != LEGACY_DEFAULT_TITLE {
        return false;
    }
    chat.title = DEFAULT_TITLE.to_string();
    true
}

pub fn list_meta() -> Vec<ChatMeta> {
    scan(None)
}

/// [`list_meta`] на старте приложения: файлы, записанные до того, как
/// инструменты, скилы и промпт стали храниться в чате, получают `defaults`
/// — общие настройки, с которыми эти чаты до сих пор и работали — и
/// переписываются один раз. Иначе такой чат при первом открытии взял бы
/// настройки чата, открытого перед ним.
pub fn list_meta_filling(defaults: &ChatSettings) -> Vec<ChatMeta> {
    scan(Some(defaults))
}

fn scan(fill: Option<&ChatSettings>) -> Vec<ChatMeta> {
    // Список читают на старте и перед сборкой мусора CAS: ссылки на
    // вложения должны быть уже на диске.
    flush_all();
    let dir = syn_chats_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            eprintln!("[synthos/syn_chat] не удалось прочитать {:?}: {e}", dir);
            return Vec::new();
        }
    };

    let mut out: Vec<ChatMeta> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<StoredChat>(&text) {
                Ok(mut chat) => {
                    // Список чатов читается на старте — удобная точка для
                    // одноразового переименования старого дефолта.
                    let mut migrated = migrate_legacy_title(&mut chat);
                    if let (None, Some(defaults)) = (&chat.settings, fill) {
                        chat.settings = Some(defaults.clone());
                        migrated = true;
                    }
                    out.push(chat.to_meta());
                    if migrated {
                        save_async(chat);
                    }
                }
                Err(e) => eprintln!("[synthos/syn_chat] пропускаю битый файл {:?}: {e}", path),
            },
            Err(e) => eprintln!("[synthos/syn_chat] не смог прочитать {:?}: {e}", path),
        }
    }

    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

/// Чат с диска. Сначала дожидается записей этого файла, стоящих в очереди:
/// иначе возврат в фоновый чат прочитал бы ленту без последних правок.
pub fn load(id: &str) -> Option<StoredChat> {
    let path = chat_path(id);
    writer().flush(&path);
    read_chat(&path)
}

fn read_chat(path: &Path) -> Option<StoredChat> {
    match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<StoredChat>(&text) {
            Ok(mut chat) => {
                migrate_legacy_title(&mut chat);
                Some(chat)
            }
            Err(e) => {
                eprintln!("[synthos/syn_chat] битый JSON {:?}: {e}", path);
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("[synthos/syn_chat] не смог прочитать {:?}: {e}", path);
            None
        }
    }
}

/// Записать чат и дождаться, пока файл окажется на диске.
pub fn save(chat: &StoredChat) {
    let path = chat_path(&chat.id);
    writer().put(path.clone(), chat.clone());
    writer().flush(&path);
}

/// Поставить снимок чата в очередь фоновой записи и вернуться сразу.
/// Снимок, ещё не начатый писателем, заменяется следующим.
pub fn save_async(chat: StoredChat) {
    writer().put(chat_path(&chat.id), chat);
}

/// Поправить файл чата в фоне: писатель прочитает его после всех записей,
/// стоящих перед правкой, применит `f` и запишет. Файла нет — правка
/// пропускается.
pub fn update_async(id: &str, f: impl FnOnce(&mut StoredChat) + Send + 'static) {
    writer().patch(chat_path(id), Box::new(f));
}

/// Дождаться записи всего, что стоит в очереди для чата `id`.
pub fn flush(id: &str) {
    writer().flush(&chat_path(id));
}

/// Дождаться, пока очередь записи опустеет (выход, сканирование каталога).
pub fn flush_all() {
    writer().flush_all();
}

/// Удалить файл чата. Записи, ещё стоящие в очереди, отменяются — иначе
/// запоздавший автосейв воскресил бы удалённый чат.
pub fn delete(id: &str) {
    writer().remove(&chat_path(id));
}

fn remove_file(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[synthos/syn_chat] не удалось удалить {:?}: {e}", path),
    }
    let _ = std::fs::remove_file(tmp_path(path));
}

fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

/// Через временный файл и rename: обрыв посреди записи (выход, падение) не
/// должен оставлять полфайла на месте ленты. `.json.tmp` списки чатов и
/// поиск не читают.
fn write_chat(path: &Path, chat: &StoredChat) -> bool {
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("[synthos/syn_chat] не удалось создать {:?}: {e}", dir);
            return false;
        }
    }
    let content = match serde_json::to_string_pretty(chat) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[synthos/syn_chat] ошибка сериализации {}: {e}", chat.id);
            return false;
        }
    };
    let tmp = tmp_path(path);
    if let Err(e) = std::fs::write(&tmp, content) {
        eprintln!("[synthos/syn_chat] не удалось записать {:?}: {e}", tmp);
        return false;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!("[synthos/syn_chat] не удалось заменить {:?}: {e}", path);
        return false;
    }
    true
}

// ─────────────────────────── фоновый писатель ───────────────────────────

/// Правка файла чата поверх того, что лежит на диске.
type Patch = Box<dyn FnOnce(&mut StoredChat) + Send + 'static>;

enum Op {
    /// Полный снимок (автосейв открытого чата, новый чат, архив).
    Put(Box<StoredChat>),
    /// Прочитать, поправить, записать (лента фонового чата, флаг архива).
    Patch(Patch),
}

struct Job {
    path: PathBuf,
    op: Op,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    /// Файл, который писатель обрабатывает прямо сейчас.
    busy: Option<PathBuf>,
    /// Ручку писателя уронили: поток доделывает очередь и выходит.
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    /// Будит писателя (новая запись) и ждущих (запись закончена).
    cv: Condvar,
    /// Сколько раз файлы реально переписаны.
    writes: AtomicU64,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<'a>(
        &self,
        guard: std::sync::MutexGuard<'a, Queue>,
    ) -> std::sync::MutexGuard<'a, Queue> {
        self.cv.wait(guard).unwrap_or_else(|e| e.into_inner())
    }
}

/// Очередь записи файлов чатов и её поток. Один поток на всё — записи
/// одного файла не обгоняют друг друга, старый снимок не ложится поверх
/// нового.
pub(crate) struct Writer {
    shared: Arc<Shared>,
}

fn writer() -> &'static Writer {
    static WRITER: OnceLock<Writer> = OnceLock::new();
    WRITER.get_or_init(Writer::new)
}

impl Writer {
    pub(crate) fn new() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            cv: Condvar::new(),
            writes: AtomicU64::new(0),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("synthos-chat-writer".to_string())
            .spawn(move || run_writer(&worker))
            .expect("поток записи чатов");
        Self { shared }
    }

    /// Снимок в очередь. Если последняя запись этого файла в очереди — тоже
    /// снимок и писатель её ещё не взял, она заменяется: на диск уходит
    /// только последний.
    pub(crate) fn put(&self, path: PathBuf, chat: StoredChat) {
        let mut q = self.shared.lock();
        let last = q.jobs.iter_mut().rev().find(|j| j.path == path);
        match last {
            Some(Job {
                op: op @ Op::Put(_),
                ..
            }) => *op = Op::Put(Box::new(chat)),
            _ => q.jobs.push_back(Job {
                path,
                op: Op::Put(Box::new(chat)),
            }),
        }
        self.shared.cv.notify_all();
    }

    pub(crate) fn patch(&self, path: PathBuf, f: Patch) {
        let mut q = self.shared.lock();
        q.jobs.push_back(Job {
            path,
            op: Op::Patch(f),
        });
        self.shared.cv.notify_all();
    }

    /// Ждать, пока файл `path` не останется ни в очереди, ни в работе.
    pub(crate) fn flush(&self, path: &Path) {
        let mut q = self.shared.lock();
        while q.busy.as_deref() == Some(path) || q.jobs.iter().any(|j| j.path == path) {
            q = self.shared.wait(q);
        }
    }

    pub(crate) fn flush_all(&self) {
        let mut q = self.shared.lock();
        while q.busy.is_some() || !q.jobs.is_empty() {
            q = self.shared.wait(q);
        }
    }

    /// Отменить записи файла из очереди, дождаться начатой и удалить файл.
    /// Удаление идёт под замком очереди: новая запись этого файла не
    /// начнётся, пока он не удалён.
    pub(crate) fn remove(&self, path: &Path) {
        let mut q = self.shared.lock();
        q.jobs.retain(|j| j.path != path);
        while q.busy.as_deref() == Some(path) {
            q = self.shared.wait(q);
        }
        remove_file(path);
    }

    #[cfg(test)]
    pub(crate) fn writes(&self) -> u64 {
        self.shared.writes.load(Ordering::Relaxed)
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.shared.lock().closed = true;
        self.shared.cv.notify_all();
    }
}

fn run_writer(shared: &Shared) {
    loop {
        let job = {
            let mut q = shared.lock();
            loop {
                if let Some(job) = q.jobs.pop_front() {
                    q.busy = Some(job.path.clone());
                    break job;
                }
                if q.closed {
                    return;
                }
                q = shared.wait(q);
            }
        };
        // Паника в правке не должна оставить `busy` взведённым: ждущие
        // `flush` повисли бы навсегда.
        let written = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(job)))
            .unwrap_or_else(|_| {
                eprintln!("[synthos/syn_chat] правка файла чата упала, запись пропущена");
                false
            });
        if written {
            shared.writes.fetch_add(1, Ordering::Relaxed);
        }
        shared.lock().busy = None;
        shared.cv.notify_all();
    }
}

fn run_job(job: Job) -> bool {
    match job.op {
        Op::Put(chat) => write_chat(&job.path, &chat),
        Op::Patch(f) => {
            let Some(mut chat) = read_chat(&job.path) else {
                return false;
            };
            f(&mut chat);
            write_chat(&job.path, &chat)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn chat(id: &str, bodies: &[&str]) -> StoredChat {
        StoredChat {
            id: id.into(),
            title: "t".into(),
            messages: bodies.iter().map(|b| ChatMsg::user(*b)).collect(),
            ..StoredChat::default()
        }
    }

    fn bodies(path: &Path) -> Vec<String> {
        read_chat(path)
            .expect("файл чата")
            .messages
            .into_iter()
            .map(|m| m.body)
            .collect()
    }

    /// Правка, которую писатель держит, пока тест её не отпустит.
    /// Возвращает «отпустить» и «писатель внутри правки».
    fn held_patch(
        w: &Writer,
        path: &Path,
        body: &'static str,
    ) -> (mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (release, gate) = mpsc::channel::<()>();
        let (entered, started) = mpsc::channel::<()>();
        w.patch(
            path.to_path_buf(),
            Box::new(move |c| {
                let _ = entered.send(());
                let _ = gate.recv();
                c.messages.push(ChatMsg::user(body));
            }),
        );
        (release, started)
    }

    /// Записи одного файла не обгоняют друг друга; снимки, которые писатель
    /// ещё не взял, сворачиваются в последний.
    #[test]
    fn writes_of_one_file_keep_order_and_last_snapshot_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.json");
        let w = Writer::new();
        w.put(path.clone(), chat("c", &["v1"]));
        w.flush(&path);

        let (release, started) = held_patch(&w, &path, "p1");
        started.recv().unwrap();
        w.put(path.clone(), chat("c", &["v2"]));
        w.put(path.clone(), chat("c", &["v3"]));
        w.patch(
            path.clone(),
            Box::new(|c| c.messages.push(ChatMsg::user("p2"))),
        );
        release.send(()).unwrap();
        w.flush(&path);

        assert_eq!(bodies(&path), ["v3", "p2"]);
        assert_eq!(
            w.writes(),
            4,
            "v1, первая правка, один снимок вместо двух, вторая правка"
        );
        assert!(!tmp_path(&path).exists(), "временный файл переименован");
    }

    /// Удаление отменяет записи файла, ещё стоящие в очереди: иначе
    /// запоздавший автосейв воскресил бы удалённый чат.
    #[test]
    fn delete_cancels_writes_still_in_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.json");
        let other = dir.path().join("e.json");
        let w = Writer::new();
        w.put(path.clone(), chat("d", &["v1"]));
        w.put(other.clone(), chat("e", &["e1"]));
        w.flush_all();

        // Писатель занят другим файлом — автосейв удаляемого ждёт в очереди.
        let (release, started) = held_patch(&w, &other, "e2");
        started.recv().unwrap();
        w.put(path.clone(), chat("d", &["поздний автосейв"]));
        w.remove(&path);
        release.send(()).unwrap();
        w.flush_all();
        assert!(!path.exists(), "запоздавший автосейв не воскресил файл");
        assert_eq!(bodies(&other), ["e1", "e2"]);
    }

    /// Писатель посреди правки удаляемого файла: удаление ждёт её конца,
    /// иначе начатая запись легла бы уже после удаления.
    #[test]
    fn delete_waits_for_the_write_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.json");
        let w = Writer::new();
        w.put(path.clone(), chat("d", &["v1"]));
        w.flush(&path);

        let (release, started) = held_patch(&w, &path, "p1");
        started.recv().unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            release.send(()).unwrap();
        });
        w.remove(&path);
        releaser.join().unwrap();
        w.flush_all();
        assert!(!path.exists(), "начатая запись не легла поверх удаления");
        assert!(!tmp_path(&path).exists());
    }

    /// Правка фонового чата без файла пропускается, как раньше `load` → None.
    #[test]
    fn patch_of_a_missing_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("none.json");
        let w = Writer::new();
        w.patch(
            path.clone(),
            Box::new(|c| c.messages.push(ChatMsg::user("x"))),
        );
        w.flush(&path);
        assert!(!path.exists());
        assert_eq!(w.writes(), 0);
    }

    /// `load` видит правки фонового чата, ещё стоящие в очереди, — так
    /// `registry::select` читает ленту чата, ход которого шёл в фоне.
    #[test]
    fn load_waits_for_queued_background_edits() {
        let dir = tempfile::tempdir().unwrap();
        let _files = test_dir(dir.path());
        save_async(chat("l", &["вопрос"]));
        update_async("l", |c| c.messages.push(ChatMsg::user("из фона")));
        let loaded = load("l").expect("чат на диске");
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(list_meta().len(), 1);
        delete("l");
        assert!(load("l").is_none());
    }

    /// Файлы до 19.09.2026 без настроек получают общие настройки того
    /// времени один раз; у записанных позже ничего не меняется.
    #[test]
    fn startup_scan_fills_missing_settings_once() {
        let dir = tempfile::tempdir().unwrap();
        let _files = test_dir(dir.path());
        let own = ChatSettings {
            tools_active: vec!["web".into()],
            system_prompt: "свой".into(),
            ..ChatSettings::default()
        };
        save(&chat("old", &["давно"]));
        save(&StoredChat {
            settings: Some(own.clone()),
            ..chat("new", &["недавно"])
        });
        let global = ChatSettings {
            tools_active: vec!["bash".into(), "notes".into()],
            tools_auto: vec!["pipelines".into()],
            skills_active: vec!["tone".into()],
            system_prompt: "общий".into(),
            prompt_preset: "p1".into(),
        };

        assert_eq!(list_meta_filling(&global).len(), 2);
        assert_eq!(load("old").unwrap().settings.as_ref(), Some(&global));
        assert_eq!(load("new").unwrap().settings.as_ref(), Some(&own));
        assert_eq!(bodies(&chat_path("old")), ["давно"], "лента не тронута");

        // Повторный старт с другими общими настройками файлы не трогает.
        let other = ChatSettings { system_prompt: "другой".into(), ..ChatSettings::default() };
        list_meta_filling(&other);
        assert_eq!(load("old").unwrap().settings.as_ref(), Some(&global));
        // Обычный список (сборка мусора CAS) ничего не заполняет.
        save(&chat("plain", &[]));
        list_meta();
        assert!(load("plain").unwrap().settings.is_none());
    }
}
