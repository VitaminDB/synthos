fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match synthos::cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("synthos: {e}\n\n{}", synthos::cli::USAGE);
            std::process::exit(2);
        }
    };
    if cli.help {
        println!("{}", synthos::cli::USAGE);
        return;
    }
    if cli.version {
        println!("synthos {}", env!("SYNTHOS_VERSION"));
        return;
    }
    if cli.list_terminals {
        synthos::cli::print_terminals();
        return;
    }
    // Хранитель терминалов: тот же бинарник, без интерфейса и движка.
    if let Some(socket) = cli.pty_holder.as_deref() {
        if let Err(e) = synthos::pages::code_editor::terminal_holder::run(socket) {
            eprintln!("synthos --pty-holder: {e}");
            std::process::exit(1);
        }
        return;
    }
    // synthos уже запущен — передать ему аргументы и поднять его окно, не
    // поднимая здесь движок и интерфейс.
    let forward = cli.to_forward_args();
    if syngui::app::forward_to_running_instance("synthos", &forward).is_ok() {
        if !forward.is_empty() {
            println!("synthos: передано запущенному synthos");
        }
        return;
    }
    // Запуск из службы (например сессией агента под syncourier.service):
    // setsid/disown не выводят из её cgroup, и `systemctl restart` службы
    // убил бы synthos со всеми вкладками (22.11). Уходим в свой scope.
    match syngui::platform::cgroup::escape_service_cgroup("synthos") {
        Ok(Some(unit)) => eprintln!("synthos: запущен из службы — перенесён в {unit}"),
        Ok(None) => {}
        Err(e) => eprintln!("synthos: не удалось покинуть cgroup службы: {e}"),
    }
    synthos::cli::set_startup_args(forward);
    synthos::migrate::migrate_user_data();
    synthos::run_desktop();
}
