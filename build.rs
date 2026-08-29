//! Номер сборки для заголовка окна.
//!
//! `CARGO_PKG_VERSION` — это `0.1.0`, он меняется редко, а вот `pkgrel` в
//! `packaging/PKGBUILD` растёт с каждой пересборкой пакета и по нему видно,
//! какая именно сборка сейчас установлена. Пробрасываем его в бинарь, чтобы
//! титлбар мог показать пилюлю с номером.
//!
//! Пусто, если файла нет (сборка вне репозитория) — пилюля тогда просто не
//! рисуется, см. `components::titlebar`.

fn main() {
    println!("cargo:rerun-if-changed=packaging/PKGBUILD");

    let pkgrel = std::fs::read_to_string("packaging/PKGBUILD")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix("pkgrel=").map(str::to_owned))
        })
        .map(|v| v.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_default();

    println!("cargo:rustc-env=SYNTHOS_PKGREL={pkgrel}");
}
