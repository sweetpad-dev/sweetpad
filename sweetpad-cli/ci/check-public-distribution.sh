#!/usr/bin/env bash
# Validate the published release as a consumer in a disposable container.
set -euo pipefail
[[ -f /.dockerenv ]] || { echo 'Run this check in a disposable Docker container' >&2; exit 1; }
: "${CLI_VERSION:?set CLI_VERSION to the published stable release}"
[[ "$CLI_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || exit 1
export CLI_VERSION
export PATH="/opt/sweetpad-bin:$PATH"
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -yqq --no-install-recommends ca-certificates curl gnupg python3
mkdir -p /root/.swiftpm/swift-sdks /reports
printf 'private state' > /root/.swiftpm/swift-sdks/validation-marker
curl -fsSLo /tmp/key.asc https://sweetpad-dev.github.io/sweetpad/apt/sweetpad-archive-keyring.asc
FINGERPRINT=$(gpg --batch --with-colons --show-keys /tmp/key.asc | awk -F: '/^fpr:/ {print $10; exit}')
test "$FINGERPRINT" = "$(cat "$(dirname "$0")/apt/signing-key.fingerprint")"
install -Dm644 /tmp/key.asc /etc/apt/keyrings/sweetpad.asc
cat > /etc/apt/sources.list.d/sweetpad.sources <<'EOF'
Types: deb
URIs: https://sweetpad-dev.github.io/sweetpad/apt
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/sweetpad.asc
EOF
# Allow the Pages CDN to pick up the newly deployed index.
for attempt in {1..12}; do
  apt-get update -o APT::Update::Error-Mode=any
  if apt-cache madison sweetpad | grep -Fq " $CLI_VERSION-1 "; then break; fi
  sleep 10
done
apt-get install -y --no-install-recommends "sweetpad=$CLI_VERSION-1"
test "$(sweetpad --version)" = "sweetpad $CLI_VERSION"
sweetpad self-update --json > /reports/apt-self-update.json
python3 -c 'import json; assert json.load(open("/reports/apt-self-update.json"))["data"]["method"] == "apt"'
sweetpad-dap --help > /reports/apt-dap-help.txt
dpkg-query -W -f='${Version}' sweetpad > /reports/debian-version.txt
apt-get remove -y sweetpad
test ! -e /usr/bin/sweetpad
test "$(cat /root/.swiftpm/swift-sdks/validation-marker)" = 'private state'
ARCH=$(uname -m)
case "$ARCH" in aarch64) TOOL_ARCH=arm64;; x86_64) TOOL_ARCH=x64;; *) exit 1;; esac
/tools/packslip-v1.11.3-linux-$TOOL_ARCH install github.com/sweetpad-dev/sweetpad/tools/cli --version "$CLI_VERSION" --install-dir /opt/sweetpad --bin-dir /opt/sweetpad-bin > /reports/packslip-install.txt
test "$(/opt/sweetpad-bin/sweetpad --version)" = "sweetpad $CLI_VERSION"
/opt/sweetpad-bin/sweetpad self-update --json > /reports/packslip-self-update.json
python3 -c 'import json; assert json.load(open("/reports/packslip-self-update.json"))["data"]["method"] == "packslip"'
/opt/sweetpad-bin/sweetpad-dap --help > /reports/packslip-dap-help.txt
python3 - <<'PY'
from pathlib import Path
import json,platform,os
Path('/reports/summary.json').write_text(json.dumps({'passed':True,'hostArchitecture':platform.machine(),'version':os.environ['CLI_VERSION'],'publicAptInstall':True,'publicPackslipInstall':True,'dapAlias':True,'ownerAwareUpgradeGuidance':True,'privateStatePreserved':True},indent=2)+'\n')
PY
