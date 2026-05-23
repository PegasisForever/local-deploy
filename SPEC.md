# Local Deploy — Specification

## Overview

Local Deploy is a system for serving static websites (SPAs) under unique UUID subpaths. It consists of:

1. **Server** — a Rust binary inside a Docker container (`pegasis0/local-deploy`) that runs nginx (public serving) and a management HTTP API (allocation + upload).
2. **MCP CLI** — a stdio MCP server with two tools that talk to the management API.

No authentication. Linux x64 only.

**Repository:** [PegasisForever/local-deploy](https://github.com/PegasisForever/local-deploy)

---

## Architecture

```
┌─────────────────┐     stdio MCP      ┌──────────────────┐
│  Cursor / IDE   │ ◄────────────────► │  local-deploy-   │
│                 │                    │  mcp (CLI)       │
└─────────────────┘                    └────────┬─────────┘
                                                │ HTTP JSON
                                                │ LOCAL_DEPLOY_MANAGEMENT_ADDRESS
                                                ▼
                                       ┌──────────────────┐
                                       │  Rust server     │
                                       │  (in container)  │
                                       ├──────────────────┤
                                       │  :11001 mgmt API │
                                       │  :11000 nginx    │
                                       └────────┬─────────┘
                                                │
                    LOCAL_DEPLOY_PUBLIC_ADDRESS │
                                                ▼
                              http(s)://host:11000/<uuid>/index.html
```

---

## Environment Variables

| Variable | Component | Required | Description |
|----------|-----------|----------|-------------|
| `LOCAL_DEPLOY_PUBLIC_ADDRESS` | Server (container) | Yes | Base URL where nginx serves content. May be `http` or `https`. Trailing slash is optional — server normalizes internally. |
| `LOCAL_DEPLOY_MANAGEMENT_ADDRESS` | MCP CLI | Yes | Base URL of management API, e.g. `http://localhost:11001`. Trailing slash is optional — CLI normalizes internally. |

### Address normalization

Both components strip trailing slashes before use. `LOCAL_DEPLOY_PUBLIC_ADDRESS` must include a scheme (`http://` or `https://`).

Examples (all equivalent after normalization):

- `https://deploy.example.com`
- `https://deploy.example.com/`
- `http://localhost:11000`

---

## Docker Container

- **Image:** `pegasis0/local-deploy:latest` (only `latest` tag; no semver tags)
- **Platform:** `linux/amd64` only
- **Entrypoint:** Rust server binary (`local-deploy-server`)
- **Exposed ports:**
  - **11000** — nginx (public static serving)
  - **11001** — management API
- **Volume:** mount host directory to `/data` for persistence across restarts

```bash
docker run -d \
  -p 11000:11000 \
  -p 11001:11001 \
  -v local-deploy-data:/data \
  -e LOCAL_DEPLOY_PUBLIC_ADDRESS=http://localhost:11000 \
  pegasis0/local-deploy:latest
```

---

## Server Binary

### Startup

1. Read `LOCAL_DEPLOY_PUBLIC_ADDRESS` (required; exit with error if missing).
2. Ensure `/data` exists (create if needed).
3. Write nginx config to `/etc/nginx/nginx.conf` (or included conf).
4. Start nginx as child process (listen on `11000`).
5. Bind management HTTP server on `0.0.0.0:11001`.

### Data layout

```
/data/
  <uuid>/
    index.html
    assets/...
```

Each allocated UUID gets a directory under `/data/<uuid>/`. Upload writes files into this directory. Allocations are **permanent** — no delete endpoint.

---

## Nginx Configuration

Single wildcard config serves all UUID subpaths. No per-allocation nginx reload needed.

### SPA routing behavior

| Request | Result |
|---------|--------|
| `GET /<uuid>/index.html` | Serves `/data/<uuid>/index.html` |
| `GET /<uuid>/assets/app.js` | Serves the file if it exists |
| `GET /<uuid>/dashboard/settings` | No such file → serves `/data/<uuid>/index.html` (client router handles it) |
| `GET /<uuid>` | `301` redirect to `/<uuid>/` (so relative asset paths resolve correctly) |

Rules:

- **Existing files win** — real static assets (JS, CSS, images, fonts, etc.) are served as-is when present on disk.
- **Unknown paths fall back to `index.html`** — any request under `/<uuid>/` that does not match a file or directory gets `index.html`, enabling client-side routers (React Router, Vue Router, etc.).
- Use `root` + `try_files`, not `alias` + `try_files` (alias breaks SPA fallback in regex locations).

### Example server block

```nginx
server {
    listen 11000;
    server_name _;

    # Redirect bare /<uuid> to /<uuid>/
    location ~ ^/(?<uuid>[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})$ {
        return 301 /$uuid/;
    }

    # SPA + static files under /<uuid>/...
    location ~ ^/(?<uuid>[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}(/.*)?$ {
        root /data;
        try_files $uri $uri/ /$uuid/index.html;

        # CORS — allow access from anywhere
        add_header Access-Control-Allow-Origin * always;
        add_header Access-Control-Allow-Methods "GET, HEAD, OPTIONS" always;
        add_header Access-Control-Allow-Headers * always;

        if ($request_method = OPTIONS) {
            return 204;
        }
    }

    # Short cache for SPA shell; hashed assets can be cached longer via separate rule if needed
    location ~* ^/[0-9a-f-]{36}/index\.html$ {
        root /data;
        add_header Cache-Control "no-cache";
    }
}
```

How `try_files` resolves (with `root /data`):

1. `GET /abc-uuid/assets/app.js` → try `/data/abc-uuid/assets/app.js` → found, serve file.
2. `GET /abc-uuid/dashboard` → not found as file or dir → internal redirect to `/abc-uuid/index.html` → serve `/data/abc-uuid/index.html`.
3. `GET /abc-uuid/` → try `/data/abc-uuid/` (directory) → typically serves `index.html` via dir index, or falls through to explicit fallback.

The server binary should set `index index.html` in the `http` or `server` block so directory requests also resolve correctly.

### Optimizations

- `gzip on` with `gzip_types` for text/css, application/javascript, application/json, image/svg+xml, etc.
- `gzip_static on` where pre-compressed assets exist.
- Sensible `expires` / `Cache-Control` for hashed static assets; short cache for `index.html`.
- `sendfile on`, `tcp_nopush on`, `tcp_nodelay on`.
- `open_file_cache` for frequently accessed files.

---

## Management API

All requests and responses use **JSON**. Content-Type: `application/json`.

No authentication. No max upload size limit (bounded only by disk and HTTP client/server defaults).

### Error response shape

```json
{
  "error": "human-readable message"
}
```

HTTP status codes: `400` bad request, `404` not found, `409` conflict, `500` internal error.

### `GET /health`

Liveness check.

**Response `200`:**

```json
{ "status": "ok" }
```

### `POST /allocate`

Create a new UUID v4 subpath. No prefix. Creates empty directory `/data/<uuid>/`.

**Request:** empty body (or `{}`).

**Response `201`:**

```json
{ "uuid": "a1b2c3d4-e5f6-7890-abcd-ef1234567890" }
```

### `PUT /upload/{uuid}`

Upload website files for an allocated UUID.

**Upload format:** `application/gzip` body containing a **tar.gz** archive. The MCP CLI tarballs the local folder (preserving relative paths) and sends the stream.

**Rules:**

- UUID must exist (was allocated via `POST /allocate`).
- Directory `/data/<uuid>/` must be **empty** (no prior upload). Returns `409 Conflict` if files already exist — **no overwrites**.
- Server extracts tar.gz into `/data/<uuid>/`, preserving paths. Rejects path traversal (`../` entries).

**Response `200`:**

```json
{
  "files": 12,
  "url": "http://localhost:11000/a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}
```

- `files` — count of extracted files.
- `url` — full public URL for the deployment, `{LOCAL_DEPLOY_PUBLIC_ADDRESS}/{uuid}` (normalized, no trailing slash). Built server-side from `LOCAL_DEPLOY_PUBLIC_ADDRESS`.

**Response `404`:** UUID not allocated.

**Response `409`:** UUID already has uploaded content.

---

## MCP CLI (stdio)

- **Binary name:** `local-deploy-mcp`
- **Language:** Rust (same Cargo workspace as server)
- **Transport:** stdio (Model Context Protocol)
- **Config:** `LOCAL_DEPLOY_MANAGEMENT_ADDRESS` (required; fail at startup if missing)
- **Target:** `x86_64-unknown-linux-gnu` only
- **Distribution:** GitHub Release asset on [PegasisForever/local-deploy](https://github.com/PegasisForever/local-deploy)

### Error handling

- On HTTP error: return MCP tool error including HTTP status and JSON error body from server.
- Validate inputs before calling server (e.g. `local_folder` must exist and be a directory).

### Tool 1: `allocate_subpath`

| Field | Value |
|-------|-------|
| **Arguments** | none |
| **Action** | `POST /allocate` |
| **Returns** | UUID string only (e.g. `a1b2c3d4-e5f6-7890-abcd-ef1234567890`) |

### Tool 2: `upload_website`

| Field | Value |
|-------|-------|
| **Arguments** | `uuid` (string), `local_folder` (string, absolute or relative path) |
| **Validation** | `local_folder` must exist and be a directory; fail with MCP error if not |
| **Action** | Recursively read all files under `local_folder`, create tar.gz, `PUT /upload/{uuid}` |
| **Example** | `local_folder=dist` → `dist/index.html` served at `<public>/<uuid>/index.html` |
| **Returns** | `url` field from upload response (public deployment URL) |

The MCP CLI does not need `LOCAL_DEPLOY_PUBLIC_ADDRESS` — it returns the `url` from `PUT /upload/{uuid}` directly.

---

## Project Structure (Rust workspace)

```
local-deploy/
├── Cargo.toml              # workspace
├── crates/
│   ├── server/             # local-deploy-server
│   └── mcp/                # local-deploy-mcp
├── docker/
│   └── Dockerfile
├── SPEC.md
└── README.md
```

---

## Build & Release (manual, no CI)

### Docker image

```bash
docker build --platform linux/amd64 -t pegasis0/local-deploy:latest .
docker push pegasis0/local-deploy:latest
```

### MCP CLI binary

```bash
cargo build --release --target x86_64-unknown-linux-gnu -p local-deploy-mcp
```

### GitHub Release

1. Tag release on [PegasisForever/local-deploy](https://github.com/PegasisForever/local-deploy).
2. Attach `local-deploy-mcp` binary (linux x64) to release assets.
3. Optionally attach SHA256 checksum file.

No GitHub Actions CI in v1 — build, test, push, and release are done manually.

---

## End-to-End Test Plan

1. Build Docker image: `pegasis0/local-deploy:latest`.
2. Run container with volume and env vars (ports 11000/11001).
3. Build MCP CLI; set `LOCAL_DEPLOY_MANAGEMENT_ADDRESS=http://localhost:11001`.
4. **`allocate_subpath`** → receive UUID.
5. Create sample SPA in `dist/` (`index.html`, `assets/`, test client-side route).
6. **`upload_website`** with UUID and `dist` → receive public URL.
7. Verify via HTTP:
   - `GET /<uuid>/index.html` → 200
   - `GET /<uuid>/some/client/route` → 200 with SPA fallback (`index.html`)
   - `GET /<uuid>/assets/...` → 200, correct content-type
   - CORS header `Access-Control-Allow-Origin: *` present
8. Restart container; verify files still served (volume persistence).
9. Attempt second upload to same UUID → `409 Conflict`.
10. Push Docker image; create GitHub Release with MCP binary.

---

## Out of Scope (v1)

- Authentication / API keys
- ARM64 / non-linux builds
- HTTPS termination inside container
- Custom domains per UUID
- Delete / deallocate endpoint
- Overwriting uploaded content
- GitHub Actions CI
- Docker semver tags (only `latest`)
- Build pipeline for user projects (upload pre-built folders only)

---

## Implementation Checklist

- [ ] Rust workspace with `server` and `mcp` crates
- [ ] Server: nginx config generation, process management, JSON management API
- [ ] Server: tar.gz extract with path-traversal protection
- [ ] MCP: stdio transport, two tools, error propagation
- [ ] Dockerfile (multi-stage, linux/amd64, nginx included)
- [ ] E2E test script
- [ ] README with usage examples
- [ ] Manual: build, push Docker image, GitHub release
