# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This repository provides a Rust-based tool and scripts for testing Redpanda Helm chart upgrades in Kubernetes environments. The workflow simulates upgrading from older chart versions (e.g., v5.0.10) to the latest chart version, while maintaining backward compatibility for configuration files.

## Technology Stack

**Language**: Rust (2021 edition)

**Key Dependencies**:
- `reqwest` (0.11) with JSON features - HTTP client for fetching latest chart values and schema
- `tokio` (1.0) with full features - Async runtime
- `serde_yaml` (0.9) - YAML parsing and serialization
- `serde_json` (1.0) - JSON handling for schema validation
- `clap` (4.4) with derive features - CLI argument parsing
- `regex` (1.10) - Version format validation
- `jsonschema` (0.18) - JSON Schema validation

**External Tools Required**: `kind`, `helm`, `kubectl`, `yq`, `jq`, `gcloud`

## Core Architecture

### Configuration Transformation Tool

The Rust application transforms legacy Helm values files to be compatible with the latest chart schema. The codebase is organized into modular components:

- `src/main.rs` - CLI interface and orchestration
- `src/transformation_engine.rs` - Core transformation logic
- `src/transformation_rule.rs` - Rule definitions and application
- `src/schema_registry.rs` - Schema version management
- `src/schema_version.rs` - Version detection
- `src/validation.rs` - Configuration validation
- `src/reporter.rs` - Transformation reporting

**Key transformation logic**:

1. **License Migrations**:
   - `license_key` → `enterprise.license`
   - `license_secret_ref` → `enterprise.licenseSecretRef` (with field renaming)

2. **Tiered Storage Migrations**:
   - `storage.tieredConfig.*` → `storage.tiered.config.*`
   - `storage.tieredStorageHostPath` → `storage.tiered.hostPath`
   - `storage.tieredStoragePersistentVolume` → `storage.tiered.persistentVolume`

3. **StatefulSet to PodTemplate Migration**:
   - Moves `nodeSelector`, `tolerations`, `affinity`, `securityContext`, `priorityClassName`, `topologySpreadConstraints`, `terminationGracePeriodSeconds` from `statefulset` to `podTemplate.spec`
   - Converts `podAntiAffinity` from v5 format (simple topologyKey) to standard Kubernetes format with labelSelector
   - Migrates `nodeAffinity` to `podTemplate.spec.affinity.nodeAffinity`
   - Migrates `statefulset.annotations` to `statefulset.podTemplate.annotations` (e.g., `karpenter.sh/do-not-disrupt`)
   - Automatically adds required labelSelector to topologySpreadConstraints if missing

4. **External Configuration Migration**:
   - `external.service.domain` → `external.domain` (moved up one level)
   - Enables proper domain configuration for external access

5. **Resource Format Conversion**:
   - Adds new format: `resources.requests` and `resources.limits` (matching values for production)
   - Preserves old format: `resources.cpu.cores` and `resources.memory.container.max` (required by schema)
   - Both formats maintained for backward compatibility

6. **Console v2 to v3 Migration**:
   - Moves `kafka.schemaRegistry` to top-level `schemaRegistry`
   - Wraps credentials in `authentication.basic` structure for both schemaRegistry and adminApi
   - Preserves SASL configuration with credentials
   - Removes deprecated Console fields (`enterprise`, `secret.login`, `secret.enterprise`)
   - Filters null values from `ingress.className` and `service.targetPort`

7. **Listeners Cleanup**:
   - Removes deprecated `kafkaEndpoint` field from HTTP and SchemaRegistry listeners

8. **InitContainers Filtering**:
   - Removes deprecated fields: `tuning`, `extraInitContainers`, `setTieredStorageCacheDirOwnership`
   - Removes `extraVolumeMounts` and `resources` from individual init containers

9. **SideCars Cleanup**:
   - Removes `extraVolumeMounts`, `resources`, and `securityContext` from configWatcher

10. **Schema Validation**:
   - Fetches latest `values.schema.json` from GitHub
   - Validates output against schema before writing file
   - Fails early with clear error messages if output doesn't conform to schema
   - Prevents invalid configurations from being deployed

The tool fetches the latest chart schema from GitHub, merges it with existing configurations (preserving user values), validates the output against the schema, and outputs a transformed `updated-values.yaml` file with incremental numbering if needed.

### Environment Configuration

The `.env` file controls:
- `CHART_VERSION`: Initial Redpanda chart version to deploy (e.g., `5.0.10`)
- `VALUES_FILE`: Path to existing Helm values file (e.g., `values.yaml`)
- `BUCKET_NAME`: GCS bucket name for tiered storage testing
- `BUCKET_REGION`: GCS bucket region
- `REDPANDA_EXTERNAL_DOMAIN`: External domain for certificate generation

**Important**: Source this file before running scripts: `export $(cat .env | xargs)`

### Test Environment Setup

The repository assumes a **kind** Kubernetes cluster with:
- 1 control plane node
- 5 worker nodes (for 5 Redpanda brokers)
- Custom node labels and taints for dedicated Redpanda pods
- cert-manager for TLS certificate management
- Custom StorageClass (`standard-redpanda`)

## Common Development Commands

### Build and Run the Transformation Tool

```bash
# Build the Rust application
cargo build --release

# Transform an existing values file (preserves existing versions)
cargo run <path-to-existing-values.yaml>
# Example: cargo run values.yaml
# Output: updated-values.yaml (or updated-values-N.yaml if file exists)

# Transform with explicit Redpanda version (when input is missing image.tag)
cargo run <path-to-existing-values.yaml> --redpanda-version <VERSION>
# Example: cargo run values-minimal.yaml --redpanda-version v23.2.24

# Transform with both Redpanda and Console versions
cargo run <path-to-existing-values.yaml> --redpanda-version <VERSION> --console-version <VERSION>
# Example: cargo run values.yaml --redpanda-version v23.2.24 --console-version v3.3.2
```

