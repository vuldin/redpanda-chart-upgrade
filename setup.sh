#!/bin/bash

set -euxo pipefail

# create namespace
kubectl create ns redpanda

# create SASL secret
echo 'username:password:SCRAM-SHA-256' > superusers.txt
kubectl create secret generic redpanda-superusers -n redpanda --from-file=superusers.txt

# create license secret
kubectl create secret generic redpanda-license -n redpanda --from-file=redpanda.license

# generate external cert
curl -sLO https://gist.githubusercontent.com/vuldin/e4b4a776df6dc0b4593302437ea57eed/raw/generate-certs.sh
chmod +x generate-certs.sh
./generate-certs.sh $REDPANDA_EXTERNAL_DOMAIN

# create tls secret
kubectl create secret generic tls-external -n redpanda --from-file=ca.crt=certs/ca.crt --from-file=tls.crt=certs/node.crt --from-file=tls.key=certs/node.key

# create google bucket
gcloud storage buckets create gs://$BUCKET_NAME --uniform-bucket-level-access --no-public-access-prevention --location=$BUCKET_REGION

# get service account
GOOGLE_SERVICE_ACCOUNT=$(gcloud iam service-accounts list | tail -1 | awk '{print $6}')

# create HMAC key and set related envars
gcloud storage hmac create $GOOGLE_SERVICE_ACCOUNT --format=json > hmac.json
HMAC_ACCESS_KEY=$(cat hmac.json | jq -r .metadata.accessId)
HMAC_SECRET_KEY=$(cat hmac.json | jq -r .secret)

# update values.yaml with bucket and HMAC values
accessKey=$HMAC_ACCESS_KEY yq -i '(.storage.tieredConfig.cloud_storage_access_key = strenv(accessKey))' $VALUES_FILE
secretKey=$HMAC_SECRET_KEY yq -i '(.storage.tieredConfig.cloud_storage_secret_key = strenv(secretKey))' $VALUES_FILE
bucketName=$BUCKET_NAME yq -i '(.storage.tieredConfig.cloud_storage_bucket = strenv(bucketName))' $VALUES_FILE
bucketRegion=$BUCKET_REGION yq -i '(.storage.tieredConfig.cloud_storage_region = strenv(bucketRegion))' $VALUES_FILE

# apply node taint and labels
# TODO auto get node names and run the following commands for each node
kubectl label node jlp-cluster-worker nodetype=redpanda-pool
kubectl label node jlp-cluster-worker2 nodetype=redpanda-pool
kubectl label node jlp-cluster-worker3 nodetype=redpanda-pool
kubectl taint node jlp-cluster-worker redpanda-pool=true:NoSchedule
kubectl taint node jlp-cluster-worker2 redpanda-pool=true:NoSchedule
kubectl taint node jlp-cluster-worker3 redpanda-pool=true:NoSchedule

# create storageClass
cat <<EOF | kubectl apply -f -
apiVersion: storage.k8s.io/v1
kind: StorageClass
metadata:
  name: standard-redpanda
provisioner: rancher.io/local-path
volumeBindingMode: WaitForFirstConsumer
EOF

# deploy cert-manager
helm install cert-manager cert-manager --repo https://charts.jetstack.io --set crds.enabled=true --namespace cert-manager --create-namespace

# deploy redpanda
helm upgrade --install redpanda redpanda --repo https://charts.redpanda.com -n redpanda --wait --timeout 2h --create-namespace --version $CHART_VERSION -f $VALUES_FILE

# cleanup
rm -rf certs private-ca-key tls-external.yaml superusers.txt generate-certs.sh

