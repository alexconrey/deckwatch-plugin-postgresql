//! Deckwatch plugin — provisions a PostgreSQL sidecar with PVC persistence via the
//! Infrastructure tab.
//!
//! ## WASM entry points
//!
//! - [`metadata`] — declares the "database" provisionable resource; called once at load time.
//! - [`provision`] — creates a PVC, generates deterministic credentials, and injects a
//!   PostgreSQL sidecar when an operator submits the provisioning form.
//! - [`deprovision`] — called when the resource is removed; returns a message noting that
//!   the PVC must be manually deleted if data should be removed.
//!
//! ## Provision behaviour
//!
//! When an operator clicks "Add PostgreSQL Database" on the Application Infrastructure tab,
//! `provision()`:
//! - Generates a deterministic placeholder password from `{application_name}-{db_name}`.
//! - Returns `state` containing `PG_HOST`, `POSTGRES_DB`, `POSTGRES_USER`,
//!   `POSTGRES_PASSWORD`, and `PGDATA` — persisted at application level and injected as
//!   env vars into all deployments on the next reconcile cycle.
//! - Returns a `postgres-sidecar` container with the relevant env vars set directly.
//! - Returns a `PersistentVolumeClaim` in `kubernetes_resources` for data persistence.
//!
//! ## Architecture
//!
//! Association is managed at the application level via the Infrastructure tab —
//! no deployment annotation required.

#[cfg(target_arch = "wasm32")]
use deckwatch_plugin_sdk::PluginMetadata;
use deckwatch_plugin_sdk::{
    ConfigField, ConfigFieldType, EnvVarSpec, PluginResource, ResourceDeprovisionRequest,
    ResourceDeprovisionResult, ResourceProvisionRequest, ResourceProvisionResult, SidecarSpec,
};
#[cfg(target_arch = "wasm32")]
use extism_pdk::*;

use serde_json::json;

const DEFAULT_IMAGE: &str = "postgres:16-alpine";
const PG_PORT: i32 = 5432;
const DEFAULT_PGDATA: &str = "/var/lib/postgresql/data/pgdata";
const DEFAULT_STORAGE_SIZE: &str = "1Gi";

// ── Resource declaration ───────────────────────────────────────────────────────

/// Declares the "database" provisionable resource shown on the Infrastructure tab.
///
/// Not `#[cfg(target_arch = "wasm32")]`-gated so it can be called in host-target
/// unit tests.
pub fn resource_decl() -> PluginResource {
    PluginResource {
        id: "database".into(),
        label: "PostgreSQL Database".into(),
        icon: "mdi-database".into(),
        description:
            "Provision a PostgreSQL database with a persistent volume for this application.".into(),
        singleton: true,
        fields: vec![
            ConfigField {
                key: "db_name".into(),
                label: "Database Name".into(),
                description: "Name of the PostgreSQL database to create.".into(),
                field_type: ConfigFieldType::String,
                default: Some("app".into()),
                required: true,
                options: vec![],
                env_source: None,
            },
            ConfigField {
                key: "username".into(),
                label: "Username".into(),
                description: "PostgreSQL user that owns the database.".into(),
                field_type: ConfigFieldType::String,
                default: Some("app".into()),
                required: true,
                options: vec![],
                env_source: None,
            },
            ConfigField {
                key: "storage_size".into(),
                label: "Storage Size".into(),
                description: "PVC storage request for the PostgreSQL data volume (e.g. 1Gi, 10Gi)."
                    .into(),
                field_type: ConfigFieldType::String,
                default: Some(DEFAULT_STORAGE_SIZE.into()),
                required: false,
                options: vec![],
                env_source: None,
            },
            ConfigField {
                key: "POSTGRES_IMAGE".into(),
                label: "PostgreSQL Image".into(),
                description: "PostgreSQL container image to inject as a sidecar.".into(),
                field_type: ConfigFieldType::String,
                default: Some(DEFAULT_IMAGE.into()),
                required: false,
                options: vec![],
                env_source: None,
            },
        ],
        output_keys: vec![
            "PG_HOST".into(),
            "POSTGRES_DB".into(),
            "POSTGRES_USER".into(),
            "POSTGRES_PASSWORD".into(),
            "PGDATA".into(),
        ],
    }
}

