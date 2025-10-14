# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This repository provides a Rust-based tool and scripts for testing Redpanda Helm chart upgrades in Kubernetes environments. The workflow simulates upgrading from older chart versions (e.g., v5.0.10) to the latest chart version, while maintaining backward compatibility for configuration files.

## Core Architecture

### Configuration Transformation Tool (`src/main.rs`)

The Rust application transforms legacy Helm values files to be compatible with the latest chart schema. Key transformation logic:

1. **Tiered Storage Migration** (`storage.tieredConfig.*` → `storage.tiered.config.*`)
2. **License Key Migration** (`license_key` → `enterprise.license` and `license_secret_ref` → `enterprise.licenseSecretRef`)
3. **Persistent Volume Path Migration** (`storage.tieredStorageHostPath` → `storage.tiered.hostPath`)

The tool fetches the latest chart schema from GitHub, merges it with existing configurations (preserving user values), and outputs a transformed `updated-values.yaml` file with incremental numbering if needed.

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

# Transform an existing values file
cargo run <path-to-existing-values.yaml>
# Example: cargo run values.yaml
# Output: updated-values.yaml (or updated-values-N.yaml if file exists)
```

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
