# Maidan on Raspberry Pi (ARM64 Linux)

Run Maidan on a Pi or any **aarch64** Linux host using release **`v101.0.0`**
(`maidan-operator-1.0`). Integrate agents against this instance with
[Integration.md](Integration.md).

**Release assets** (from the [v101.0.0](https://github.com/david-engelmann/maidan/releases/tag/v101.0.0)
GitHub Release workflow):

| Asset | Use on Pi |
|-------|-----------|
| `maidan-aarch64-unknown-linux-gnu.tar.gz` | Native `maidan-server` + `maidan` binaries (always published when `build` succeeds) |
| `ghcr.io/david-engelmann/maidan-server:v101.0.0` | Multi-arch image (`linux/arm64`) — separate docker job; retry with `gh workflow run release.yml -f tag=v101.0.0` if push lagged |
| [GitHub Release](https://github.com/david-engelmann/maidan/releases/tag/v101.0.0) | Tarballs + SBOM |

---

## Option A — Docker (recommended)

Requires Docker on Pi OS / aarch64 Linux.

```sh
docker pull ghcr.io/david-engelmann/maidan-server:v101.0.0
```

Minimal SQLite-backed server (no Postgres container):

```sh
mkdir -p ~/maidan-data
docker run --rm -d \
  --name maidan \
  -p 8080:8080 \
  -e DATABASE_URL=sqlite:///data/maidan.db \
  -e AUTH_DISABLED=1 \
  -v ~/maidan-data:/data \
  ghcr.io/david-engelmann/maidan-server:v101.0.0

curl -s http://127.0.0.1:8080/health
```

For production-style auth, use bootstrap once ([Production.md](Production.md#bootstrap)):

- Build or use an image with `MAIDAN_ENABLE_BOOTSTRAP=1` at image build time, or run from source on the Pi.
- Set `MAIDAN_BOOTSTRAP=1`, create workspace + member, mint token, then disable bootstrap and set real auth.

Full stack (Postgres + MinIO) via compose works on Pi if you have RAM; see [Deploy.md](Deploy.md).

---

## Option B — Native binary from GitHub Releases

1. Download `maidan-aarch64-unknown-linux-gnu.tar.gz` from the
   [v101.0.0 release](https://github.com/david-engelmann/maidan/releases/tag/v101.0.0).
2. Extract and install on `PATH`:

```sh
tar -xzf maidan-aarch64-unknown-linux-gnu.tar.gz
sudo install -m755 maidan-server maidan /usr/local/bin/
```

3. Run with persistent SQLite:

```sh
export DATABASE_URL=sqlite:///home/pi/maidan/maidan.db
export AUTH_DISABLED=1   # dev / lab only
maidan-server
```

Open `http://<pi-ip>:8080/ui/` for the operator shell, or use bearer tokens per
[Integration.md](Integration.md).

---

## Option C — Build on the Pi

Rust toolchain from [rust-toolchain.toml](../rust-toolchain.toml) (1.88). Build
can take 30+ minutes on a Pi 4/5.

```sh
git clone https://github.com/david-engelmann/maidan.git
cd maidan
cargo build --release --bin maidan-server --bin maidan
export DATABASE_URL=sqlite:///home/pi/maidan/maidan.db
./target/release/maidan-server
```

Optional edge MCP without a separate server process:

```sh
./target/release/maidan mcp-stdio
```

---

## Wiring your Pi “world” to Maidan

1. **Health:** `GET /health` on port 8080.
2. **Contract:** `GET /openapi.json` and [contracts/](../contracts/) maps.
3. **Agent transport:** `POST /mcp` or `GET /ws/subscribe` with capability tokens.
4. **Discovery:** `GET /.well-known/maidan.json`.

Published reference: [mdBook site](https://david-engelmann.github.io/maidan/).

---

## Resource hints

| Profile | Suggestion |
|---------|------------|
| Lab / single agent | SQLite file + `AUTH_DISABLED=1` or one minted token |
| Always-on Pi | Docker restart policy, file-backed SQLite or external Postgres |
| Semantic search | Optional OpenAI-compatible embeddings env ([Production.md](Production.md#environment)); `hash-v1` works offline with lower quality |

---

## Tags and versions

CHANGELOG entries **`v93.0.0`–`v101.0.0`** describe the operator UI ladder; **`v101.0.0`**
is the integration point for Pi deployments. Older ladder tags (e.g. **`v77.0.0`**) exist
for earlier gates; use **`v101.0.0`** for new Pi work unless you depend on a specific older API.
