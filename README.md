# Redpanda Helm Chart Upgrade Tool

This repository provides a Rust-based tool and scripts for testing Redpanda Helm chart upgrades in Kubernetes environments. The workflow simulates upgrading from older chart versions (e.g., v5.0.10) to the latest chart version, while maintaining backward compatibility for configuration files.

## Overview

The tool transforms legacy Helm values files to be compatible with the latest chart schema, handling:
- **Tiered Storage Migration**: `storage.tieredConfig.*` → `storage.tiered.config.*`
- **License Key Migration**: `license_key` → `enterprise.license` and `license_secret_ref` → `enterprise.licenseSecretRef`
- **Persistent Volume Path Migration**: `storage.tieredStorageHostPath` → `storage.tiered.hostPath`
- **StatefulSet to PodTemplate Migration**: `statefulset.*` → `podTemplate.spec.*`
- **Resource Format Conversion**: Old format (`cpu.cores`, `memory.container.max`) → New format (`requests/limits`)

## Prerequisites

- **Kubernetes Environment**: kind (recommended) or any Kubernetes cluster
- **Tools**: `helm`, `kubectl`, `kind`, `jq`, `yq`, `gcloud` CLI
- **Rust**: For building the conversion tool
- **Redpanda License**: Valid license from https://license.redpanda.com/ (required for tiered storage testing)
- **GCP Credentials**: Refresh SSO credentials for `gcloud` CLI (if using tiered storage)

## Quick Start

### 1. Build the Rust conversion tool

```bash
cargo build --release
```

### 2. Set up environment variables

Copy the example environment file and update it with your configuration. **Important**: The `.env` file contains sensitive credentials and is not committed to git (listed in `.gitignore`).

```bash
cp .env.example .env
vi .env
```

Configure the following variables:
- `CHART_VERSION`: Initial Redpanda Helm chart version (e.g., `5.0.10`)
- `VALUES_FILE`: Path to your values file (e.g., `values-initial.yaml`)
- `BUCKET_NAME`: GCS bucket name for tiered storage
- `BUCKET_REGION`: GCS bucket region (e.g., `us-central1`)
- `REDPANDA_EXTERNAL_DOMAIN`: External domain for TLS certificates
- `CLOUD_STORAGE_ACCESS_KEY`: GCS HMAC access key (generated in step 10 below, or use existing)
- `CLOUD_STORAGE_SECRET_KEY`: GCS HMAC secret key (generated in step 10 below, or use existing)

Then export the variables:

```bash
export $(cat .env | xargs)
```

### 3. Create kind cluster

Create a kind cluster with control plane and worker nodes:

```bash
cat <<EOF > kind-config.yaml
apiVersion: kind.x-k8s.io/v1alpha4
kind: Cluster
networking:
  podSubnet: "10.18.0.0/16"
nodes:
  - role: control-plane
  - role: worker
  - role: worker
  - role: worker
  - role: worker
  - role: worker
  - role: worker
  - role: worker
EOF

kind create cluster --name redpanda-upgrade --config kind-config.yaml
```

### 4. Deploy initial Redpanda cluster

Follow the detailed manual setup steps below to deploy Redpanda v23.2.24 with chart v5.0.10. The setup includes:
- MetalLB installation and configuration
- TLS certificate generation
- SASL secret creation
- GCS bucket and HMAC credentials (for tiered storage)
- StorageClass creation (for kind clusters)
- cert-manager installation
- Node taints and labels
- Redpanda deployment with old chart version
- Tiered storage timeout configuration

See the "Detailed Manual Setup" section below for step-by-step instructions.

### 5. Verify SASL secret is not Helm-managed

**IMPORTANT**: Before upgrading to the new chart, verify that the `redpanda-superusers` secret is NOT managed by Helm. If Helm manages this secret, it will be deleted during the upgrade because the new chart doesn't include it in its manifests.

```bash
# Check if the secret has Helm ownership labels/annotations
kubectl get secret redpanda-superusers -n redpanda -o yaml | grep -E "(annotations|labels)" -A 5
```

