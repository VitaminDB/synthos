# Выпуск релиза: self-hosted runner, GitHub Actions, AUR

Дата: 16.09.2026.

Сборка synthos не влезает в бесплатный GitHub-раннер: три репозитория, ~540k
строк Rust, LTO и `codegen-units = 1`, плюс CUDA toolkit ради `nvcc` и системный
FFmpeg. И главное — бинарь должен линковаться с библиотеками **Arch**, иначе
пакет для Arch собирать бессмысленно. Поэтому релизы собирает self-hosted
runner на рабочей машине.

## Раскладка

```
_work/synthos/
├── synthos/     ← checkout репозитория (тег)
├── syngui/      ← checkout по SHA из packaging/deps.lock
├── synaptix/    ← то же
└── dist/        ← артефакты релиза
_work/synthos-ci-target/   ← CARGO_TARGET_DIR, переживает очистку workspace
```

Путь `../syngui/syngui` из манифеста synthos разрешается сам собой: соседние
директории — ровно то, что ждут path-зависимости.

## Установка runner'а (один раз)

```sh
# 1. Токен: Settings → Actions → Runners → New self-hosted runner
mkdir -p ~/actions-runner && cd ~/actions-runner
curl -o actions-runner.tar.gz -L \
  https://github.com/actions/runner/releases/latest/download/actions-runner-linux-x64.tar.gz
tar xzf actions-runner.tar.gz

# 2. Регистрация (метки Linux/X64/self-hosted ставятся сами)
./config.sh --url https://github.com/VitaminDB/synthos --token <TOKEN>

# 3. Как systemd-сервис от своего пользователя
sudo ./svc.sh install "$USER"
sudo ./svc.sh start
```

Проверка: `Settings → Actions → Runners` — статус `Idle`.

### Безопасность на публичном репозитории

Self-hosted runner на публичном репозитории — это чужой код на вашей машине,
если дать ему такую возможность. Закрыто тремя вещами:

1. В `release.yml` нет триггера `pull_request` — только тег и `workflow_dispatch`.
   Форк не может запустить сборку.
2. `Settings → Actions → General → Fork pull request workflows`: оставить
   «Require approval for all external contributors».
3. Раннер работает от обычного пользователя, не от root.

## Как выпускается релиз

```sh
packaging/pin-deps.sh                    # SHA syngui и synaptix → deps.lock
$EDITOR Cargo.toml packaging/PKGBUILD    # version == pkgver, pkgrel +1
git commit -am "релиз v0.2.0"
git tag v0.2.0
git push --follow-tags
```

Дальше workflow сам: клонирует три репозитория по пинам, сверяет тег с версией
(расхождение `Cargo.toml` и `PKGBUILD` — ошибка, а не тихий кривой пакет),
собирает, гоняет `makepkg`, пакует tarball, считает `SHA256SUMS` и создаёт
релиз.

Ручной прогон без тега: `Actions → release → Run workflow`, ref = `master`.
Такая сборка уезжает в релиз `v<версия>-build<pkgrel>` и по умолчанию черновиком.

## Грабли

- **Сборка и запущенный synthos не уживаются в RAM.** Flash-Next держит 66 ГБ
  pinned из 93; параллельная сборка получала OOM, и убивало обычно не её.
  В workflow шаг сборки поднимает себе `oom_score_adj` (`choom -n 1000`), так
  что первой падает сборка. Во время релиза лучше не держать модель загруженной.
- **`CARGO_BUILD_JOBS: 16`** из 24 ядер — чтобы рабочий стол оставался живым.
  Ограничивать до 4–8 не нужно: это только удлиняет сборку.
- **Проверка mtime в PKGBUILD.** Локально она ловит «подняли pkgrel после
  сборки»; в CI checkout делает файлы свежее бинаря при инкрементальной сборке,
  поэтому шаг `makepkg` делает `touch` бинаря — сборка в том же job только что
  прошла.
- **`--locked` не используется.** Cargo.lock synthos зависит от версий крейтов в
  syngui/synaptix; при смене пина `--locked` уронит релиз на ровном месте.

## AUR

См. [`packaging/aur/README.md`](../packaging/aur/README.md): `synthos-bin`
(готовый бинарь из релиза) и `synthos-git` (сборка master всех трёх репозиториев;
пины `deps.lock` — только для релиза по тегу). Публикация — `packaging/aur/publish.sh <пакет>`.
