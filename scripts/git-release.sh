#!/bin/bash
set -xeuo pipefail
ROOT="$(git rev-parse --show-toplevel)"
cd "${ROOT}"
${ROOT}/scripts/bump-version.sh
git add .
git commit -m 'cleanup and bump version'
git push
${ROOT}/scripts/release.sh