If you see Helm labels (like `app.kubernetes.io/managed-by: Helm`) or annotations (like `meta.helm.sh/release-name`), remove them:

```bash
# Remove Helm ownership from the SASL secret
kubectl annotate secret redpanda-superusers -n redpanda \
  meta.helm.sh/release-name- \
  meta.helm.sh/release-namespace-

kubectl label secret redpanda-superusers -n redpanda \
  app.kubernetes.io/managed-by- \
  app.kubernetes.io/component- \
  app.kubernetes.io/instance- \
  app.kubernetes.io/name- \
  helm.sh/chart-
```

**Note**: If you followed the deployment steps correctly (creating the secret manually with `users: []` in values), the secret should already be external to Helm and this step can be skipped.

### 6. Convert values for new chart

```bash
cargo run $VALUES_FILE
```

This creates `updated-values.yaml` with transformed configuration.

### 7. Upgrade Helm chart

Upgrade to the latest chart version (keeps same Redpanda version):

```bash
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml
```

### 8. Upgrade Redpanda version

Upgrade Redpanda iteratively through major versions:

```bash
# Get current version
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt

# List available versions from GitHub releases (recommended)
curl -s "https://api.github.com/repos/redpanda-data/redpanda/releases" | jq -r '.[].tag_name' | sort -V

# Check for latest patch version in a specific major.minor series
# Example for v24.1.x:
curl -s "https://api.github.com/repos/redpanda-data/redpanda/releases" | jq -r '.[].tag_name' | grep 'v24\.1\.' | sort -V | tail -1

# Upgrade path example: v23.2.24 → v23.3.20 → v24.1.21 → v24.2.27 → v24.3.18 → v25.1.12 → v25.2.9
# As of October 2025, latest patch versions tested are:
#   - v23.3.20 (latest in v23.3 series)
#   - v24.1.21 (latest in v24.1 series)
#   - v24.2.27 (latest in v24.2 series)
#   - v24.3.18 (latest in v24.3 series)
#   - v25.1.12 (latest in v25.1 series)
#   - v25.2.9 (latest in v25.2 series)

# Upgrade to v23.3.20
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml \
  --set image.tag=v23.3.20

# Upgrade to v24.1.21
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml \
  --set image.tag=v24.1.21

# Upgrade to v24.2.27
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml \
  --set image.tag=v24.2.27

# Upgrade to v24.3.18
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml \
  --set image.tag=v24.3.18

# Upgrade to v25.1.12
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml \
  --set image.tag=v25.1.12

# Upgrade to v25.2.9 (latest as of Oct 2025)
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 2h \
  --create-namespace \
  -f updated-values.yaml \
  --set image.tag=v25.2.9
```

**Important**: Never skip major versions. Always upgrade incrementally.

## Tested Upgrade Path

This upgrade workflow has been successfully tested with the following path, validating chart migration and incremental Redpanda version upgrades:

### Environment Configuration

**Initial Setup:**
- **Kubernetes**: kind cluster (1 control plane + 7 worker nodes)
- **Starting Helm Chart**: v5.0.10
- **Starting Redpanda Version**: v23.2.24
- **Target Redpanda Version**: v25.2.9
- **Object Storage**: Google Cloud Storage (GCS) with HMAC authentication
- **Features Tested**:
  - TLS (internal + external certificates)
  - SASL authentication (SCRAM-SHA-256)
  - Tiered storage to GCS
  - External LoadBalancer services
  - Node affinity and taints

### Complete Upgrade Path

| Step | Helm Chart | Redpanda Version | Notes |
|------|------------|------------------|-------|
| 1. Initial Deployment | v5.0.10 | v23.2.24 | Deployed with legacy chart schema |
| 2. Values Conversion | v5.0.10 | v23.2.24 | Rust tool converted values file |
| 3. Chart Upgrade | Latest | v23.2.24 | Upgraded to latest chart, keeping same Redpanda version |
| 4. Version Upgrade | Latest | v23.3.20 | First major version upgrade |
| 5. Version Upgrade | Latest | v24.1.21 | Second major version upgrade |
| 6. Version Upgrade | Latest | v24.2.27 | Third major version upgrade |
| 7. Version Upgrade | Latest | v24.3.18 | Fourth major version upgrade |
| 8. Version Upgrade | Latest | v25.1.12 | Fifth major version upgrade |
| 9. Version Upgrade | Latest | v25.2.9 | Final version (as of Oct 2025) |

