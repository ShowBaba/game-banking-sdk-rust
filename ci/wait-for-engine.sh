#!/usr/bin/env sh
# Waits until the engine answers /v1/health/ready (migrated and connected).
url="${GBE_BASE_URL:-http://localhost:8080}/v1/health/ready"
for i in $(seq 1 60); do
  if curl -fsS "$url" >/dev/null 2>&1; then echo "engine ready at $url"; exit 0; fi
  sleep 2
done
echo "engine did not become ready at $url" >&2
docker compose -f "$(dirname "$0")/docker-compose.yml" logs engine >&2 || true
exit 1