**Version Pinning Behavior**:
- If input has `image.tag` → preserved in output (CLI flags ignored)
- If input missing `image.tag` → REQUIRES `--redpanda-version` flag (errors if not provided)
- If input has `console.image.tag` → preserved in output
- If input missing `console.image.tag` and console enabled:
  - If `--console-version` provided → uses it
  - If no flag → auto-fetches latest from chart metadata
- All versions validated for proper semver format (vX.Y.Z)

### Test the Full Upgrade Path

1. **Initial Setup** (deploys old chart version):
```bash
export $(cat .env | xargs)
./setup.sh
```

2. **Transform Configuration**:
```bash
cargo run $VALUES_FILE
```

3. **Upgrade Chart** (keeps same Redpanda version):
```bash
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace -f updated-values.yaml
```

4. **Upgrade Redpanda** (iterative major version upgrades):
```bash
# Example: v23.2.24 → v23.3.20 → v24.1.16 → v24.2.4
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace -f updated-values.yaml --set image.tag=v23.3.20
```

5. **Cleanup**:
```bash
./teardown.sh
```

### Verification Commands

#### Cluster Health
```bash
# Verify broker versions
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt

# Test external Kafka listener with TLS
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk cluster info --brokers redpanda.redpanda:9094 --tls-enabled --tls-truststore /etc/tls/certs/external/ca.crt --user username --password password

# Test internal Kafka listener with TLS
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk cluster info --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore /etc/tls/certs/default/ca.crt --user username --password password
```

#### Tiered Storage Testing
```bash
# Create topic with tiered storage enabled
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore /etc/tls/certs/default/ca.crt --user username --password password topic create log1 -c redpanda.remote.read=true -c redpanda.remote.write=true

# Produce test data (run from inside pod)
kubectl -n redpanda exec -ti redpanda-0 -c redpanda -- bash
BATCH=$(date); printf "$BATCH %s\n" {1..100000} | rpk --brokers redpanda-0.redpanda.redpanda:9093 --tls-enabled --tls-truststore=/etc/tls/certs/default/ca.crt --user username --password password topic produce log1 -p 0

# Verify objects in GCS
gcloud storage ls --recursive "gs://$BUCKET_NAME/**"
```

## Setup Prerequisites

1. **Redpanda License**: Place valid license in `redpanda.license` file (required for tiered storage). Obtain from https://license.redpanda.com/
2. **GCP Credentials**: Refresh SSO credentials for `gcloud` CLI
3. **Required Tools**: `kind`, `helm`, `kubectl`, `yq`, `jq`, `gcloud`
4. **kind Cluster**: Create with 1 control plane + 5 worker nodes (see README for config)

## Important Implementation Details

### Schema Validation

The Redpanda Helm chart includes comprehensive JSON schema validation (`values.schema.json`) that validates configurations at multiple points:

**1. Transformation Tool Validation (NEW)**:
- The tool **automatically validates output** against the latest schema before writing the file
- Fetches `values.schema.json` from GitHub at runtime
- Fails immediately with clear error messages if output doesn't conform
- Prevents generating invalid configurations that would fail at deployment time

**2. Helm Validation**:
- Helm validates during `helm install` or `helm upgrade`
- Uses `additionalProperties: false` to reject unknown/deprecated fields
- Fails immediately if values don't match the schema (exit code 1)
- No silent ignoring: Invalid or deprecated fields cause complete failure

**Benefits of dual validation**:
- **Catch errors early**: Tool validation happens before committing transformed files
- **Clear context**: Tool errors reference transformation rules that may need updates
- **Safety net**: Helm provides final validation at deployment time

**Manual validation (if needed)**:
```bash
# Fast validation test
helm template redpanda redpanda/redpanda -f values.yaml

# Dry-run with cluster context
helm upgrade --install redpanda redpanda/redpanda -n redpanda -f values.yaml --dry-run
```

**Schema validation output example**:
```
=== Schema Validation ===
  ℹ Fetching latest chart schema...
  ✓ Schema validation passed
```

If validation fails, the tool will exit with clear error messages listing all schema violations before writing any output file.

### TLS Configuration
The setup creates two certificate sets:
- **default**: For internal communication (admin API, internal Kafka/HTTP/Schema Registry)
- **external**: For external listeners (generated via `generate-certs.sh` from external domain)

### SASL Authentication
Default credentials created by `setup.sh`:
- Username: `username`
- Password: `password`
- Mechanism: `SCRAM-SHA-256`

**IMPORTANT**: Always use `auth.sasl.secretRef` in values.yaml, never inline `auth.sasl.users`. The secretRef approach is required for proper operation with the config-watcher sidecar container.

### Node Affinity
Worker nodes are labeled with `nodetype=redpanda-pool` and tainted with `redpanda-pool=true:NoSchedule` to ensure dedicated scheduling.

### Tiered Storage
Uses GCS with HMAC authentication. The `setup.sh` script:
1. Creates GCS bucket
2. Generates HMAC keys from default service account
3. Injects credentials into values file via `yq`

### Upgrade Strategy
Always upgrade **chart first**, then **Redpanda version** in iterative major version steps (never skip major versions).

## Warning: Destructive Operations

The `teardown.sh` script is highly destructive. It will:
- Delete the entire `redpanda` namespace
- Delete the entire GCS bucket (`gs://$BUCKET_NAME`)
- Remove all node labels and taints
- Deactivate and delete HMAC keys

Review the script before running.