### Validation Steps at Each Stage

After each upgrade, the following validations were performed:

1. **Cluster Health Check**
   ```bash
   kubectl exec -n redpanda redpanda-0 -c redpanda -- \
     rpk cluster health \
     --api-urls redpanda.redpanda:9644 \
     -X admin.tls.enabled=true \
     -X admin.tls.ca=/etc/tls/certs/default/ca.crt
   ```
   - Verified all nodes healthy
   - No leaderless partitions
   - No under-replicated partitions

2. **Version Verification**
   ```bash
   kubectl exec -n redpanda redpanda-0 -c redpanda -- rpk version
   ```
   - Confirmed all brokers running expected version
   - Verified cluster-wide version consistency

3. **Tiered Storage Validation**
   ```bash
   # Produce test data
   kubectl exec -n redpanda redpanda-0 -c redpanda -- \
     rpk topic produce test-tiered-storage \
     --brokers redpanda-0.redpanda.redpanda:9093 \
     --tls-enabled --tls-truststore=/etc/tls/certs/default/ca.crt \
     --user=username --password=password

   # Verify data in GCS
   gcloud storage ls -r gs://${BUCKET_NAME}/
   ```
   - Confirmed segment uploads to GCS
   - Verified manifest files created
   - Validated `cloud_storage_enabled: true`

4. **Topic Configuration Test**
   - Created test topic with small segment size (1MB)
   - Set `retention.local.target.bytes: 2MB` for fast tiered storage uploads
   - Produced 50,000 messages per validation cycle
   - Confirmed data accessibility after upgrades

### Critical Configuration Fixes

During testing, the following configuration issues were identified and resolved:

1. **Missing GCS Endpoint** (values-initial.yaml:110)
   ```yaml
   cloud_storage_api_endpoint: storage.googleapis.com
   ```
   - **Issue**: Uploads timing out without explicit endpoint
   - **Fix**: Added `cloud_storage_api_endpoint` to tiered storage config

2. **Credentials Source** (values-initial.yaml:106)
   ```yaml
   cloud_storage_credentials_source: config_file
   ```
   - **Issue**: Implicit credential source caused connection issues
   - **Fix**: Explicitly set to `config_file` for HMAC authentication

3. **Upload Timeout** (cluster config)
   ```bash
   rpk cluster config set cloud_storage_manifest_upload_timeout_ms 60000
   ```
   - **Issue**: Default 10-second timeout too short for GCS uploads
   - **Fix**: Increased to 60 seconds via `rpk cluster config set`

4. **HMAC Permissions** (GCS IAM)
   ```bash
   gcloud storage buckets add-iam-policy-binding gs://BUCKET_NAME \
     --member="serviceAccount:SERVICE_ACCOUNT" \
     --role="roles/storage.objectAdmin"
   ```
   - **Issue**: HMAC key lacked bucket write permissions
   - **Fix**: Granted `storage.objectAdmin` role to service account

### Key Takeaways

- **Chart First, Then Version**: Always upgrade the Helm chart before upgrading Redpanda versions
- **Incremental Upgrades**: Never skip major versions (e.g., v23.x → v24.x → v25.x)
- **Tiered Storage Resilience**: GCS tiered storage remained operational through all 6 version upgrades
- **Zero Downtime**: All upgrades completed with rolling updates, maintaining cluster availability
- **Configuration Persistence**: Custom configs (timeouts, endpoints) persisted across upgrades

## Detailed Manual Setup

Follow these steps to manually set up the Redpanda test environment:

### 1. Create kind cluster

```bash
kind create cluster --name redpanda-upgrade --config kind-config.yaml
```

