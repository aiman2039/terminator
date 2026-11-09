#!/bin/sh
# Tests R2 credentials the way CI uses them (.github/actions/sccache), so they
# can be checked before updating the GitHub secrets.
#
# Usage: scripts/test-r2-sccache.sh [env-file]
#
# Reads SCCACHE_BUCKET, SCCACHE_ENDPOINT, R2_ACCESS_KEY_ID and
# R2_SECRET_ACCESS_KEY from env-file (default: .env in the repo root, which Git
# ignores), falling back to the environment. Exits 0 only if every check passes.
#
# Writes to the bucket: one probe object, which is deleted, and one small
# sccache entry per run, which stays until the bucket is cleaned.
set -u
cd "$(dirname "$0")/.." || exit 2

names="SCCACHE_BUCKET SCCACHE_ENDPOINT R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY"
env_file="${1:-.env}"
failures=0

pass() { printf '  ok    %s\n' "$1"; }
warn() { printf '  warn  %s\n' "$1"; }
fail() {
    printf '  FAIL  %s\n' "$1"
    failures=$((failures + 1))
}
section() { printf '\n%s\n' "$1"; }
note() { printf '        %s\n' "$1"; }

# Replaces the credentials wherever they appear in tool output.
redact() {
    awk '
        function mask(text, needle, label, at) {
            while (needle != "" && (at = index(text, needle)) > 0)
                text = substr(text, 1, at - 1) label substr(text, at + length(needle))
            return text
        }
        {
            line = mask($0, ENVIRON["R2_SECRET_ACCESS_KEY"], "<secret>")
            print mask(line, ENVIRON["R2_ACCESS_KEY_ID"], "<access-key>")
        }'
}

hint() {
    case $1 in
    *SignatureDoesNotMatch*)
        note "hint: the secret does not belong to this access key. Copy both from the same R2 token." ;;
    *InvalidAccessKeyId* | *Credential\ access\ key* | *Unauthorized*)
        note "hint: R2 does not accept this access key. Use the S3 'Access Key ID', not the API token value." ;;
    *NoSuchBucket* | *"(404)"* | *"status: 404"*)
        note "hint: no such bucket on this account. Check SCCACHE_BUCKET and the account ID in SCCACHE_ENDPOINT." ;;
    *AccessDenied* | *Forbidden* | *"(403)"* | *"status: 403"*)
        note "hint: the keys were rejected or lack access. The token needs 'Object Read & Write' on this bucket." ;;
    *"Could not connect"* | *"Connect timeout"* | *"dns error"* | *"Name or service"*)
        note "hint: cannot reach the endpoint. Check the account ID in SCCACHE_ENDPOINT." ;;
    esac
}

# Runs a command and reports it; on failure prints its output and a hint.
attempt() {
    label=$1
    shift
    if output="$("$@" 2>&1)"; then
        pass "$label"
        return 0
    fi
    fail "$label"
    printf '%s\n' "$output" | redact | sed 's/^/        /'
    hint "$output"
    return 1
}

load_from_file() {
    line="$(grep -E "^[[:space:]]*(export[[:space:]]+)?$1=" "$env_file" | tail -n 1)"
    [ -n "$line" ] || return 0
    value=${line#*=}
    case $value in
    \"*\") value=${value#\"} value=${value%\"} ;;
    \'*\') value=${value#\'} value=${value%\'} ;;
    esac
    export "$1=$value"
}

matches() { printf '%s' "$1" | grep -Eq "$2"; }

# Checks a value against its expected format without printing it.
check_format() {
    name=$1 pattern=$2 expected=$3
    eval "value=\${$name:-}"
    case $value in
    *[[:space:]]*) fail "$name contains whitespace or a line ending; re-copy it" ;;
    *)
        if matches "$value" "$pattern"; then
            pass "$name format"
        else
            fail "$name should be $expected (got ${#value} characters)"
        fi
        ;;
    esac
}

s3() {
    aws --endpoint-url "$SCCACHE_ENDPOINT" --region auto --no-cli-pager \
        --cli-connect-timeout 10 --cli-read-timeout 20 s3api "$@"
}

stats() { sccache --show-stats --stats-format=json 2>/dev/null; }
stat() { stats | jq -r "$1"; }

compile_probe() {
    rm -rf "$work/out"
    mkdir -p "$work/out"
    sccache rustc --edition=2021 --crate-name r2probe --crate-type lib \
        --emit=dep-info,link -C debuginfo=0 --out-dir "$work/out" "$work/r2probe.rs"
}

summarize() {
    section "Result"
    if [ "$failures" -eq 0 ]; then
        echo "  PASS: these values work with sccache."
        echo "  These are local values. Update the GitHub secrets with:"
        for name in $names; do
            echo "    gh secret set $name --repo aiman2039/terminator"
        done
        exit 0
    fi
    echo "  FAILED: $failures check(s). Fix the values and run this again."
    exit 1
}

for tool in aws curl jq rustc sccache; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "missing required tool: $tool" >&2
        exit 2
    fi
done

section "Values"
if [ -f "$env_file" ]; then
    for name in $names; do load_from_file "$name"; done
    echo "  reading $env_file, then the environment"