// ── PVC manifest helper ───────────────────────────────────────────────────────

/// Build a Kubernetes PVC manifest for PostgreSQL data persistence.
fn postgres_pvc(pvc_name: &str, namespace: &str, storage_size: &str) -> serde_json::Value {
    json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": pvc_name,
            "namespace": namespace,
            "labels": {
                "managed-by": "deckwatch",
                "plugin": "deckwatch-plugin-postgresql",
            },
        },
        "spec": {
            "accessModes": ["ReadWriteOnce"],
            "resources": {
                "requests": {
                    "storage": storage_size,
                },
            },
        },
    })
}

// ── Pure core: provision ──────────────────────────────────────────────────────

/// Provision a PostgreSQL database sidecar for an application.
///
/// Returns a [`ResourceProvisionResult`] with:
/// - `state`: `PG_HOST`, `POSTGRES_DB`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `PGDATA`
/// - `sidecars`: the `postgres-sidecar` container with credentials env vars
/// - `kubernetes_resources`: a `PersistentVolumeClaim` for the data volume
/// - `deployment_annotations`: stamps `deckwatch.io/postgres-db: {db_name}`
///
/// The password is generated deterministically from `{application_name}-{db_name}` so
/// it is stable across reconcile cycles without requiring external secret storage.
///
/// Testable on the native target without extism.
pub fn provision_inner(req: &ResourceProvisionRequest) -> ResourceProvisionResult {
    let mut result = ResourceProvisionResult::default();

    let db_name = req
        .fields
        .get("db_name")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "app".to_string());

    let username = req
        .fields
        .get("username")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "app".to_string());

    let storage_size = req
        .fields
        .get("storage_size")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_STORAGE_SIZE.to_string());

    let image = req
        .fields
        .get("POSTGRES_IMAGE")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| DEFAULT_IMAGE.to_string());

    // Deterministic placeholder password — stable across reconciles without
    // needing a secret manager. In production, replace this with a Vault or
    // AWS Secrets Manager integration.
    let password = format!("pgpass-{}-{}", req.application_name, db_name);

    let pvc_name = format!("{}-postgres-data", req.application_name);

    // Persist connection state at application level.
    result.state.insert("PG_HOST".into(), "localhost".into());
    result.state.insert("POSTGRES_DB".into(), db_name.clone());
    result
        .state
        .insert("POSTGRES_USER".into(), username.clone());
    result
        .state
        .insert("POSTGRES_PASSWORD".into(), password.clone());
    result.state.insert("PGDATA".into(), DEFAULT_PGDATA.into());

    // Stamp the db name annotation for visibility in kubectl.
    result
        .deployment_annotations
        .insert("deckwatch.io/postgres-db".into(), db_name.clone());

    // Inject the postgres sidecar with credentials passed as env vars so
    // the container can initialise the database on first start.
    result.sidecars.push(SidecarSpec {
        name: "postgres-sidecar".into(),
        image,
        env: vec![
            EnvVarSpec::value("POSTGRES_DB", &db_name),
            EnvVarSpec::value("POSTGRES_USER", &username),
            EnvVarSpec::value("POSTGRES_PASSWORD", &password),
            EnvVarSpec::value("PGDATA", DEFAULT_PGDATA),
        ],
        port: Some(PG_PORT),
        cpu: Some("100m".into()),
        memory: Some("256Mi".into()),
    });

    // Create the PVC for data persistence.
    result
        .kubernetes_resources
        .push(postgres_pvc(&pvc_name, &req.namespace, &storage_size));

    result
}

// ── Pure core: deprovision ────────────────────────────────────────────────────

/// Deprovision — the deckwatch resource record is removed and the sidecar disappears
/// on the next reconcile, but the PVC is NOT deleted automatically to prevent data loss.
/// The operator must manually delete the PVC if the data is no longer needed.
pub fn deprovision_inner(_req: &ResourceDeprovisionRequest) -> ResourceDeprovisionResult {
    ResourceDeprovisionResult {
        errors: vec![],
        message: "PostgreSQL sidecar will be removed from all application deployments on the next reconcile. The PersistentVolumeClaim is NOT deleted automatically — delete it manually if the data is no longer needed.".into(),
    }
}

// ── WASM entry points ─────────────────────────────────────────────────────────