### 2. Deploy MetalLB

```bash
kubectl apply -f https://raw.githubusercontent.com/metallb/metallb/v0.13.12/config/manifests/metallb-native.yaml
kubectl wait -n metallb-system --for=condition=ready pod --selector=app=metallb --timeout=90s
```

### 3. Configure MetalLB IP address pool

```bash
# Run the automated script to calculate IP range and create metallb-config.yaml
./get-docker-ip-range.sh

# Apply configuration
kubectl apply -f metallb-config.yaml
```

The script automatically:
1. Inspects the Docker 'kind' network
2. Extracts the subnet (e.g., `172.19.0.0/16`)
3. Calculates a suitable IP pool range (e.g., `172.19.255.200-172.19.255.250`)
4. Creates `metallb-config.yaml` with the calculated range

### 4. Generate TLS certificates

```bash
curl -sLO https://gist.githubusercontent.com/vuldin/e4b4a776df6dc0b4593302437ea57eed/raw/generate-certs.sh
chmod +x generate-certs.sh
./generate-certs.sh testdomain.local
```

### 5. Create Redpanda namespace

```bash
kubectl create ns redpanda
```

### 6. Create SASL secret

**IMPORTANT**: Use `users.txt` as the key name (not `superusers.txt`). Do NOT add Helm labels/annotations so the secret persists across Helm upgrades:

```bash
echo 'username:password:SCRAM-SHA-256' > users.txt
kubectl create secret generic redpanda-superusers -n redpanda --from-file=users.txt
```

### 7. Create license secret

**REQUIRED**: A valid Redpanda Enterprise license is required for tiered storage. Get a license from https://license.redpanda.com/ and save it to `redpanda.license`:

```bash
kubectl create secret generic redpanda-license -n redpanda --from-file=redpanda.license
```

### 8. Create TLS secret

Do NOT add Helm labels/annotations so the secret persists across Helm upgrades:

```bash
kubectl create secret generic tls-external -n redpanda \
  --from-file=ca.crt=certs/ca.crt \
  --from-file=tls.crt=certs/node.crt \
  --from-file=tls.key=certs/node.key
```

### 9. Apply node taints and labels

```bash
kubectl taint node redpanda-upgrade-worker redpanda-pool1=true:NoSchedule
kubectl label node redpanda-upgrade-worker nodetype=redpanda-pool1
kubectl taint node redpanda-upgrade-worker2 redpanda-pool1=true:NoSchedule
kubectl label node redpanda-upgrade-worker2 nodetype=redpanda-pool1
kubectl taint node redpanda-upgrade-worker3 redpanda-pool1=true:NoSchedule
kubectl label node redpanda-upgrade-worker3 nodetype=redpanda-pool1
```

### 10. Create GCS bucket and HMAC credentials (for tiered storage)

If using GCS for tiered storage, create the bucket and HMAC credentials:

