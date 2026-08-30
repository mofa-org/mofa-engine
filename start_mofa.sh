#!/usr/bin/env bash
# ==============================================================================
# MoFA Stack Launcher Script
# Starts Grafana/Prometheus, Ollama, Kokoro TTS, MoFA Engine, & React Frontend
# ==============================================================================

set -e

ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "[LAUNCH] Starting MoFA Full Stack..."

# 1. Observability (Grafana & Prometheus Docker)
echo "1⃣  Starting Observability (Grafana & Prometheus)..."
cd "$ENGINE_DIR/mofa-observability/docker"
docker compose up -d 2>/dev/null || docker-compose up -d

# 2. Check/Start Ollama
echo "2⃣  Checking Ollama status..."
if ! pgrep -x "ollama" > /dev/null; then
    echo "Starting Ollama serve in background..."
    ollama serve > /tmp/ollama.log 2>&1 &
    sleep 2
else
    echo "Ollama is already running."
fi

# 3. Start Kokoro TTS Server
echo "3⃣  Starting Kokoro TTS Server..."
cd "$ENGINE_DIR"
if [ -d ".kokoro-venv" ]; then
    source .kokoro-venv/bin/activate
fi
if ! pgrep -f "kokoro_tts_server.py" > /dev/null; then
    python3 kokoro_tts_server.py > /tmp/kokoro.log 2>&1 &
    echo "Kokoro TTS Server running on port 8421."
else
    echo "Kokoro TTS Server is already running."
fi

# 4. Start MoFA Engine Core
echo "4⃣  Starting MoFA Engine Daemon (Port 8420)..."
cd "$ENGINE_DIR"
cargo run --release -p mofa-engine -- -c mofa_hybrid.toml
