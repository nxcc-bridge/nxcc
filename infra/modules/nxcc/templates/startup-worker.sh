#!/bin/bash
# NXCC Worker Node Startup Script
set -euo pipefail

# Logging
exec > >(tee /var/log/nxcc-startup.log) 2>&1
echo "Starting NXCC worker node startup at $(date)"

# Template variables (substituted by Terraform)
# shellcheck disable=SC2154
DOCKER_IMAGE="${docker_image}"
# shellcheck disable=SC2034,SC2154
NODE_TYPE="${node_type}"
# shellcheck disable=SC2034,SC2154
ENVIRONMENT="${environment}"
# shellcheck disable=SC2034,SC2154
NAMESPACE="${namespace}"
# shellcheck disable=SC2154
OPERATOR_KEY="${operator_key}"

# Update system and install required packages
apt-get update
apt-get install -y curl wget gnupg lsb-release jq

# Install Google Cloud SDK if not present (needed for Secret Manager access)
if ! command -v gcloud &>/dev/null; then
	echo "Installing Google Cloud SDK..."
	echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" | tee -a /etc/apt/sources.list.d/google-cloud-sdk.list
	curl https://packages.cloud.google.com/apt/doc/apt-key.gpg | apt-key --keyring /usr/share/keyrings/cloud.google.gpg add -
	apt-get update && apt-get install -y google-cloud-sdk
fi

# Install Docker
if ! command -v docker &>/dev/null; then
	echo "Installing Docker..."
	curl -fsSL https://get.docker.com | sh
	usermod -aG docker ubuntu
	systemctl enable docker
	systemctl start docker
fi

# Configure Docker for GCP Artifact Registry authentication (if using GCP registry)
if [[ "$DOCKER_IMAGE" == *"pkg.dev"* ]]; then
	echo "Configuring Docker authentication for GCP Artifact Registry..."
	gcloud auth configure-docker --quiet
	# For specific regions, also configure regional endpoints
	if [[ "$DOCKER_IMAGE" == *"europe-west4-docker.pkg.dev"* ]]; then
		gcloud auth configure-docker europe-west4-docker.pkg.dev --quiet
	elif [[ "$DOCKER_IMAGE" == *"us-central1-docker.pkg.dev"* ]]; then
		gcloud auth configure-docker us-central1-docker.pkg.dev --quiet
	fi
fi

# Pull NXCC Docker image
echo "Pulling NXCC Docker image: $DOCKER_IMAGE"
docker pull "$DOCKER_IMAGE"

# Create NXCC directories
mkdir -p /opt/nxcc/{config,data,logs}
chown -R ubuntu:ubuntu /opt/nxcc

# Retrieve and setup operator key from GCP Secret Manager
if [[ -n "$OPERATOR_KEY" ]]; then
	echo "Setting up operator key..."
	if [[ "$OPERATOR_KEY" == "projects/"* ]]; then
		# OPERATOR_KEY is a secret name, fetch from Secret Manager
		echo "Fetching operator key from Secret Manager: $OPERATOR_KEY"
		if command -v gcloud &>/dev/null; then
			gcloud secrets versions access latest --secret="$OPERATOR_KEY" >/opt/nxcc/config/operator.key
		else
			echo "ERROR: gcloud CLI not available for secret access"
			exit 1
		fi
	else
		# OPERATOR_KEY is the key data itself (base64 encoded)
		echo "$OPERATOR_KEY" | base64 -d >/opt/nxcc/config/operator.key
	fi
	chown ubuntu:ubuntu /opt/nxcc/config/operator.key
	chmod 600 /opt/nxcc/config/operator.key
	echo "Operator key configured successfully"
else
	echo "No operator key provided - attestations will not include operator signatures"
fi

# Create systemd service
cat >/etc/systemd/system/nxcc.service <<EOF
[Unit]
Description=NXCC Worker Node
Documentation=https://docs.nxcc.dev
After=docker.service
Requires=docker.service
StartLimitBurst=3
StartLimitInterval=60

