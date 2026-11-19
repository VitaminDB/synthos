//! Номер сборки для заголовка окна.
//!
//! Версия synthos — `0.2.<номер сборки>` (`0.2.322`): `version` в `Cargo.toml`
//! и тег релиза `v0.2.322`. Выпуск — `syngui release <версия> --publish`.

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    println!("cargo:rustc-env=SYNTHOS_VERSION={version}");
}
