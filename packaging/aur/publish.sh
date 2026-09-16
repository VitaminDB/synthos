#!/usr/bin/env bash
# Публикация пакета в AUR.
#
#   packaging/aur/publish.sh synthos-bin      # обновить и запушить bin-пакет
#   packaging/aur/publish.sh synthos-git
#   packaging/aur/publish.sh synthos-bin --dry-run
#
# Что делает: клонирует ssh://aur@aur.archlinux.org/<pkg>.git во временную
# директорию, копирует туда PKGBUILD из этого репозитория, для bin-пакета
# пересчитывает sha256 по опубликованному релизу, генерирует .SRCINFO,
# коммитит и пушит.
#
# Требуется один раз: аккаунт на https://aur.archlinux.org, публичный SSH-ключ
# в профиле AUR и в ~/.ssh/config:
#
#   Host aur.archlinux.org
#       User aur
#       IdentityFile ~/.ssh/aur

set -euo pipefail

pkg="${1:-}"
dry="${2:-}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "$pkg" in
    synthos-bin|synthos-git) ;;
    *) echo "использование: $0 {synthos-bin|synthos-git} [--dry-run]" >&2; exit 1 ;;
esac

[ -f "$here/$pkg/PKGBUILD" ] || { echo "нет $here/$pkg/PKGBUILD" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "→ клонирую AUR-репозиторий $pkg"
if ! git clone --quiet "ssh://aur@aur.archlinux.org/$pkg.git" "$work/$pkg" 2>/dev/null; then
    echo "  репозитория ещё нет — создаю первый коммит (AUR заводит пакет при push)"
    git init --quiet "$work/$pkg"
    git -C "$work/$pkg" remote add origin "ssh://aur@aur.archlinux.org/$pkg.git"
fi

cp "$here/$pkg/PKGBUILD" "$work/$pkg/PKGBUILD"
[ -f "$here/$pkg/.gitignore" ] && cp "$here/$pkg/.gitignore" "$work/$pkg/.gitignore"

cd "$work/$pkg"

if [ "$pkg" = "synthos-bin" ]; then
    echo "→ пересчитываю sha256 по релизному tarball"
    updpkgsums
fi

echo "→ .SRCINFO"
makepkg --printsrcinfo > .SRCINFO

# Копия обновлённого PKGBUILD (updpkgsums меняет суммы) возвращается в репозиторий,
# чтобы источник правды и AUR не расходились.
cp PKGBUILD "$here/$pkg/PKGBUILD"

pkgver="$(grep -m1 '^pkgver=' PKGBUILD | cut -d= -f2)"
pkgrel="$(grep -m1 '^pkgrel=' PKGBUILD | cut -d= -f2)"

git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
    echo "изменений нет — публиковать нечего"
    exit 0
fi

git -c user.name="Alexeyev Vitaly" -c user.email="vitamindbnfkz@gmail.com" \
    commit --quiet -m "$pkgver-$pkgrel"

if [ "$dry" = "--dry-run" ]; then
    echo "--dry-run: коммит готов, push пропущен. Дифф:"
    git show --stat HEAD
    exit 0
fi

echo "→ push в AUR"
git push origin HEAD:master
echo "готово: https://aur.archlinux.org/packages/$pkg"
