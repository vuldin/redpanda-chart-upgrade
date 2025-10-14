#!/bin/bash

# Run docker network inspect for network 'kind'
output=$(docker network inspect kind)

# Extract the subnet string from the output using jq
subnet=$(echo "$output" | jq -r '.[0].IPAM.Config[0].Subnet')

# Split subnet into base IP and mask
IFS='/' read -r base_ip mask <<< "$subnet"

# Extract first two octets for /16
IFS='.' read -r oct1 oct2 oct3 oct4 <<< "$base_ip"

# Start IP for MetalLB pool - e.g., 172.19.255.200
start_ip="${oct1}.${oct2}.255.200"

# End IP for MetalLB pool - e.g., 172.19.255.250
end_ip="${oct1}.${oct2}.255.250"

# Output the calculated range
echo "Docker network subnet: ${subnet}"
echo "MetalLB IP range: ${start_ip}-${end_ip}"

# Create metallb-config.yaml with calculated IP range
cat > metallb-config.yaml <<EOF
apiVersion: metallb.io/v1beta1
kind: IPAddressPool
metadata:
  name: default-pool
  namespace: metallb-system
spec:
  addresses:
  - ${start_ip}-${end_ip}
---
apiVersion: metallb.io/v1beta1
kind: L2Advertisement
metadata:
  name: default-l2
  namespace: metallb-system
spec:
  ipAddressPools:
  - default-pool
EOF

echo ""
echo "Created metallb-config.yaml with IP range ${start_ip}-${end_ip}"
