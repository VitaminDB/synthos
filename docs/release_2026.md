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

## Runner на этой машине (установлен 17.09.2026)

- Каталог: `~/actions-runner` (версия 2.337.0, SHA-256 архива сверена с релизом
  actions/runner), имя `synthos-builder`, метки `self-hosted, Linux, X64`.
- Сервис — **пользовательский** systemd, без sudo:
  `~/.config/systemd/user/actions-runner-synthos.service`. Живёт без входа в
  сессию благодаря `loginctl enable-linger` (уже включён). `OOMScoreAdjust=500`.
- **PATH задаётся в двух местах, и `.path` — не одно из них.** У сервиса systemd
  даёт `PATH=/usr/local/bin:/usr/bin`, а `~/actions-runner/.path` на job не
  распространяется: 20.09 сборка упала на `build.rs` форка cudarc с
  «`nvcc --version` failed … NotFound». Теперь PATH прописан
  `Environment=` в `~/.config/systemd/user/actions-runner-synthos.service` и
  продублирован шагом «Окружение сборки» в `release.yml` (`$GITHUB_PATH`) —
  второе версионируется в репозитории и переживает переустановку раннера.
- **Рестарт сервиса оставляет старый `Runner.Listener` сиротой** (`KillMode=process`
  убивает только `run.sh`). Два слушателя на одну регистрацию — путаница в
  очереди; после `systemctl --user restart` проверяйте
  `ps -eo pid,cmd | grep actions-runner/bin/Runner.Listener` и убивайте лишний.
  Осторожно с `pkill -f Runner.Listener`: шаблон совпадает и с собственной
  командной строкой шелла.

```sh
systemctl --user status actions-runner-synthos        # жив ли
journalctl --user -u actions-runner-synthos -f        # лог раннера
systemctl --user disable --now actions-runner-synthos # выключить совсем
```

Переустановка с нуля (токен берётся через API, веб-интерфейс не нужен):

```sh
cd ~/actions-runner
./config.sh remove --token "$(gh api -X POST repos/VitaminDB/synthos/actions/runners/remove-token --jq .token)"
./config.sh --unattended --url https://github.com/VitaminDB/synthos --name synthos-builder --work _work \
  --token "$(gh api -X POST repos/VitaminDB/synthos/actions/runners/registration-token --jq .token)"
systemctl --user restart actions-runner-synthos
```

### Безопасность на публичном репозитории

Self-hosted runner на публичном репозитории — это чужой код на вашей машине,
если дать ему такую возможность. Закрыто тремя вещами:

1. В `release.yml` нет триггера `pull_request` — только тег и `workflow_dispatch`.
   Форк не может запустить сборку.
2. Одобрение workflow из форков — `all_external_contributors` (выставлено 17.09 через
   `gh api -X PUT repos/VitaminDB/synthos/actions/permissions/fork-pr-contributor-approval`;
   по умолчанию было `first_time_contributors` — после одного принятого PR чужой код
   запускался бы на этой машине без спроса).
3. Раннер работает от обычного пользователя, не от root.

## Нумерация (с 20.09.2026)

Версия synthos — **сквозной номер сборки**: `268`, `269`, `270`. Тот самый
номер, что показывает титлбар; он же `pkgver`, он же тег релиза `v268`, он же
версия пакета в AUR (`synthos-bin 268-1`). Semver у приложения, которое
выпускается по несколько раз в неделю и не имеет публичного API, только мешал:
раньше номер сборки жил в `pkgrel` (`0.2.0-267`), и релиз в AUR назывался
`0.2.0-1`, то есть не совпадал ни с чем.

Где номер лежит:

| Место | Как выглядит |
|---|---|
| `Cargo.toml` | `version = "268.0.0"` — Cargo принимает только semver, номер живёт в мажоре |
| `packaging/PKGBUILD` | `pkgver=268`, `pkgrel=1` |
| тег и релиз | `v268` |
| AUR | `synthos-bin 268-1`, `synthos-git 268.rN.g<sha>` |
| в приложении | `Synthos v268` — `build.rs` читает `pkgver` из PKGBUILD в `SYNTHOS_VERSION` |

`pkgrel` остался тем, чем он является в Arch: ревизией пакета. Поднимается,
только если **тот же** код пересобирается под новые библиотеки (мажорный
ffmpeg), и сбрасывается в `1` вместе с новым `pkgver`. Пилюля в титлбаре
показывает его лишь тогда, когда он больше единицы.

Workflow сверяет мажор `Cargo.toml` с `pkgver` и `pkgver` с тегом — расхождение
останавливает релиз.

## Как выпускается релиз

```sh
packaging/pin-deps.sh                    # SHA syngui и synaptix → deps.lock
$EDITOR Cargo.toml packaging/PKGBUILD    # version = "268.0.0" ↔ pkgver=268, pkgrel=1
git commit -am "релиз v268"
git tag v268
git push origin master v268        # именно так: --follow-tags легковесный тег не пушит
```

Дальше workflow сам: клонирует три репозитория по пинам, сверяет тег с версией
(расхождение `Cargo.toml` и `PKGBUILD` — ошибка, а не тихий кривой пакет),
собирает, гоняет `makepkg`, пакует tarball, считает `SHA256SUMS` и создаёт
релиз.

Ручной прогон без тега: `Actions → release → Run workflow`, ref = `master`.
Такая сборка уезжает в релиз `v<версия>-build-<коммит>` (имя настоящего релиза
она не занимает) и по умолчанию черновиком.

## Грабли

- **Сборка и запущенный synthos не уживаются в RAM.** Flash-Next держит 66 ГБ
  pinned из 93; параллельная сборка получала OOM, и убивало обычно не её.
  В workflow шаг сборки поднимает себе `oom_score_adj` (`choom -n 1000`), так
  что первой падает сборка. Во время релиза лучше не держать модель загруженной.
- **`CARGO_BUILD_JOBS: 16`** из 24 ядер — чтобы рабочий стол оставался живым.
  Ограничивать до 4–8 не нужно: это только удлиняет сборку.
- **Проверка mtime в PKGBUILD.** Локально она ловит «подняли pkgver после
  сборки» (номер зашивается в бинарь на компиляции); в CI checkout делает файлы свежее бинаря при инкрементальной сборке,
  поэтому шаг `makepkg` делает `touch` бинаря — сборка в том же job только что
  прошла.
- **`--locked` не используется.** Cargo.lock synthos зависит от версий крейтов в
  syngui/synaptix; при смене пина `--locked` уронит релиз на ровном месте.

## AUR

См. [`packaging/aur/README.md`](../packaging/aur/README.md): `synthos-bin`
(готовый бинарь из релиза) и `synthos-git` (сборка master всех трёх репозиториев;
пины `deps.lock` — только для релиза по тегу). Публикация — `packaging/aur/publish.sh <пакет>`.