```bash
# Ensure environment variables are loaded
export $(cat .env | xargs)

# Create GCS bucket
gcloud storage buckets create gs://$BUCKET_NAME \
  --uniform-bucket-level-access \
  --no-public-access-prevention \
  --location=$BUCKET_REGION

# Get default service account
GOOGLE_SERVICE_ACCOUNT=$(gcloud iam service-accounts list | tail -1 | awk '{print $6}')

# If you need to create new HMAC credentials (skip if using existing credentials from .env):
gcloud storage hmac create $GOOGLE_SERVICE_ACCOUNT --format=json > hmac.json
export CLOUD_STORAGE_ACCESS_KEY=$(cat hmac.json | jq -r .metadata.accessId)
export CLOUD_STORAGE_SECRET_KEY=$(cat hmac.json | jq -r .secret)

# Update .env file with the new credentials (if generated above)
# sed -i "s/CLOUD_STORAGE_ACCESS_KEY=.*/CLOUD_STORAGE_ACCESS_KEY=$CLOUD_STORAGE_ACCESS_KEY/" .env
# sed -i "s/CLOUD_STORAGE_SECRET_KEY=.*/CLOUD_STORAGE_SECRET_KEY=$CLOUD_STORAGE_SECRET_KEY/" .env

# Grant bucket permissions to service account
gcloud storage buckets add-iam-policy-binding gs://$BUCKET_NAME \
  --member="serviceAccount:$GOOGLE_SERVICE_ACCOUNT" \
  --role="roles/storage.objectAdmin"

# Update values file with credentials from environment variables
accessKey=$CLOUD_STORAGE_ACCESS_KEY yq -i '(.storage.tieredConfig.cloud_storage_access_key = strenv(accessKey))' $VALUES_FILE
secretKey=$CLOUD_STORAGE_SECRET_KEY yq -i '(.storage.tieredConfig.cloud_storage_secret_key = strenv(secretKey))' $VALUES_FILE
bucketName=$BUCKET_NAME yq -i '(.storage.tieredConfig.cloud_storage_bucket = strenv(bucketName))' $VALUES_FILE
bucketRegion=$BUCKET_REGION yq -i '(.storage.tieredConfig.cloud_storage_region = strenv(bucketRegion))' $VALUES_FILE
yq -i '.storage.tieredConfig.cloud_storage_api_endpoint = "storage.googleapis.com"' $VALUES_FILE
yq -i '.storage.tieredConfig.cloud_storage_credentials_source = "config_file"' $VALUES_FILE
```

**Important Configuration for GCS**:
- The HMAC credentials are stored in `.env` (which is git-ignored) and injected into the values file during setup
- If you have existing HMAC credentials, add them to `.env` and skip the `gcloud storage hmac create` step
- Ensure your values file includes the API endpoint and credentials source to avoid timeout issues (see "Critical Configuration Fixes" in the Tested Upgrade Path section)

### 11. Create StorageClass (if needed)

For kind clusters or environments without a default StorageClass:

```bash
cat <<EOF | kubectl apply -f -
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: standard-redpanda
provisioner: rancher.io/local-path
volumeBindingMode: WaitForFirstConsumer
EOF
```

Then reference this StorageClass in your values file:
```yaml
storage:
  persistentVolume:
    enabled: true
    size: 10Gi
    storageClass: standard-redpanda
```

### 12. Install cert-manager

```bash
helm install cert-manager cert-manager \
  --repo https://charts.jetstack.io \
  --set crds.enabled=true \
  --namespace cert-manager \
  --create-namespace \
  --wait
```

### 13. Deploy Redpanda v23.2.24 with chart 5.0.10

Create `values-initial.yaml` with your configuration. **IMPORTANT**: Include both `auth.sasl.users: []` to use the externally-created secret, and `license_secret_ref` to enable tiered storage:

```yaml
license_secret_ref:
  secret_name: redpanda-license
  secret_key: redpanda.license

auth:
  sasl:
    enabled: true
    mechanism: SCRAM-SHA-256
    secretRef: redpanda-superusers
    users: []
```

Then deploy:

```bash
helm upgrade --install redpanda redpanda \
  --repo https://charts.redpanda.com \
  -n redpanda \
  --wait \
  --timeout 10m \
  --create-namespace \
  --version 5.0.10 \
  -f values-initial.yaml
```

### 14. Verify SASL secret is not Helm-managed (before upgrade)

**IMPORTANT**: Before upgrading to the new chart, verify that the `redpanda-superusers` secret is NOT managed by Helm. If Helm manages this secret, it will be deleted during the upgrade because the new chart doesn't include it in its manifests.

```bash
# Check if the secret has Helm ownership labels/annotations
kubectl get secret redpanda-superusers -n redpanda -o yaml | grep -E "(annotations|labels)" -A 5
```

If you see Helm labels (like `app.kubernetes.io/managed-by: Helm`) or annotations (like `meta.helm.sh/release-name`), remove them:

```bash
# Remove Helm ownership from the SASL secret
kubectl annotate secret redpanda-superusers -n redpanda \
  meta.helm.sh/release-name- \
  meta.helm.sh/release-namespace-

kubectl label secret redpanda-superusers -n redpanda \
  app.kubernetes.io/managed-by- \
  app.kubernetes.io/component- \
  app.kubernetes.io/instance- \
  app.kubernetes.io/name- \
  helm.sh/chart-
```

