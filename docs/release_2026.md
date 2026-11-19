# Выпуск релиза

Сборка, пакеты и выпуск synthos делаются только утилитой `syngui` (крейт `syngui-cli`
в репозитории syngui). Свои PKGBUILD, скрипты и workflow GitHub Actions больше не
используются: с 19.11.2026 их нет (каталог `packaging/` удалён).

Описание выпуска — `[package.metadata.syngui]` в `Cargo.toml`: зависимости Arch,
optdepends, makedepends, `epoch`, состав релиза. Значок — `data/icons/synthos.svg`,
ярлык — `data/synthos.desktop`.

```sh
syngui package -f arch --install         # пакет Arch из текущего кода и установка в систему
syngui release 0.2.323                   # сухой прогон: что будет сделано
syngui release 0.2.323 --publish         # версия, коммит «Выпуск …», тег, архив x86_64,
                                         # git push, релиз GitHub, PKGBUILD для AUR
```

Версия — `0.2.<номер сборки>`: `version` в `[workspace.package]`, тег `v0.2.<номер>`,
она же в заголовке окна (`build.rs` → `SYNTHOS_VERSION`).

В AUR — `synthos-bin` (архив из релиза GitHub) и `synthos-git` (сборка из master
четырёх репозиториев: synthos, syngui, synaptix, synshell). `epoch=1` обязателен: с
20.09 по 23.09.2026 версией был голый номер сборки (268…278), и без epoch pacman счёл
бы 0.2.x откатом. PKGBUILD с суммами `syngui release --publish` кладёт в
`target/syngui/aur/`; оттуда они пушатся в `ssh://aur@aur.archlinux.org/<пакет>.git`.

Веса моделей в релиз не входят — скачиваются из Hugging Face внутри приложения.
