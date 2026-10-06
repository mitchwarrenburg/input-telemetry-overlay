#!/usr/bin/env bash
# macOS / POSIX launcher. All repositories share this user's isolated environment.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
venv="${XDG_CACHE_HOME:-$HOME/.cache}/home-network/venv-v1"
if [[ "${1:-}" == setup ]]; then
  command -v python3 >/dev/null || { echo 'Install Python 3.9+ before setup.' >&2; exit 1; }
  python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else "Python 3.9+ is required.")'
  python3 -m venv "$venv"
  "$venv/bin/python" -m pip install -r "$script_dir/home-network-requirements.txt"
  echo "Home-network tools installed in $venv"
  exit 0
fi
if [[ -x "$venv/bin/python" ]]; then
  exec "$venv/bin/python" "$script_dir/home_network.py" "$@"
fi
command -v python3 >/dev/null || { echo 'Install Python 3.9+, then run this script with setup.' >&2; exit 1; }
exec python3 "$script_dir/home_network.py" "$@"