**Note**: If you followed step 6 correctly (creating the secret manually with `users: []` in values), the secret should already be external to Helm and this step can be skipped.

### 15. Configure tiered storage timeout (important for GCS)

If using tiered storage with GCS, increase the upload timeout to prevent connection timeouts:

```bash
kubectl exec -n redpanda redpanda-0 -c redpanda -- \
  rpk cluster config set cloud_storage_manifest_upload_timeout_ms 60000 \
  --api-urls redpanda.redpanda:9644 \
  -X admin.tls.enabled=true \
  -X admin.tls.ca=/etc/tls/certs/default/ca.crt

# Restart pods to apply the configuration change
kubectl rollout restart statefulset redpanda -n redpanda
kubectl rollout status statefulset redpanda -n redpanda --timeout=10m
```

This increases the timeout from the default 10 seconds to 60 seconds, which is necessary for reliable uploads to cloud storage.

### 16. Configure local rpk profile (optional)

If you have `rpk` installed locally and want to interact with the cluster via external LoadBalancer endpoints:

```bash
# Extract the CA certificate
kubectl get secret -n redpanda tls-external -o go-template='{{ index .data "ca.crt" | base64decode }}' > ca.crt

# Get the external IP addresses
kubectl get svc -n redpanda -l app.kubernetes.io/component=redpanda-statefulset -o jsonpath='{range .items[*]}{.status.loadBalancer.ingress[0].ip}{"\n"}{end}'

# Create rpk profile manually (redpanda-rpk ConfigMap doesn't exist in chart v5.0.10)
# Use the first broker's external IP
BROKER_IP=$(kubectl get svc -n redpanda lb-redpanda-0 -o jsonpath='{.status.loadBalancer.ingress[0].ip}')

rpk profile create redpanda-helm
rpk profile set brokers=${BROKER_IP}:31092
rpk profile set tls.enabled=true
rpk profile set tls.ca_file=ca.crt
rpk profile set sasl.mechanism=SCRAM-SHA-256
rpk profile set sasl.user=username
rpk profile set sasl.password=password

# Test connectivity
rpk cluster info
```

**Note**: The `redpanda-rpk` ConfigMap is only available in newer chart versions. After upgrading to the latest chart, you can use:

```bash
rpk profile create --from-profile <(kubectl get configmap -n redpanda redpanda-rpk -o go-template='{{ .data.profile }}') redpanda-helm
rpk profile set kafka_api.sasl.user=username kafka_api.sasl.password=password kafka_api.sasl.mechanism=SCRAM-SHA-256
```

## Verification

### Cluster Health

```bash
# Check pods are running
kubectl get pods -n redpanda

# Verify Redpanda version
kubectl exec -n redpanda redpanda-0 -c redpanda -- rpk version

# Verify external Kafka listener connectivity
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- \
  rpk cluster info \
  --brokers redpanda.redpanda:9094 \
  --tls-enabled \
  --tls-truststore /etc/tls/certs/external/ca.crt \
  --user username \
  --password password

# Verify internal Kafka listener connectivity
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- \
  rpk cluster info \
  --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 \
  --tls-enabled \
  --tls-truststore /etc/tls/certs/default/ca.crt \
  --user username \
  --password password

# Verify admin listener
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- \
  rpk cluster health \
  --api-urls redpanda.redpanda:9644 \
  -X admin.tls.enabled=true \
  -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

### Tiered Storage Testing

```bash
# Create topic with tiered storage enabled
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- \
  rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 \
  --tls-enabled \
  --tls-truststore /etc/tls/certs/default/ca.crt \
  --user username \
  --password password \
  topic create log1 \
  -c redpanda.remote.read=true \
  -c redpanda.remote.write=true

# Produce test data
kubectl -n redpanda exec -ti redpanda-0 -c redpanda -- bash
BATCH=$(date); printf "$BATCH %s\n" {1..100000} | rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore=/etc/tls/certs/default/ca.crt --user username --password password topic produce log1 -p 0

