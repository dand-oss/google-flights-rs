#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd -- "$repo_dir"
cargo install --path . --root ~/.local --locked --force

skill_source="$repo_dir/skills/google-flights-rs"
skill_target="$(cd ~ && pwd)/.claude/skills/google-flights-rs"

if [[ ! -d "$skill_source" ]]; then
    printf 'error: skill directory not found: %s\n' "$skill_source" >&2
    exit 1
fi

mkdir -p -- "$(dirname -- "$skill_target")"

if [[ -L "$skill_target" ]]; then
    rm -- "$skill_target"
elif [[ -e "$skill_target" && ! -d "$skill_target" ]]; then
    printf 'error: refusing to replace existing non-directory: %s\n' "$skill_target" >&2
    exit 1
fi

mkdir -p -- "$skill_target"
cp -a -- "$skill_source/." "$skill_target/"

printf 'Installed skill files into: %s\n' "$skill_target"
