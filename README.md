# Local Deploy

Serve static websites (including SPAs) under unique UUID subpaths. The system includes a Dockerized Rust server (nginx + management API) and an MCP CLI for Cursor and other MCP clients.

## Components

| Component | Description |
|-----------|-------------|
| `local-deploy-server` | Runs inside Docker: nginx on `:11000`, management API on `:11001` |
| `local-deploy-mcp` | Stdio MCP server with `allocate_subpath` and `upload_website` tools |

## Quick start (Docker)

```bash
docker run -d \
  -p 11000:11000 \
  -p 11001:11001 \
  -v local-deploy-data:/data \
  -e LOCAL_DEPLOY_PUBLIC_ADDRESS=http://localhost:11000 \
  pegasis0/local-deploy:latest
```

Check health:

```bash
curl http://localhost:11001/health
# {"status":"ok"}
```

Allocate a UUID and upload a built site:

```bash
UUID=$(curl -s -X POST http://localhost:11001/allocate -H 'Content-Type: application/json' -d '{}' | jq -r .uuid)
tar -czf /tmp/site.tar.gz -C dist .
curl -X PUT "http://localhost:11001/upload/$UUID" \
  -H 'Content-Type: application/gzip' \
  --data-binary @/tmp/site.tar.gz
```

Open `http://localhost:11000/<uuid>/` in a browser.

## MCP CLI

Download `local-deploy-mcp` from [GitHub Releases](https://github.com/PegasisForever/local-deploy/releases) or build locally:

```bash
cargo build --release -p local-deploy-mcp
```

Configure in Cursor (`.cursor/mcp.json` or MCP settings):

```json
{
  "mcpServers": {
    "local-deploy": {
      "command": "/path/to/local-deploy-mcp",
      "env": {
        "LOCAL_DEPLOY_MANAGEMENT_ADDRESS": "http://localhost:11001"
      }
    }
  }
}
```

### Tools

**`allocate_subpath`** — Creates a new UUID v4 subpath. Returns the UUID string.

**`upload_website`** — Uploads a local folder as tar.gz to an allocated UUID.

| Argument | Description |
|----------|-------------|
| `uuid` | UUID from `allocate_subpath` |
| `local_folder` | Path to built static site (e.g. `dist`) |

Returns the public deployment URL (e.g. `http://localhost:11000/<uuid>`).

## Environment variables

| Variable | Component | Required | Example |
|----------|-----------|----------|---------|
| `LOCAL_DEPLOY_PUBLIC_ADDRESS` | Server | Yes | `http://localhost:11000` |
| `LOCAL_DEPLOY_MANAGEMENT_ADDRESS` | MCP CLI | Yes | `http://localhost:11001` |

Trailing slashes are stripped automatically.

## Management API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness check |
| `POST` | `/allocate` | Create empty UUID directory (201) |
| `PUT` | `/upload/{uuid}` | Upload tar.gz (`application/gzip`); 409 if already uploaded |

Upload response:

```json
{
  "files": 12,
  "url": "http://localhost:11000/a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}
```

## Build from source

```bash
# Server + MCP
cargo build --release

# Docker image (linux/amd64)
docker build --platform linux/amd64 -t pegasis0/local-deploy:latest .

# MCP release binary
cargo build --release --target x86_64-unknown-linux-gnu -p local-deploy-mcp
```

## End-to-end test

```bash
./scripts/e2e-test.sh
```

## SPA behavior

- `GET /<uuid>/assets/app.js` — serves the file when present
- `GET /<uuid>/dashboard/settings` — falls back to `index.html` for client-side routing
- `GET /<uuid>` — redirects to `/<uuid>/`
- CORS: `Access-Control-Allow-Origin: *`

## License

MIT
