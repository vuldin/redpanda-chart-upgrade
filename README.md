## Prerequisites

- refresh SSO credentials for cloud CLI
- kind (optional, any Kubernetes environment should work but kind is used below)
- helm

## Create initial cluster state

These steps will create a cluster in the initial state we want in order to be prepared for an upgrade. The initial cluster state is an older Redpanda version deployed with an older helm chart version.

Prepare your Kubernetes environment. These instructions use kind, but any Kubernetes deployment will work. Create a kind config that spins up a control plane and 3 worker nodes (for 3 Redpanda brokers):

```
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
```

Start Kubernetes:

```
kind create cluster --name jlp-cluster --config kind-config.yaml
```

Place a valid Redpanda license in `redpanda.license`. Your CS representative can generate a temporary license for testing from https://license.redpanda.com/ (or you can use your existing license). The licence is required for enabling tiered storage.

Validate and/or update the environment variables:

```
vi .env
```

By default chart v5.0.10 will be used and the existing config will be gathered from `values.yaml` in the current directory (if no changes are made to `.env`). Once validated, set the environment variables:

```
export $(cat .env | xargs)
```

Copy your values.yaml file from your existing deployment Deploy Redpanda in initial state:

```
./setup.sh
```

## Verify state

Verify cluster status, health, and TLS config for both the Kafka and admin listeners:

```
# verify external Kafka listener connectivity
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk cluster info --brokers redpanda.redpanda:9094 --tls-enabled --tls-truststore /etc/tls/certs/external/ca.crt --user username --password password

# verify internal Kafka listener connectivity
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk cluster info --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore /etc/tls/certs/default/ca.crt --user username --password password

# verify admin listener
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk cluster health --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

Test tiered storage (original instructions [here](https://gist.github.com/vuldin/d6e4c56115ad8d7a3c5ff12438ecf5d7)):

```
# create topic
kubectl exec -it -n redpanda redpanda-0 -c redpanda -- rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore /etc/tls/certs/default/ca.crt --user username --password password topic create log1 -c redpanda.remote.read=true -c redpanda.remote.write=true

# create data
kubectl -n redpanda exec -ti redpanda-0 -c redpanda -- bash
BATCH=$(date); printf "$BATCH %s\n" {1..100000} | rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore=/etc/tls/certs/default/ca.crt --user username --password password topic produce log1 -p 0
BATCH=$(date); printf "$BATCH %s\n" {1..100000} | rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore=/etc/tls/certs/default/ca.crt --user username --password password topic produce log1 -p 1
BATCH=$(date); printf "$BATCH %s\n" {1..100000} | rpk --brokers redpanda-0.redpanda.redpanda:9093,redpanda-1.redpanda.redpanda:9093,redpanda-2.redpanda.redpanda:9093 --tls-enabled --tls-truststore=/etc/tls/certs/default/ca.crt --user username --password password topic produce log1 -p 2
```

Once the above commands are ran, check the bucket for any created objects:

```
gcloud storage ls --recursive "gs://$BUCKET_NAME/**"
```

You should see something similar to the following output (`<bucket-name>` will be the value of `$BUCKET_NAME`):

```
gs://<bucket-name>/10000000/meta/kafka/log1/1_27/manifest.bin
gs://<bucket-name>/40000000/meta/kafka/log1/topic_manifest.json
gs://<bucket-name>/6c4e8f1c/kafka/log1/0_27/0-100002-867329-1-v1.log.1
gs://<bucket-name>/6c4e8f1c/kafka/log1/0_27/0-100002-867329-1-v1.log.1.index
gs://<bucket-name>/a0000000/meta/kafka/log1/0_27/manifest.bin
gs://<bucket-name>/bb30efb9/kafka/log1/1_27/0-100002-876549-1-v1.log.1
gs://<bucket-name>/bb30efb9/kafka/log1/1_27/0-100002-876549-1-v1.log.1.index
gs://<bucket-name>/c0000000/meta/kafka/log1/2_27/manifest.bin
gs://<bucket-name>/cc89f375/kafka/log1/2_27/0-2-277-1-v1.log.2
gs://<bucket-name>/cc89f375/kafka/log1/2_27/0-2-277-1-v1.log.2.index
gs://<bucket-name>/eac0a263/kafka/log1/2_27/3-100005-891850-2-v1.log.2
gs://<bucket-name>/eac0a263/kafka/log1/2_27/3-100005-891850-2-v1.log.2.index
```

TODO: test other enabled features?

## Helm chart upgrade

Transform the current chart config so that it is compatible with latest chart:

```
cargo run $VALUES_FILE
```

This will create the file `updated-values.yaml`.

Upgrade the chart version to latest by using the updated file:

```
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace -f updated-values.yaml
```

This will keep the same Redpanda version while upgrading the chart version. Verify Redpanda versions remain the same while the chart version is being upgraded:

```
kubectl exec -it -n redpanda redpanda-2 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

Once chart upgrade is complete, run the following command to verify:

```
helm list -n redpanda
```

## Redpanda upgrade

Now we can focus on upgrading Redpanda. Get your current Redpanda version:

```
kubectl exec -it -n redpanda redpanda-2 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

In this example we are running Redpanda `v23.2.24`.

Get a list of all available Redpanda versions:

```
curl -s 'https://hub.docker.com/v2/repositories/redpandadata/redpanda/tags/?ordering=last_updated&page=1&page_size=50' | jq -r '.results[].name' | grep -v 64 | grep -v latest | sort
```

We will take the following upgrade path:

1. `v23.2.24` to `v23.3.20`
2. `v23.3.20` to `v24.1.16`
3. `v24.1.16` to `v24.2.4`

Upgrade 1:

```
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace -f updated-values-9.yaml --set image.tag=v23.3.20
```

Verify version:

```
kubectl exec -it -n redpanda redpanda-2 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

Upgrade 2:

```
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace -f updated-values-9.yaml --set image.tag=v24.1.16
```

Verify version:

```
kubectl exec -it -n redpanda redpanda-2 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

Upgrade 3 (final):

```
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace -f updated-values-9.yaml --set image.tag=v24.2.4
```

Verify version (final):

```
kubectl exec -it -n redpanda redpanda-2 -c redpanda -- rpk redpanda admin brokers list --api-urls redpanda.redpanda:9644 -X admin.tls.enabled=true -X admin.tls.ca=/etc/tls/certs/default/ca.crt
```

## Teardown

Once the test is complete, `teardown.sh` can be ran to delete the cluster, bucket, all Kubernetes resources, and also the remaining local files created during testing.

> Warning: This is very destructive! This script will delete everything in the `redpanda` namespace, and the entire GCP object storage bucket that was provided in `.env`. Do not run this script without first understanding the commands it runs, and understanding the consequences.

```
./teardown.sh
```

