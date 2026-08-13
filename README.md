# deckwatch-plugin-postgresql

A [deckwatch](https://github.com/alexconrey/deckwatch) plugin that provisions a PostgreSQL
database sidecar with persistent volume for an application via the **Infrastructure tab** —
no annotations required.

## Usage

1. Open the Application detail page in deckwatch.
2. Click the **Infrastructure** tab.
3. Click **+ Add PostgreSQL Database**.
4. Fill in the required fields (database name, username) and click **Provision**.

Once provisioned, deckwatch injects on the next reconcile:

- **Sidecar**: `postgres:16-alpine` on port 5432 (100m CPU / 256Mi memory), named `postgres-sidecar`
  - Sidecar env vars: `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `PGDATA`
- **Env vars** into every deployment in the application:
  - `PG_HOST=localhost`
  - `POSTGRES_DB=<db_name>`
  - `POSTGRES_USER=<username>`
  - `POSTGRES_PASSWORD=<generated>`
  - `PGDATA=/var/lib/postgresql/data/pgdata`
- **PVC**: `{application_name}-postgres-data` created in the application's namespace
- **Annotation**: `deckwatch.io/postgres-db: <db_name>` on all deployments

To remove the sidecar, click **Deprovision** on the Infrastructure tab. The sidecar is removed
from all deployments on the next reconcile. **The PVC is NOT deleted automatically** — delete it
manually if the data is no longer needed.

## Configuration

### Resource fields (per-provisioned instance)

| Key | Type | Required | Default | Description |
|-----|------|----------|---------|-------------|
| `db_name` | String | Yes | `app` | PostgreSQL database name |
| `username` | String | Yes | `app` | PostgreSQL user |
| `storage_size` | String | No | `1Gi` | PVC storage request (e.g. `5Gi`, `10Gi`) |
| `POSTGRES_IMAGE` | String | No | `postgres:16-alpine` | PostgreSQL container image |

### Plugin config (global defaults)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `POSTGRES_IMAGE` | String | `postgres:16-alpine` | Default PostgreSQL image (overridable per-resource) |

Set global defaults in **Settings → Plugins → PostgreSQL → Configure**.

## Password generation

Passwords are generated deterministically from `{application_name}-{db_name}` so that the
value is stable across reconcile cycles without requiring an external secret manager. For
production workloads, consider integrating with Vault or AWS Secrets Manager.

## Building

```bash
# Install the wasm32 target (once)
rustup target add wasm32-unknown-unknown

# Build the WASM artifact
make build          # → dist/plugin.wasm

# Run host-target unit tests
make test
```

## Marketplace

This plugin is available on the [deckwatch marketplace](http://market.deckwatch.io).

To install via the deckwatch UI: **Settings → Plugins → Browse Marketplace → PostgreSQL**.

### Submitting to the marketplace

To add or update this plugin in the marketplace catalog, open a PR against
[alexconrey/deckwatch-marketplace](https://github.com/alexconrey/deckwatch-marketplace)
adding the following entry to `catalog.json`:

```json
{
  "name": "PostgreSQL",
  "slug": "deckwatch-plugin-postgresql",
  "description": "Provisions a PostgreSQL database sidecar with PVC persistence via the Application Infrastructure tab.",
  "author": "alexconrey",
  "homepage": "https://github.com/alexconrey/deckwatch-plugin-postgresql",
  "trust_level": "community",
  "tags": ["postgresql", "database", "sidecar"],
  "latest_version": "0.1.0",
  "source": {
    "type": "github",
    "repo": "alexconrey/deckwatch-plugin-postgresql",
    "ref": "v0.1.0",
    "path": "plugin.wasm",
    "use_release": true
  },
  "allowed_hosts_hint": []
}
```

Also add `plugins/deckwatch-plugin-postgresql.md` following the template in the marketplace repo.

## License

Apache-2.0
