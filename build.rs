//! Номер сборки для заголовка окна.
//!
//! Версия synthos — `0.2.<номер сборки>` (`0.2.278`), она же `pkgver` в
//! `packaging/PKGBUILD`, `version` в `Cargo.toml` и тег релиза `v0.2.278`.
//! `SYNTHOS_VERSION` — `pkgver` из PKGBUILD (источник правды для пакета), а
//! вне репозитория (cargo install, tarball с исходниками) — `CARGO_PKG_VERSION`.
//!
//! `SYNTHOS_PKGREL` — ревизия пакета Arch. Обычно `1` и никому не интересна;
//! растёт только при пересборке того же кода под новые библиотеки (например,
//! мажорный ffmpeg). Пусто при `1` или вне репозитория — пилюля тогда не
//! рисуется, см. `components::titlebar`.

fn main() {
    println!("cargo:rerun-if-changed=packaging/PKGBUILD");

    let pkgbuild = std::fs::read_to_string("packaging/PKGBUILD").ok();
    let field = |key: &str| -> Option<String> {
        pkgbuild.as_ref()?.lines().find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .map(|v| v.trim().trim_matches(['"', '\'']).to_owned())
        })
    };

    let cargo_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let version = field("pkgver=")
        .filter(|v| !v.is_empty())
        .unwrap_or(cargo_version);

    // Ревизия пакета: показывать имеет смысл только вторую и следующие.
    let pkgrel = field("pkgrel=")
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) && v != "1")
        .unwrap_or_default();

    println!("cargo:rustc-env=SYNTHOS_VERSION={version}");
    println!("cargo:rustc-env=SYNTHOS_PKGREL={pkgrel}");
}
