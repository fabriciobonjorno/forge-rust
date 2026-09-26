#!/usr/bin/env bash
# End-to-end check of `forge new`: the generated application must build, pass
# its own quality gates, serve HTTP, answer its health probe and shut down
# gracefully, both as a host binary and as the generated container image.
#
# The framework is not published yet, so the application depends on a copy of
# this checkout placed inside the application directory (and therefore inside
# the Docker build context). Everything else is exactly what users get.
#
#   bash scripts/e2e-generated-app.sh
#   FORGE_E2E_SKIP_DOCKER=1 bash scripts/e2e-generated-app.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/forge-e2e.XXXXXX")"
app_name="forge-e2e-app"
app_dir="$work_dir/$app_name"
image="$app_name:e2e"
container="$app_name-$$"
server_pid=""

log() { printf '\n==> %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -KILL "$server_pid" 2>/dev/null || true
  fi
  if command -v docker >/dev/null 2>&1; then
    docker rm -f "$container" >/dev/null 2>&1 || true
  fi
  rm -rf "$work_dir"
}
trap cleanup EXIT

free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

wait_for_http() {
  local url="$1"
  for _ in $(seq 1 100); do
    curl --silent --fail --max-time 1 "$url" >/dev/null 2>&1 && return 0
    sleep 0.1
  done
  fail "timed out waiting for $url"
}

log "Building the forge CLI"
cargo build --quiet --locked --manifest-path "$repo_root/Cargo.toml" -p forge-cli
forge="$repo_root/target/debug/forge"

log "Generating $app_name"
(cd "$work_dir" && "$forge" new "$app_name" --skip-lockfile --forge-path .forge/crates/forge)
mkdir -p "$app_dir/.forge"
tar -C "$repo_root" --exclude ./target --exclude ./.git --exclude './examples/*/target' -cf - . \
  | tar -C "$app_dir/.forge" -xf -

cd "$app_dir"
cargo generate-lockfile --quiet

log "Running the generated application's quality gates"
cargo fmt --all -- --check
cargo clippy --quiet --all-targets --locked -- -D warnings
cargo test --quiet --locked

log "Host run: serve, probe and graceful shutdown"
cargo build --quiet --locked
port="$(free_port)"
FORGE_BIND="127.0.0.1:$port" FORGE_SHUTDOWN_GRACE_SECS=5 "target/debug/$app_name" &
server_pid=$!
wait_for_http "http://127.0.0.1:$port/health/live"
curl --silent --fail "http://127.0.0.1:$port/health/ready" | grep -q '"ready"' || fail "readiness body"
curl --silent --fail "http://127.0.0.1:$port/" | grep -q "\"application\":\"$app_name\"" || fail "index route"
[[ "$(curl --silent --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$port/missing")" == 404 ]] \
  || fail "unknown route must return 404"
FORGE_BIND="127.0.0.1:$port" "target/debug/$app_name" healthcheck || fail "healthcheck subcommand"
if FORGE_BIDN="typo" "target/debug/$app_name" healthcheck 2>/dev/null; then
  fail "unknown FORGE_* keys must be rejected"
fi
kill -TERM "$server_pid"
status=0
wait "$server_pid" || status=$?
server_pid=""
[[ "$status" == 0 ]] || fail "SIGTERM should exit 0, got $status"
if FORGE_BIND="127.0.0.1:$port" "target/debug/$app_name" healthcheck 2>/dev/null; then
  fail "healthcheck must fail when the server is down"
fi
echo "host run OK"

if [[ "${FORGE_E2E_SKIP_DOCKER:-0}" == 1 ]]; then
  log "Skipping container checks (FORGE_E2E_SKIP_DOCKER=1)"
  exit 0
fi

log "Building the container image"
docker build --quiet --tag "$image" . >/dev/null

user="$(docker image inspect --format '{{.Config.User}}' "$image")"
[[ "$user" == "65532:65532" ]] || fail "image must run as 65532:65532, got '$user'"
if docker run --rm --entrypoint /bin/sh "$image" -c true >/dev/null 2>&1; then
  fail "the runtime image must not contain a shell"
fi
size="$(docker image inspect --format '{{.Size}}' "$image")"
echo "image user $user, size $((size / 1024 / 1024)) MiB, no shell"

log "Container run: health, HTTP and graceful stop"
port="$(free_port)"
docker run --detach --name "$container" --read-only --cap-drop ALL \
  --security-opt no-new-privileges --publish "127.0.0.1:$port:3000" "$image" >/dev/null
health=""
for _ in $(seq 1 60); do
  health="$(docker inspect --format '{{.State.Health.Status}}' "$container")"
  [[ "$health" == healthy ]] && break
  sleep 1
done
[[ "$health" == healthy ]] || { docker logs "$container" >&2; fail "container health is '$health'"; }
curl --silent --fail "http://127.0.0.1:$port/" | grep -q "\"application\":\"$app_name\"" || fail "container index route"
docker logs "$container" 2>&1 | head -n 1 | grep -q '^{' || fail "production logs must be JSON"
docker stop --timeout 20 "$container" >/dev/null
exit_code="$(docker inspect --format '{{.State.ExitCode}}' "$container")"
[[ "$exit_code" == 0 ]] || { docker logs "$container" >&2; fail "docker stop should exit 0, got $exit_code"; }
docker rm "$container" >/dev/null
docker image rm "$image" >/dev/null

log "All end-to-end checks passed"
