#!/usr/bin/env bash
# Python 3.11+ is also required by bump-version.sh.
set -euo pipefail
exec python3 "$(dirname -- "${BASH_SOURCE[0]}")/release_flow.py"  --skip-windows "$@"
