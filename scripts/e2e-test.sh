#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${IMAGE:-pegasis0/local-deploy:latest}"
CONTAINER="${CONTAINER:-local-deploy-e2e}"
VOLUME="${VOLUME:-local-deploy-e2e-data}"
DIST_DIR="$ROOT/test-data/dist"

pass=0
fail=0

log() {
  echo "[e2e] $*"
}

container_ip() {
  docker inspect -f '{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$CONTAINER"
}

assert_eq() {
  local name="$1"
  local expected="$2"
  local actual="$3"
  if [[ "$expected" == "$actual" ]]; then
    log "PASS: $name"
    pass=$((pass + 1))
  else
    log "FAIL: $name (expected '$expected', got '$actual')"
    fail=$((fail + 1))
  fi
}

assert_http() {
  local name="$1"
  local expected_status="$2"
  local url="$3"
  shift 3
  local status
  status="$(docker exec "$CONTAINER" curl -s -o /tmp/e2e-body.txt -w '%{http_code}' "$@" "$url")"
  if [[ "$status" == "$expected_status" ]]; then
    log "PASS: $name (HTTP $status)"
    pass=$((pass + 1))
  else
    log "FAIL: $name (expected HTTP $expected_status, got $status)"
    docker exec "$CONTAINER" cat /tmp/e2e-body.txt >&2 || true
    fail=$((fail + 1))
  fi
}

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}

trap cleanup EXIT

log "Building Docker image: $IMAGE"
docker build --platform linux/amd64 -t "$IMAGE" "$ROOT"

log "Starting container"
cleanup
docker volume rm "$VOLUME" >/dev/null 2>&1 || true
docker run -d \
  --name "$CONTAINER" \
  -v "$VOLUME:/data" \
  -e LOCAL_DEPLOY_PUBLIC_ADDRESS=http://127.0.0.1:11000 \
  "$IMAGE" >/dev/null

log "Waiting for services"
for _ in $(seq 1 30); do
  if docker exec "$CONTAINER" curl -sf http://127.0.0.1:11001/health >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

health=$(docker exec "$CONTAINER" curl -sf http://127.0.0.1:11001/health)
assert_eq "health response" "{\"status\":\"ok\"}" "$health"

CONTAINER_IP="$(container_ip)"
MGMT_URL="http://${CONTAINER_IP}:11001"
PUBLIC_URL="http://127.0.0.1:11000"

log "Building MCP CLI"
cargo build --release -p local-deploy-mcp --manifest-path "$ROOT/Cargo.toml"

log "Creating sample SPA dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/assets"
cat > "$DIST_DIR/index.html" <<'HTML'
<!doctype html>
<html>
  <head><title>Local Deploy E2E</title></head>
  <body>
    <h1 id="title">Local Deploy E2E</h1>
    <script src="assets/app.js"></script>
  </body>
</html>
HTML
printf '%s\n' "document.getElementById('title').textContent = 'SPA Loaded';" > "$DIST_DIR/assets/app.js"

log "Running MCP allocate_subpath + upload_website"
export LOCAL_DEPLOY_MANAGEMENT_ADDRESS="$MGMT_URL"
MCP_OUTPUT="$(
  "$ROOT/target/release/local-deploy-mcp-e2e" "$DIST_DIR"
)"
UUID="$(echo "$MCP_OUTPUT" | sed -n '1p')"
URL="$(echo "$MCP_OUTPUT" | sed -n '2p')"
log "Allocated UUID: $UUID"
log "Upload URL: $URL"

assert_http "index.html" 200 "$PUBLIC_URL/$UUID/index.html"
assert_http "SPA fallback route" 200 "$PUBLIC_URL/$UUID/some/client/route"
assert_http "static asset" 200 "$PUBLIC_URL/$UUID/assets/app.js"

cors="$(docker exec "$CONTAINER" curl -sI "$PUBLIC_URL/$UUID/index.html" | tr -d '\r' | grep -i '^access-control-allow-origin:' | awk '{print $2}')"
assert_eq "CORS header" "*" "$cors"

content_type="$(docker exec "$CONTAINER" curl -sI "$PUBLIC_URL/$UUID/assets/app.js" | tr -d '\r' | grep -i '^content-type:' | awk '{print $2}')"
if [[ "$content_type" == *javascript* ]]; then
  log "PASS: asset content-type ($content_type)"
  pass=$((pass + 1))
else
  log "FAIL: asset content-type ($content_type)"
  fail=$((fail + 1))
fi

log "Restarting container to verify persistence"
docker restart "$CONTAINER" >/dev/null
sleep 3
for _ in $(seq 1 30); do
  if docker exec "$CONTAINER" curl -sf http://127.0.0.1:11001/health >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
assert_http "persistence after restart" 200 "$PUBLIC_URL/$UUID/index.html"

log "Second upload should conflict"
archive="$(mktemp /tmp/local-deploy-e2e-XXXXXX.tar.gz)"
tar -czf "$archive" -C "$DIST_DIR" .
docker cp "$archive" "$CONTAINER:/tmp/e2e-upload2.tar.gz"
status="$(docker exec "$CONTAINER" curl -s -o /tmp/e2e-upload2.txt -w '%{http_code}' \
  -X PUT \
  -H 'Content-Type: application/gzip' \
  --data-binary @/tmp/e2e-upload2.tar.gz \
  "http://127.0.0.1:11001/upload/$UUID")"
rm -f "$archive"
assert_eq "second upload status" "409" "$status"

log "Results: $pass passed, $fail failed"
if [[ "$fail" -gt 0 ]]; then
  exit 1
fi

log "All E2E checks passed"