[Service]
Type=simple
Restart=always
RestartSec=10
User=ubuntu
Group=ubuntu

# Stop existing container
ExecStartPre=-/usr/bin/docker stop nxcc
ExecStartPre=-/usr/bin/docker rm nxcc

# Start NXCC container (fully unconfined - no isolation)
ExecStart=/usr/bin/docker run \\
    --name nxcc \\
    --network host \\
    --privileged \\
    --rm \\
    --pid host \\
    --ipc host \\
    --uts host \\
    --cgroupns host \\
    --security-opt apparmor=unconfined \\
    --security-opt seccomp=unconfined \\
    --security-opt label=disable \\
    --cap-add=ALL \\
    --device-cgroup-rule='a *:* rwm' \\
    -v /:/host:rw \\
    -v /opt/nxcc/config:/opt/nxcc/config:ro \\
    -v /opt/nxcc/data:/opt/nxcc/data \\
    -v /opt/nxcc/logs:/opt/nxcc/logs \\
    -v /var/run/docker.sock:/var/run/docker.sock \\
    -v /proc:/host/proc:rw \\
    -v /sys:/host/sys:rw \\
    -v /dev:/host/dev:rw \\
    -v /run:/host/run:rw \\
    -v /var:/host/var:rw \\
    -v /tmp:/host/tmp:rw \\
    -e NXCC_DAEMON_LISTEN_ADDRESSES="/ip4/0.0.0.0/tcp/9000" \
    -e NXCC_DAEMON_HTTP_LISTEN_ADDR="0.0.0.0:6922" \
    -e NXCC_DAEMON_API_ENABLED="true" \
    -e NXCC_DAEMON_TDX_ENABLED="true" \
    -e NXCC_DAEMON_OPERATOR_SIGNING_KEY_PATH="/opt/nxcc/config/operator.key" \\
    -e RUST_LOG=info \\
    -e RUST_BACKTRACE=1 \\
    "$DOCKER_IMAGE"

# Cleanup on stop
ExecStop=/usr/bin/docker stop nxcc

# Health check
ExecStartPost=/bin/bash -c 'sleep 10 && curl -f http://localhost:6922/api/status || exit 1'

[Install]
WantedBy=multi-user.target
EOF

# Enable and start NXCC service
systemctl daemon-reload
systemctl enable nxcc
systemctl start nxcc

# Setup log rotation
cat >/etc/logrotate.d/nxcc <<EOF
/var/log/nxcc-startup.log {
    weekly
    missingok
    rotate 4
    compress
    delaycompress
    copytruncate
}

/opt/nxcc/logs/*.log {
    daily
    missingok
    rotate 7
    compress
    delaycompress
    copytruncate
}
EOF

# Create monitoring script
cat >/opt/nxcc/health-check.sh <<EOF
#!/bin/bash
# NXCC Health Check Script

set -euo pipefail

# Check if service is running
if ! systemctl is-active --quiet nxcc; then
    echo "ERROR: NXCC service is not running"
    exit 1
fi

# Check if container is healthy
if ! docker ps | grep -q nxcc; then
    echo "ERROR: NXCC container is not running"
    exit 1
fi

# Check HTTP endpoint
if ! curl -sf http://localhost:6922/api/status > /dev/null; then
    echo "ERROR: NXCC HTTP API is not responding"
    exit 1
fi

echo "OK: NXCC worker node is healthy"
EOF

chmod +x /opt/nxcc/health-check.sh
chown ubuntu:ubuntu /opt/nxcc/health-check.sh

# Setup cron for health monitoring
echo "*/5 * * * * ubuntu /opt/nxcc/health-check.sh >> /opt/nxcc/logs/health.log 2>&1" >>/etc/crontab

echo "NXCC worker node startup completed successfully at $(date)"
echo "Service status:"
systemctl status nxcc --no-pager

echo "Container status:"
docker ps | grep nxcc || echo "Container not yet running"

echo "Logs location: /opt/nxcc/logs/"
echo "Config location: /opt/nxcc/config/"
echo "Health check: /opt/nxcc/health-check.sh"
