#!/usr/bin/env bash
# End-to-end check of forge new. With Docker available this exercises the
# Phase 2 PostgreSQL path: reversible migrations, host serving, Compose startup,
# health and graceful shutdown. FORGE_E2E_SKIP_DOCKER=1 keeps a database-free
# host-only smoke path for environments without Docker.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/forge-e2e.XXXXXX")"
app_name="forge-e2e-app"
app_dir="$work_dir/$app_name"
server_pid=""

log() { printf '\n==> %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill -KILL "$server_pid" 2>/dev/null || true
  fi
  if [[ -f "$app_dir/compose.yaml" ]] && command -v docker >/dev/null 2>&1; then
    (cd "$app_dir" && docker compose down --volumes --remove-orphans >/dev/null 2>&1) || true
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
new_args=(new "$app_name" --skip-lockfile --forge-path .forge/crates/forge)
if [[ "${FORGE_E2E_SKIP_DOCKER:-0}" == 1 ]]; then
  new_args+=(--skip-database)
fi
(cd "$work_dir" && "$forge" "${new_args[@]}")
mkdir -p "$app_dir/.forge"
tar -C "$repo_root" --exclude ./target --exclude ./.git --exclude './examples/*/target' -cf - . \
  | tar -C "$app_dir/.forge" -xf -

cd "$app_dir"
cargo generate-lockfile --quiet

log "Running the generated application's quality gates"
cargo fmt --all -- --check
cargo clippy --quiet --all-targets --locked -- -D warnings
cargo test --quiet --locked

database_url=""
if [[ "${FORGE_E2E_SKIP_DOCKER:-0}" != 1 ]]; then
  log "Starting PostgreSQL and exercising reversible migrations"
  "$forge" generate migration create_e2e_probe >/dev/null
  up_file="$(find migrations -maxdepth 1 -name '*_create_e2e_probe.up.sql' -print -quit)"
  down_file="$(find migrations -maxdepth 1 -name '*_create_e2e_probe.down.sql' -print -quit)"
  [[ -n "$up_file" && -n "$down_file" ]] || fail "migration pair was not generated"
  printf '%s\n' 'CREATE TABLE forge_e2e_probe (id bigint PRIMARY KEY);' >"$up_file"
  printf '%s\n' 'DROP TABLE forge_e2e_probe;' >"$down_file"

  # sqlx::migrate!() embeds migrations at compile time, so rebuild after creating it.
  cargo build --quiet --locked
  docker compose up --detach db >/dev/null
  database_url="postgres://app:app@127.0.0.1:5432/forge_e2e_app"

  FORGE_DATABASE_URL="$database_url" "target/debug/$app_name" migrate
  [[ "$(docker compose exec -T db psql -U app -d forge_e2e_app -Atc       "SELECT to_regclass('public.forge_e2e_probe')")" == "forge_e2e_probe" ]]     || fail "migrate did not create the probe table"

  FORGE_DATABASE_URL="$database_url" "target/debug/$app_name" rollback
  [[ -z "$(docker compose exec -T db psql -U app -d forge_e2e_app -Atc       "SELECT to_regclass('public.forge_e2e_probe')")" ]]     || fail "rollback did not remove the probe table"

  FORGE_DATABASE_URL="$database_url" "target/debug/$app_name" migrate
fi

log "Host run: serve, probe and graceful shutdown"
cargo build --quiet --locked
port="$(free_port)"
env_args=(FORGE_BIND="127.0.0.1:$port" FORGE_SHUTDOWN_GRACE_SECS=5)
if [[ -n "$database_url" ]]; then
  env_args+=(FORGE_DATABASE_URL="$database_url")
fi
env "${env_args[@]}" "target/debug/$app_name" &
server_pid=$!
wait_for_http "http://127.0.0.1:$port/health/live"
curl --silent --fail "http://127.0.0.1:$port/health/ready" | grep -q '"ready"' || fail "readiness body"
curl --silent --fail "http://127.0.0.1:$port/" | grep -q "\"application\":\"$app_name\"" || fail "index route"
[[ "$(curl --silent --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$port/missing")" == 404 ]]   || fail "unknown route must return 404"
env "${env_args[@]}" "target/debug/$app_name" healthcheck || fail "healthcheck subcommand"
if FORGE_BIDN="typo" "target/debug/$app_name" healthcheck 2>/dev/null; then
  fail "unknown FORGE_* keys must be rejected"
fi
kill -TERM "$server_pid"
status=0
wait "$server_pid" || status=$?
server_pid=""
[[ "$status" == 0 ]] || fail "SIGTERM should exit 0, got $status"
if env "${env_args[@]}" "target/debug/$app_name" healthcheck 2>/dev/null; then
  fail "healthcheck must fail when the server is down"
fi
echo "host run OK"

if [[ "${FORGE_E2E_SKIP_DOCKER:-0}" == 1 ]]; then
  log "Skipping database/container checks (FORGE_E2E_SKIP_DOCKER=1)"
  exit 0
fi

log "Building and starting the generated Compose stack"
docker compose up --detach --build >/dev/null
app_container="$(docker compose ps --quiet app)"
[[ -n "$app_container" ]] || fail "Compose app container was not created"

health=""
for _ in $(seq 1 60); do
  health="$(docker inspect --format '{{.State.Health.Status}}' "$app_container")"
  [[ "$health" == healthy ]] && break
  sleep 1
done
[[ "$health" == healthy ]] || { docker compose logs >&2; fail "container health is '$health'"; }

image="$app_name:dev"
user="$(docker image inspect --format '{{.Config.User}}' "$image")"
[[ "$user" == "65532:65532" ]] || fail "image must run as 65532:65532, got '$user'"
if docker run --rm --entrypoint /bin/sh "$image" -c true >/dev/null 2>&1; then
  fail "the runtime image must not contain a shell"
fi

curl --silent --fail "http://127.0.0.1:3000/" | grep -q "\"application\":\"$app_name\""   || fail "container index route"
docker compose logs app 2>&1 | grep -m1 -q '^app-.* | {' || fail "production logs must be JSON"

# The named volume must preserve the migrated schema across an application restart.
docker compose restart app >/dev/null
[[ "$(docker compose exec -T db psql -U app -d forge_e2e_app -Atc     "SELECT to_regclass('public.forge_e2e_probe')")" == "forge_e2e_probe" ]]   || fail "database schema did not persist across app restart"

docker compose stop --timeout 20 app >/dev/null
app_container="$(docker compose ps --quiet --all app)"
exit_code="$(docker inspect --format '{{.State.ExitCode}}' "$app_container")"
[[ "$exit_code" == 0 ]] || { docker compose logs app >&2; fail "docker stop should exit 0, got $exit_code"; }

log "All end-to-end checks passed"
