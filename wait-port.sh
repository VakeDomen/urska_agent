#!/bin/sh
# wait-port.sh HOST PORT [TIMEOUT]
HOST=$1
PORT=$2
TIMEOUT=${3:-30}

echo "⏳ Waiting for $HOST:$PORT … (max ${TIMEOUT}s)"
count=0
while ! nc -z "$HOST" "$PORT" 2>/dev/null; do
    sleep 1
    count=$((count+1))
    if [ "$count" -ge "$TIMEOUT" ]; then
        echo "❌ Timed out waiting for $HOST:$PORT"
        exit 1
    fi
done
echo "✅ $HOST:$PORT is up"
