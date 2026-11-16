#!/usr/bin/env bash
# Runs the Windows-side cargo.exe from WSL: `scripts/wcargo.sh test`, etc.
#
# Running cargo.exe directly on the \\wsl.localhost checkout fails with
# "incremental compilation: could not create session directory lock file",
# so the target dir is moved to %LOCALAPPDATA%\issen-wsl-target on the
# Windows filesystem (WSLENV's /p flag translates the path for cargo.exe).
set -euo pipefail

cargo_exe=$(command -v cargo.exe || true)
if [[ -z "$cargo_exe" ]]; then
    win_profile=$(cd /mnt/c && cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r')
    cargo_exe="$(wslpath "$win_profile")/.cargo/bin/cargo.exe"
fi
if [[ ! -x "$cargo_exe" ]]; then
    echo "wcargo: Windows cargo.exe not found (install rustup on Windows)" >&2
    exit 1
fi

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
    local_appdata=$(cd /mnt/c && cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')
    CARGO_TARGET_DIR="$(wslpath "$local_appdata")/issen-wsl-target"
fi
export CARGO_TARGET_DIR
export WSLENV="${WSLENV:+$WSLENV:}CARGO_TARGET_DIR/p"

exec "$cargo_exe" "$@"