/// Return static plugin metadata including the "database" resource declaration.
/// Called once by deckwatch at load time.
#[cfg(target_arch = "wasm32")]
#[plugin_fn]
pub fn metadata() -> FnResult<Json<PluginMetadata>> {
    Ok(Json(PluginMetadata {
        name: "postgresql".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: "Provisions a PostgreSQL database sidecar with PVC persistence via the Application Infrastructure tab".into(),
        provides: vec!["sidecar:postgres".into(), "database:postgres".into()],
        depends_on: vec![],
        optional_depends_on: vec![],
        config_schema: vec![ConfigField {
            key: "POSTGRES_IMAGE".into(),
            label: "PostgreSQL Image".into(),
            description: "Default PostgreSQL container image (can be overridden per-resource).".into(),
            field_type: ConfigFieldType::String,
            default: Some(DEFAULT_IMAGE.into()),
            required: false,
            options: vec![],
            env_source: None,
        }],
        resources: vec![resource_decl()],
    }))
}

/// Provision a PostgreSQL database sidecar. Called when an operator clicks
/// "Add PostgreSQL Database" and submits the provisioning form on the
/// Application Infrastructure tab.
#[cfg(target_arch = "wasm32")]
#[plugin_fn]
pub fn provision(
    Json(req): Json<ResourceProvisionRequest>,
) -> FnResult<Json<ResourceProvisionResult>> {
    Ok(Json(provision_inner(&req)))
}