# Verify objects in GCS
gcloud storage ls --recursive "gs://$BUCKET_NAME/**"
```

Expected output should show manifest and log files in the GCS bucket.

## Conversion Tool Details

The Rust tool performs the following transformations:

### Field Migrations

- `statefulset.nodeSelector` → `podTemplate.spec.nodeSelector`
- `statefulset.tolerations` → `podTemplate.spec.tolerations`
- `statefulset.podAffinity` → `podTemplate.spec.affinity.podAffinity`
- `statefulset.securityContext` → `podTemplate.spec.securityContext`
- `statefulset.priorityClassName` → `podTemplate.spec.priorityClassName`
- `statefulset.topologySpreadConstraints` → `podTemplate.spec.topologySpreadConstraints`
- `statefulset.terminationGracePeriodSeconds` → `podTemplate.spec.terminationGracePeriodSeconds`

### Deprecated Field Removal

The tool removes:
- `COMPUTED VALUES` header
- Root-level `tolerations` and `nodeSelector`
- `post_upgrade_job`
- `image.pullPolicy`
- `statefulset.initContainers.tuning`
- `statefulset.initContainers.setTieredStorageCacheDirOwnership`
- Empty `licenseSecretRef`
- Empty cloud storage configuration (when `cloud_storage_enabled: false`)

### Resource Format Conversion

Converts from old format:
```yaml
resources:
  cpu:
    cores: 1
  memory:
    container:
      max: 2.5Gi
```

To new format:
```yaml
resources:
  requests:
    cpu: 1
    memory: 2.5Gi
  limits:
    cpu: 1
    memory: 2.5Gi
```

## Important Notes

### TLS Configuration

The setup creates two certificate sets:
- **default**: For internal communication (admin API, internal Kafka/HTTP/Schema Registry)
- **external**: For external listeners (generated via `generate-certs.sh`)

### SASL Authentication

Default credentials:
- **Username**: `username`
- **Password**: `password`
- **Mechanism**: `SCRAM-SHA-256`

**IMPORTANT**: Always use `auth.sasl.secretRef` in values.yaml, never inline `auth.sasl.users`. The secretRef approach is required for proper operation with the config-watcher sidecar container.

### Node Affinity

Worker nodes are labeled with `nodetype=redpanda-pool1` and tainted with `redpanda-pool1=true:NoSchedule` to ensure dedicated scheduling for Redpanda pods.

### Upgrade Strategy

Always upgrade in this order:
1. **Chart first** (keeps same Redpanda version)
2. **Redpanda version** (in iterative major version steps)

Never skip major versions when upgrading Redpanda.

## Teardown

The `teardown.sh` script is highly destructive and will:
- Delete the entire `redpanda` namespace
- Delete the entire GCS bucket (`gs://$BUCKET_NAME`)
- Remove all node labels and taints
- Deactivate and delete HMAC keys

**Warning**: Review the script before running. Only use in test environments.

```bash
./teardown.sh
```

## Troubleshooting

### Chart Upgrade Fails with Schema Validation Errors

Run the conversion tool again to ensure all deprecated fields are removed:

```bash
cargo run $VALUES_FILE
```

Check the generated `updated-values.yaml` for any remaining deprecated fields.

### Pods Not Scheduling

Verify node taints and tolerations:

```bash
kubectl get nodes --show-labels
kubectl describe node redpanda-upgrade-worker
```

### TLS Connection Failures

Verify certificates are mounted correctly:

```bash
kubectl exec -n redpanda redpanda-0 -c redpanda -- ls -la /etc/tls/certs/default/
kubectl exec -n redpanda redpanda-0 -c redpanda -- ls -la /etc/tls/certs/external/
```

## Additional Resources

- [Redpanda Documentation](https://docs.redpanda.com/)
- [Redpanda Helm Chart](https://github.com/redpanda-data/helm-charts)
- [Redpanda License Portal](https://license.redpanda.com/)
