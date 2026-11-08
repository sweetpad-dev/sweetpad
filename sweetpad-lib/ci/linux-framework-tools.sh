#!/usr/bin/env bash
# Install the frozen hybrid fixture tools without changing system Ruby or Node.
set -euo pipefail
[[ "$(uname -s)" == Linux ]] || { echo "Run this tool setup on Linux." >&2; exit 1; }
ci="$(cd "$(dirname "$0")" && pwd)"
prefix="${1:-$HOME/.cache/sweetpad/framework-tools}"
mkdir -p "$prefix"
prefix="$(cd "$prefix" && pwd)"
case "$(uname -m)" in aarch64) node_arch=arm64;; x86_64) node_arch=x64;; *) exit 1;; esac
node_version=24.22.2

if [[ ! -x "$prefix/node/bin/node" ]]; then
  python3 - "$prefix" "$node_version" "$node_arch" <<'PY'
import hashlib, sys, tarfile, urllib.request
from pathlib import Path
root, version, arch = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
name = f'node-v{version}-linux-{arch}.tar.xz'
base = f'https://nodejs.org/dist/v{version}/'
manifest = urllib.request.urlopen(base + 'SHASUMS256.txt').read().decode()
digest = next(line.split()[0] for line in manifest.splitlines() if line.split()[-1] == name)
archive = root / name
urllib.request.urlretrieve(base + name, archive)
assert hashlib.sha256(archive.read_bytes()).hexdigest() == digest
with tarfile.open(archive) as source: source.extractall(root, filter='data')
(root / name.removesuffix('.tar.xz')).rename(root / 'node')
archive.unlink()
PY
fi
export PATH="$prefix/node/bin:$prefix/ruby/bin:$PATH"
[[ "$(node --version)" == "v$node_version" ]]

if [[ ! -x "$prefix/ruby/bin/ruby" ]]; then
  python3 - "$prefix" <<'PY'
import hashlib, sys, tarfile, urllib.request
from pathlib import Path
root = Path(sys.argv[1]); archive = root / 'ruby-3.4.8.tar.xz'
urllib.request.urlretrieve('https://cache.ruby-lang.org/pub/ruby/3.4/ruby-3.4.8.tar.xz', archive)
# Ruby's published release checksum, not an unsigned local lock update.
assert hashlib.sha256(archive.read_bytes()).hexdigest() == '53a8ec71111449cbbd42224d8d27c493fa6ded228636731051c48604d4255d68'
with tarfile.open(archive) as source: source.extractall(root, filter='data')
archive.unlink()
PY
  (
    cd "$prefix/ruby-3.4.8"
    ./configure --prefix="$prefix/ruby" --disable-install-doc --disable-yjit
    make -j "${SWEETPAD_BUILD_JOBS:-4}"
    make install
  ) > "$prefix/ruby-build.log" 2>&1
fi
[[ "$(ruby -e 'print RUBY_VERSION')" == 3.4.8 ]]
if [[ "$(bundle --version 2>/dev/null || true)" != 4.0.20 ]]; then
  gem install bundler -v 4.0.20 --no-document
fi
mkdir -p "$prefix/js"
cp "$ci/framework-tools/package.json" "$ci/framework-tools/package-lock.json" "$prefix/js/"
npm ci --prefix "$prefix/js" --no-audit --no-fund
export PATH="$prefix/js/node_modules/.bin:$PATH"
[[ "$(node --version)" == v24.22.2 && "$(bun --version)" == 1.3.14 && "$(bundle --version)" == 4.0.20 ]]
python3 - "$prefix" <<'PY'
import json, shlex, subprocess, sys
from pathlib import Path
root = Path(sys.argv[1])
tools = {name: subprocess.check_output([name, '--version'], text=True).strip() for name in ['node', 'bun', 'ruby', 'bundle']}
(root / 'tools.json').write_text(json.dumps(tools, indent=2) + '\n')
paths = [root/'js/node_modules/.bin', root/'node/bin', root/'ruby/bin']
(root / 'env.sh').write_text('export PATH=' + shlex.quote(':'.join(map(str, paths))) + ':"$PATH"\n')
print('Tool receipt:', root / 'tools.json')
print('Source environment:', root / 'env.sh')
PY
