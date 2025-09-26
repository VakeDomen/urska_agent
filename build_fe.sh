#!/usr/bin/env bash
set -euo pipefail

# usage: sudo ./build_fe.sh /path/to/out

if [[ "${EUID}" -ne 0 ]]; then
  echo "please run this script with sudo"
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required but not found"
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FRONTEND_DIR="${SCRIPT_DIR}/frontend"

if [[ ! -d "${FRONTEND_DIR}" ]]; then
  echo "missing ./frontend directory next to this script"
  exit 1
fi

if [[ $# -lt 1 ]]; then
  echo "usage: sudo $0 /absolute/or/relative/output/dir"
  exit 1
fi

OUT_DIR="$1"
mkdir -p "${OUT_DIR}"

HOST_UID="${SUDO_UID:-$(id -u)}"
HOST_GID="${SUDO_GID:-$(id -g)}"

FRONTEND_DIR_ABS="$(cd "${FRONTEND_DIR}" && pwd)"
OUT_DIR_ABS="$(cd "${OUT_DIR}" && pwd)"

NODE_IMAGE="node:20-bookworm"

VOL_NODE_MODULES="ng_node_modules_$$"
VOL_NPM_CACHE="ng_npm_cache_$$"
VOL_ANGULAR_CACHE="ng_angular_cache_$$"

docker volume create "${VOL_NODE_MODULES}" >/dev/null
docker volume create "${VOL_NPM_CACHE}" >/dev/null
docker volume create "${VOL_ANGULAR_CACHE}" >/dev/null

docker run --rm \
  -e CI=true \
  -e NG_CLI_ANALYTICS=false \
  -e HOME=/tmp \
  -v "${FRONTEND_DIR_ABS}:/app:ro" \
  -v "${VOL_NODE_MODULES}:/app/node_modules" \
  -v "${VOL_NPM_CACHE}:/tmp/.npm" \
  -v "${VOL_ANGULAR_CACHE}:/app/.angular" \
  -v "${OUT_DIR_ABS}:/out" \
  -w /app \
  "${NODE_IMAGE}" \
  bash -lc '
    set -e
    mkdir -p /tmp/.npm /app/.angular

    if [ -f pnpm-lock.yaml ]; then
      corepack enable pnpm
      pnpm install --frozen-lockfile
      pnpm dlx @angular/cli@20 build --configuration production --output-path /out
    elif [ -f yarn.lock ]; then
      corepack enable yarn
      yarn install --frozen-lockfile
      npx -y @angular/cli@20 build --configuration production --output-path /out
    else
      if [ -f package-lock.json ]; then
        npm ci
      else
        npm install
      fi
      npx -y @angular/cli@20 build --configuration production --output-path /out
    fi
  '

chown -R "${HOST_UID}:${HOST_GID}" "${OUT_DIR_ABS}"

docker volume rm "${VOL_NODE_MODULES}" "${VOL_NPM_CACHE}" "${VOL_ANGULAR_CACHE}" >/dev/null

echo "build complete, files are in: ${OUT_DIR_ABS}"