/// Deprovision the PostgreSQL database resource. The sidecar is removed on the next
/// reconcile; the PVC must be deleted manually to avoid data loss.
#[cfg(target_arch = "wasm32")]
#[plugin_fn]
pub fn deprovision(
    Json(req): Json<ResourceDeprovisionRequest>,
) -> FnResult<Json<ResourceDeprovisionResult>> {
    Ok(Json(deprovision_inner(&req)))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn provision_req(fields: &[(&str, &str)]) -> ResourceProvisionRequest {
        ResourceProvisionRequest {
            application_name: "my-app".into(),
            namespace: "production".into(),
            resource_id: "database".into(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn deprovision_req() -> ResourceDeprovisionRequest {
        ResourceDeprovisionRequest {
            application_name: "my-app".into(),
            namespace: "production".into(),
            resource_id: "database".into(),
            state: HashMap::new(),
            fields: HashMap::new(),
        }
    }

    #[test]
    fn resource_decl_has_expected_id_and_output_keys() {
        let r = resource_decl();
        assert_eq!(r.id, "database");
        assert_eq!(r.label, "PostgreSQL Database");
        assert!(r.singleton, "database must be a singleton resource");
        assert!(r.output_keys.contains(&"PG_HOST".to_string()));
        assert!(r.output_keys.contains(&"POSTGRES_DB".to_string()));
        assert!(r.output_keys.contains(&"POSTGRES_USER".to_string()));
        assert!(r.output_keys.contains(&"POSTGRES_PASSWORD".to_string()));
        assert!(r.output_keys.contains(&"PGDATA".to_string()));
        assert!(r.fields.iter().any(|f| f.key == "db_name" && f.required));
        assert!(r.fields.iter().any(|f| f.key == "username" && f.required));
        assert!(r.fields.iter().any(|f| f.key == "storage_size"));
        assert!(r.fields.iter().any(|f| f.key == "POSTGRES_IMAGE"));
    }

    #[test]
    fn provision_returns_all_required_state_keys() {
        let result = provision_inner(&provision_req(&[
            ("db_name", "mydb"),
            ("username", "dbuser"),
        ]));
        assert!(
            result.errors.is_empty(),
            "no errors expected: {:?}",
            result.errors
        );
        assert_eq!(
            result.state.get("PG_HOST").map(String::as_str),
            Some("localhost")
        );
        assert_eq!(
            result.state.get("POSTGRES_DB").map(String::as_str),
            Some("mydb")
        );
        assert_eq!(
            result.state.get("POSTGRES_USER").map(String::as_str),
            Some("dbuser")
        );
        assert!(
            result.state.contains_key("POSTGRES_PASSWORD"),
            "POSTGRES_PASSWORD must be set"
        );
        assert!(
            !result.state["POSTGRES_PASSWORD"].is_empty(),
            "password must not be empty"
        );
        assert_eq!(
            result.state.get("PGDATA").map(String::as_str),
            Some(DEFAULT_PGDATA)
        );
    }

    #[test]
    fn provision_password_is_deterministic() {
        let req = provision_req(&[("db_name", "mydb"), ("username", "u")]);
        let r1 = provision_inner(&req);
        let r2 = provision_inner(&req);
        assert_eq!(r1.state["POSTGRES_PASSWORD"], r2.state["POSTGRES_PASSWORD"]);
    }

    #[test]
    fn provision_injects_postgres_sidecar() {
        let result = provision_inner(&provision_req(&[
            ("db_name", "mydb"),
            ("username", "dbuser"),
        ]));
        assert_eq!(result.sidecars.len(), 1);
        let sc = &result.sidecars[0];
        assert_eq!(sc.name, "postgres-sidecar");
        assert_eq!(sc.image, DEFAULT_IMAGE);
        assert_eq!(sc.port, Some(PG_PORT));
        // Sidecar must have POSTGRES_DB, POSTGRES_USER, POSTGRES_PASSWORD, PGDATA
        assert!(sc
            .env
            .iter()
            .any(|e| e.name == "POSTGRES_DB" && e.value == "mydb"));
        assert!(sc
            .env
            .iter()
            .any(|e| e.name == "POSTGRES_USER" && e.value == "dbuser"));
        assert!(sc
            .env
            .iter()
            .any(|e| e.name == "POSTGRES_PASSWORD" && !e.value.is_empty()));
        assert!(sc.env.iter().any(|e| e.name == "PGDATA"));
    }

    #[test]
    fn provision_creates_pvc_with_correct_size() {
        let result = provision_inner(&provision_req(&[
            ("db_name", "mydb"),
            ("username", "u"),
            ("storage_size", "5Gi"),
        ]));
        assert!(
            !result.kubernetes_resources.is_empty(),
            "PVC must be emitted"
        );
        let pvc = &result.kubernetes_resources[0];
        assert_eq!(pvc["kind"].as_str(), Some("PersistentVolumeClaim"));
        assert_eq!(
            pvc["metadata"]["name"].as_str(),
            Some("my-app-postgres-data")
        );
        assert_eq!(
            pvc["spec"]["resources"]["requests"]["storage"].as_str(),
            Some("5Gi")
        );
    }

    #[test]
    fn provision_pvc_defaults_storage_size() {
        let result = provision_inner(&provision_req(&[("db_name", "mydb"), ("username", "u")]));
        let pvc = &result.kubernetes_resources[0];
        assert_eq!(
            pvc["spec"]["resources"]["requests"]["storage"].as_str(),
            Some(DEFAULT_STORAGE_SIZE)
        );
    }

    #[test]
    fn provision_custom_image_is_used() {
        let result = provision_inner(&provision_req(&[
            ("db_name", "mydb"),
            ("username", "u"),
            ("POSTGRES_IMAGE", "postgres:15-alpine"),
        ]));
        assert_eq!(result.sidecars[0].image, "postgres:15-alpine");
    }

    #[test]
    fn provision_stamps_annotation() {
        let result = provision_inner(&provision_req(&[("db_name", "mydb"), ("username", "u")]));
        assert_eq!(
            result
                .deployment_annotations
                .get("deckwatch.io/postgres-db")
                .map(String::as_str),
            Some("mydb")
        );
    }

    #[test]
    fn provision_sidecar_has_resource_requests() {
        let result = provision_inner(&provision_req(&[("db_name", "mydb"), ("username", "u")]));
        assert!(result.sidecars[0].cpu.is_some(), "cpu request must be set");
        assert!(
            result.sidecars[0].memory.is_some(),
            "memory request must be set"
        );
    }

    #[test]
    fn deprovision_returns_message_mentioning_pvc() {
        let result = deprovision_inner(&deprovision_req());
        assert!(result.errors.is_empty());
        assert!(
            result.message.to_lowercase().contains("pvc")
                || result
                    .message
                    .to_lowercase()
                    .contains("persistentvolumeclaim")
                || result.message.to_lowercase().contains("volume"),
            "deprovision message should mention PVC: {}",
            result.message
        );
    }
}
