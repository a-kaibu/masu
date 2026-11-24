#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."

# Keep local and CI runs on the same image: RustFS by its digest, the release ci-bundles' copy
# (tools/ci/mirror.txt; not Docker Hub at run time), loaded once under its own tag. No host data
# or credentials are mounted.
image=masu-ci/rustfs-ed9210d6411d
if ! docker image inspect "$image" >/dev/null 2>&1; then
    t=$(mktemp)
    tools/ci/fetch.sh rustfs-ed9210d6411d.tar.zst "$t"
    zstd -dc "$t" | docker load -q
    rm -f "$t"
fi
name="masu-rustfs-${GITHUB_RUN_ID:-local}-$$"
export AWS_ACCESS_KEY_ID=masu-ci-access
export AWS_SECRET_ACCESS_KEY=masu-ci-secret-only-for-tests
export AWS_EC2_METADATA_DISABLED=true
cleanup() {
    status=$?
    if [ "$status" -ne 0 ]; then docker logs "$name" >&2 || true; fi
    docker rm -fv "$name" >/dev/null 2>&1 || true
    exit "$status"
}
trap cleanup EXIT
docker run -d --name "$name" -p 127.0.0.1::9000 \
    -e RUSTFS_ACCESS_KEY="$AWS_ACCESS_KEY_ID" \
    -e RUSTFS_SECRET_KEY="$AWS_SECRET_ACCESS_KEY" \
    -e RUSTFS_CONSOLE_ENABLE=false \
    "$image" /data >/dev/null
port=$(docker port "$name" 9000/tcp | head -1)
export MASU_TEST_S3_ENDPOINT="http://$port"
ready=false
for _ in {1..60}; do
    if curl --fail --silent --max-time 2 "$MASU_TEST_S3_ENDPOINT/health/ready" >/dev/null; then
        ready=true
        break
    fi
    sleep 1
done
if [ "$ready" != true ]; then
    echo 'RustFS did not become ready' >&2
    exit 1
fi
# (--release: CI's cargo test built the workspace there, so only the test binary is reused)
cargo test --release -p masu --bin masu storage::rustfs_tests:: -- --ignored --nocapture
