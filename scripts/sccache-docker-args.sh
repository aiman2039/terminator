#!/bin/sh
# Prints `docker run` arguments that route rustc in a container through the
# host's static sccache binary (.github/actions/sccache) and its R2 settings.
# Prints nothing when sccache is off. Paths contain no spaces.
set -eu
[ "${RUSTC_WRAPPER:-}" = sccache ] || exit 0
bin="$(command -v sccache)"
echo "-v $bin:/usr/local/bin/sccache:ro -e RUSTC_WRAPPER=/usr/local/bin/sccache -e CARGO_INCREMENTAL=0 -e SCCACHE_BUCKET -e SCCACHE_ENDPOINT -e SCCACHE_REGION -e AWS_ACCESS_KEY_ID -e AWS_SECRET_ACCESS_KEY"
