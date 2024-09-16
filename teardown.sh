#!/bin/bash

#set -euxo pipefail

#BUCKET_NAME=jlptest

# uninstall redpanda
helm uninstall redpanda -n redpanda --wait
helm uninstall cert-manager -n cert-manager --wait

# delete namespace
kubectl delete ns redpanda --wait=true

# delete storageclass
kubectl delete sc standard-redpanda

# delete node taint and labels
# TODO auto get node names and run the following commands for each node
kubectl label node jlp-cluster-worker nodetype-
kubectl label node jlp-cluster-worker2 nodetype-
kubectl label node jlp-cluster-worker3 nodetype-
kubectl taint node jlp-cluster-worker redpanda-pool=true:NoSchedule-
kubectl taint node jlp-cluster-worker2 redpanda-pool=true:NoSchedule-
kubectl taint node jlp-cluster-worker3 redpanda-pool=true:NoSchedule-

# delete google bucket
gcloud storage rm -r gs://$BUCKET_NAME

# get service account
GOOGLE_SERVICE_ACCOUNT=$(gcloud iam service-accounts list | tail -1 | awk '{print $6}')

# deactivate HMAC key
HMAC_ACCESS_KEY=$(cat hmac.json | jq -r .metadata.accessId)
gcloud storage hmac update $HMAC_ACCESS_KEY --deactivate

# delete HMAC key
gcloud storage hmac delete $HMAC_ACCESS_KEY

# cleanup
rm hmac.json

