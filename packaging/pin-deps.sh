#!/usr/bin/env bash
# Записывает в packaging/deps.lock текущие HEAD соседних репозиториев syngui и
# synaptix — чтобы релизная сборка на runner'е собрала ровно то, что собрано
# локально.
#
# Запускать перед тегом:
#   packaging/pin-deps.sh && git commit -am "релиз v0.2.0: пины зависимостей"
#
# Скрипт отказывается писать пин, если в соседнем репозитории есть
# незакоммиченные правки или коммит не запушен: и то и другое означает, что
# runner склонирует не тот код, который вы проверяли.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock="$root/packaging/deps.lock"
parent="$(dirname "$root")"

fail=0
pins=""

# Функция вызывается напрямую, а не через $(...): в подстановке команд
# присваивание fail=1 ушло бы в subshell и потерялось.
pin() {
    local name="$1" repo="$parent/$1"

    if [ ! -d "$repo/.git" ]; then
        echo "✗ $name: нет репозитория в $repo" >&2
        fail=1
        return
    fi

    local dirty sha pushed
    dirty="$(git -C "$repo" status --porcelain)"
    sha="$(git -C "$repo" rev-parse HEAD)"
    pushed="$(git -C "$repo" branch -r --contains "$sha" 2>/dev/null | head -1)"

    if [ -n "$dirty" ]; then
        echo "✗ $name: незакоммиченные правки — runner их не увидит:" >&2
        echo "$dirty" | sed 's/^/    /' >&2
        fail=1
    fi

    if [ -z "$pushed" ]; then
        echo "✗ $name: коммит $sha не запушен на remote" >&2
        fail=1
    fi

    pins+="$name=$sha"$'\n'
}

pin syngui
pin synaptix

if [ "$fail" -ne 0 ]; then
    echo "" >&2
    echo "deps.lock не обновлён." >&2
    exit 1
fi

{
    grep -E '^#|^$' "$lock" | sed '/^$/d'
    echo ""
    printf '%s' "$pins"
} > "$lock.tmp"
mv "$lock.tmp" "$lock"

echo "deps.lock обновлён:"
printf '%s' "$pins" | sed 's/^/    /'