elif [ $# -gt 0 ]; then
    echo "env file not found: $env_file" >&2
    exit 2
else
    echo "  no .env file; reading the environment"
fi

for name in $names; do
    eval "value=\${$name:-}"
    if [ -z "$value" ]; then
        echo "  FAIL  $name is not set"
        failures=$((failures + 1))
    fi
done
[ "$failures" -eq 0 ] || summarize

check_format SCCACHE_BUCKET '^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$' \
    "a bare bucket name, with no path or URL"
check_format SCCACHE_ENDPOINT '^https://[0-9a-f]{32}(\.[a-z]+)?\.r2\.cloudflarestorage\.com/?$' \
    "https://<account-id>.r2.cloudflarestorage.com, with no bucket path"
check_format R2_ACCESS_KEY_ID '^[0-9a-f]{32}$' "32 hex characters (the S3 Access Key ID)"
check_format R2_SECRET_ACCESS_KEY '^[0-9a-f]{64}$' "64 hex characters (the S3 Secret Access Key)"
echo "  using bucket $SCCACHE_BUCKET at $SCCACHE_ENDPOINT"
echo "  using access key $(printf '%s' "$R2_ACCESS_KEY_ID" | cut -c1-4)...$(printf '%s' "$R2_ACCESS_KEY_ID" | rev | cut -c1-4 | rev)"

work="$(mktemp -d "${TMPDIR:-/tmp}/r2-sccache.XXXXXX")"
nonce="$(basename "$work" | cut -d. -f2)-$$"
server_started=0
cleanup() {
    [ "$server_started" -eq 0 ] || sccache --stop-server >/dev/null 2>&1
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Isolate from any AWS profile, sccache config or sccache server on this machine.
: >"$work/empty"
unset AWS_PROFILE AWS_SESSION_TOKEN AWS_ENDPOINT_URL AWS_ENDPOINT_URL_S3 SCCACHE_SERVER_UDS RUSTC_WRAPPER
unset SCCACHE_S3_KEY_PREFIX SCCACHE_S3_NO_CREDENTIALS SCCACHE_S3_ENABLE_VIRTUAL_HOST_STYLE
export AWS_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID" AWS_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export AWS_CONFIG_FILE="$work/empty" AWS_SHARED_CREDENTIALS_FILE="$work/empty"
export AWS_EC2_METADATA_DISABLED=true AWS_MAX_ATTEMPTS=2
# R2 does not support every checksum that recent AWS CLIs send by default.
export AWS_REQUEST_CHECKSUM_CALCULATION=when_required AWS_RESPONSE_CHECKSUM_VALIDATION=when_required
export R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY

section "Endpoint"
http_code="$(curl -sS -o /dev/null --max-time 15 -w '%{http_code}' "$SCCACHE_ENDPOINT" 2>"$work/curl.err")"
if [ "${http_code:-000}" = 000 ]; then
    fail "cannot reach $SCCACHE_ENDPOINT"
    redact <"$work/curl.err" | sed 's/^/        /'
    summarize
fi
pass "reachable (HTTP $http_code without credentials)"

section "Bucket access (aws cli)"
probe_key="sccache-r2-test/$nonce"
printf 'r2 probe %s\n' "$nonce" >"$work/probe.put"
attempt "head bucket" s3 head-bucket --bucket "$SCCACHE_BUCKET"
if attempt "write object" s3 put-object --bucket "$SCCACHE_BUCKET" --key "$probe_key" --body "$work/probe.put"; then
    if attempt "read object" s3 get-object --bucket "$SCCACHE_BUCKET" --key "$probe_key" "$work/probe.get"; then
        if cmp -s "$work/probe.put" "$work/probe.get"; then
            pass "read matches write"
        else
            fail "object read back differs from what was written"
        fi
    fi
    if ! s3 delete-object --bucket "$SCCACHE_BUCKET" --key "$probe_key" >/dev/null 2>&1; then
        warn "could not delete probe object $probe_key; remove it by hand"
    fi
fi

section "sccache"
ci_version="$(sed -n 's/^ *version: v//p' .github/actions/sccache/action.yml)"
local_version="$(sccache --version | awk '{print $2}')"
if [ "$local_version" = "$ci_version" ]; then
    pass "version $local_version matches CI"
else
    warn "local sccache is $local_version but CI pins $ci_version"
fi

: >"$work/sccache.conf"
export SCCACHE_REGION=auto SCCACHE_CONF="$work/sccache.conf" SCCACHE_DIR="$work/local"
export SCCACHE_LOG=warn SCCACHE_ERROR_LOG="$work/server.log"
SCCACHE_SERVER_PORT="$(awk 'BEGIN { srand(); print 20000 + int(rand() * 20000) }')"
export SCCACHE_SERVER_PORT

attempt "server starts on R2" sccache --start-server || summarize
server_started=1

location="$(stat '.cache_location')"
case $location in
*[Ss]3*) pass "cache location: $location" ;;
*)
    fail "cache location is not S3/R2: $location"
    summarize
    ;;
esac

printf 'pub const RUN: &str = "%s";\n' "$nonce" >"$work/r2probe.rs"
attempt "first compile" compile_probe || summarize

# sccache uploads in the background after the compile returns.
waited=0
while [ "$waited" -lt 30 ]; do
    [ "$(stat '.stats.cache_writes + .stats.cache_write_errors')" -gt 0 ] && break
    sleep 1
    waited=$((waited + 1))
done
if [ "$(stat '.stats.cache_writes')" -gt 0 ]; then
    pass "compile output uploaded to R2"
else
    fail "compile output did not upload ($(stat '.stats.cache_write_errors') write errors)"
    grep -E 'ERROR|WARN' "$work/server.log" | tail -n 15 | redact | sed 's/^/        /'
    summarize
fi

attempt "second compile" compile_probe || summarize
if [ "$(stat '.stats.cache_hits.counts.Rust // 0')" -gt 0 ]; then
    pass "second compile was a cache hit from R2"
else
    fail "second compile was not a cache hit"
    stats | jq '.stats | {cache_hits, cache_misses, cache_read_errors, cache_errors}' | redact | sed 's/^/        /'
fi

summarize
