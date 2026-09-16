# AUR-пакеты synthos

Источник правды для двух пакетов в AUR. Правки делаются здесь, в AUR уезжают
скриптом `publish.sh` — так PKGBUILD живёт в истории проекта, а не только в
чужом git.

| Пакет | Что делает | Кому |
|---|---|---|
| `synthos-bin` | ставит готовый бинарь из GitHub Releases | всем: установка за секунды |
| `synthos-git` | собирает из исходников трёх репозиториев | тем, кому нужен свежий master; нужен CUDA toolkit, ~2 ч и ~15 ГБ диска |

## Первая публикация (один раз)

1. Завести аккаунт на <https://aur.archlinux.org>.
2. Добавить публичный SSH-ключ в профиль (`My Account` → `SSH Public Key`).
3. Прописать в `~/.ssh/config`:

   ```
   Host aur.archlinux.org
       User aur
       IdentityFile ~/.ssh/aur
   ```

4. Убедиться, что релиз с нужным тегом уже опубликован — `synthos-bin` качает
   tarball именно оттуда, и без него `updpkgsums` не посчитает суммы.

## Выпуск новой версии

```sh
# 1. пины зависимостей и версия
packaging/pin-deps.sh
$EDITOR packaging/PKGBUILD          # pkgver/pkgrel
$EDITOR Cargo.toml                  # version — должна совпасть с pkgver

git commit -am "релиз v0.2.0"
git tag v0.2.0 && git push --follow-tags

# 2. дождаться, пока workflow release соберёт пакет и выложит артефакты

# 3. обновить версию в AUR-пакетах и запушить
$EDITOR packaging/aur/synthos-bin/PKGBUILD     # pkgver, pkgrel=1
packaging/aur/publish.sh synthos-bin
packaging/aur/publish.sh synthos-git           # pkgver считается из git автоматически
```

`--dry-run` вторым аргументом готовит коммит, но не пушит — удобно проверить
дифф перед первой публикацией.

## Проверка пакета до публикации

```sh
cd packaging/aur/synthos-bin
makepkg -si                 # соберёт и поставит локально
namcap PKGBUILD             # линтер PKGBUILD (пакет extra/namcap)
namcap ./*.pkg.tar.zst      # линтер собранного пакета
```

Для `synthos-git` то же самое, но сборка займёт часы: makepkg клонирует три
репозитория и компилирует ~540k строк Rust.

## Что важно не сломать

- **Веса моделей в пакет не входят.** Ни в bin, ни в git — они качаются в
  приложении из Hugging Face, лицензию принимает пользователь. Любой `source=`
  с весами сделает пакет нераспространяемым.
- **`depends` на ffmpeg по soname.** Мажорный апдейт ffmpeg меняет soname;
  зависимость по soname заставляет pacman пересобрать/обновить пакет вместо
  того, чтобы отдать падающий бинарь. Соответственно после каждого мажорного
  ffmpeg нужен новый релиз с пересборкой.
- **`conflicts`/`provides`.** Три пакета (`synthos`, `-bin`, `-git`) ставят один
  и тот же `/usr/bin/synthos`, поэтому конфликтуют явно.
- **deps.lock.** `synthos-git` вычитывает из него коммиты syngui и synaptix:
  master всех трёх репозиториев между собой не гарантированно совместим.
