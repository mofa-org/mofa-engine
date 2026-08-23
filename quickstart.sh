#!/usr/bin/env bash
# ==============================================================================
# MoFA Engine — 5-Minute Quickstart Launcher
# ==============================================================================
# Starts all components (Ollama, Engine Daemon, Web Studio Frontend)
# or runs automated scenario verification and benchmarks.
#
# Usage:
#   bash quickstart.sh            # Start backend daemon + frontend studio
#   bash quickstart.sh demo       # Run all 7 scenario demos in standalone mode
#   bash quickstart.sh benchmark  # Run performance baseline and latency benchmarks
#   bash quickstart.sh test       # Run end-to-end integration test suite
#   bash quickstart.sh status     # Check health of running services
#   bash quickstart.sh doctor     # Diagnose system dependencies & providers
# ==============================================================================

set -e

GREEN="\033[32m"
BLUE="\033[34m"
YELLOW="\033[33m"
CYAN="\033[36m"
BOLD="\033[1m"
RESET="\033[0m"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

echo -e "${BOLD}${BLUE}==================================================================${RESET}"
echo -e "${BOLD}${CYAN}   MoFA Engine — Multimodal Orchestration for Artifacts${RESET}"
echo -e "${BOLD}${BLUE}==================================================================${RESET}\n"

# Mode: Doctor Diagnostic
if [ "$1" = "--doctor" ] || [ "$1" = "doctor" ] || [ "$1" = "-doc" ] || [ "$1" = "doc" ]; then
    python3 mofa-fm/mofa_doctor.py
    exit 0
fi

# Mode: Help
if [ "$1" = "--help" ] || [ "$1" = "help" ] || [ "$1" = "-h" ]; then
    echo -e "Usage: bash quickstart.sh [COMMAND]\n"
    echo -e "Commands:"
    echo -e "  (no args)     Start full stack (Engine Core, Kokoro TTS, Web Studio UI)"
    echo -e "  doctor        Diagnose system dependencies, provider gateways & scenarios"
    echo -e "  status        Check health and port status of all running services"
    echo -e "  demo          Run the 30-second multimodal golden path demo suite"
    echo -e "  benchmark     Run real-time multi-provider race & latency benchmark"
    echo -e "  test          Run end-to-end scenario integration test suite"
    echo -e "  stop          Stop all running MoFA services (Engine, Frontend, Kokoro)"
    echo -e "  help          Show this help message\n"
    exit 0
fi

# Mode: Stop / Kill Services
if [ "$1" = "--stop" ] || [ "$1" = "stop" ] || [ "$1" = "-k" ] || [ "$1" = "kill" ]; then
    echo -e "${YELLOW}Stopping MoFA services...${RESET}"
    pkill -f "target.*mofa-engine" > /dev/null 2>&1 && echo -e "  +- MoFA Engine Core stopped" || true
    pkill -f "vite" > /dev/null 2>&1 && echo -e "  +- Web Studio Frontend stopped" || true
    pkill -f "kokoro_tts_server.py" > /dev/null 2>&1 && echo -e "  +- Kokoro TTS Server stopped" || true
    echo -e "${GREEN}All MoFA services stopped.${RESET}\n"
    exit 0
fi

# Mode: Golden Path Demo Runner
if [ "$1" = "--demo" ] || [ "$1" = "demo" ] || [ "$1" = "-d" ]; then
    echo -e "${CYAN}Launching MoFA 30-Second Multimodal Golden Path Demo...${RESET}\n"
    python3 examples/quickstart_demo.py
    exit 0
fi

# Mode: Benchmark Runner
if [ "$1" = "--benchmark" ] || [ "$1" = "benchmark" ] || [ "$1" = "-b" ] || [ "$1" = "bench" ]; then
    echo -e "${YELLOW}Running Real-Time Multi-Provider Race & Warmup Benchmark...${RESET}\n"
    python3 examples/01_provider_race.py
    exit 0
fi

# Mode: Test Runner
if [ "$1" = "--test" ] || [ "$1" = "test" ] || [ "$1" = "-t" ]; then
    echo -e "${YELLOW}Running MoFA End-to-End Scenario Integration Tests...${RESET}\n"
    python3 -m unittest tests/integration/test_e2e_scenarios.py
    exit 0
fi

# Mode: Status Check
if [ "$1" = "--status" ] || [ "$1" = "status" ] || [ "$1" = "-s" ] || [ "$1" = "ps" ]; then
    echo -e "${CYAN}Checking MoFA stack status...${RESET}\n"
    
    # Check Engine
    if curl -s http://127.0.0.1:8420/health > /dev/null 2>&1; then
        echo -e "  * MoFA Engine Core (8420)   : ${GREEN}[ONLINE] Running (Healthy)${RESET}"
    else
        echo -e "  * MoFA Engine Core (8420)   : ${YELLOW}[OFFLINE] Stopped${RESET}"
    fi

    # Check Frontend
    if curl -s http://localhost:3000 > /dev/null 2>&1; then
        echo -e "  * Web Studio Frontend (3000): ${GREEN}[ONLINE] Running${RESET}"
    else
        echo -e "  * Web Studio Frontend (3000): ${YELLOW}[OFFLINE] Stopped${RESET}"
    fi

    # Check Ollama
    if curl -s http://127.0.0.1:11434/api/tags > /dev/null 2>&1; then
        echo -e "  * Ollama LLM Service (11434): ${GREEN}[ONLINE] Running${RESET}"
    else
        echo -e "  * Ollama LLM Service (11434): ${YELLOW}[OFFLINE] Stopped${RESET}"
    fi

    # Check Kokoro TTS
    if curl -s http://127.0.0.1:8421/health > /dev/null 2>&1 || pgrep -f "kokoro_tts_server.py" > /dev/null 2>&1; then
        echo -e "  * Kokoro Neural TTS (8421)  : ${GREEN}[ONLINE] Running${RESET}"
    else
        echo -e "  * Kokoro Neural TTS (8421)  : ${YELLOW}[OFFLINE] Stopped${RESET}"
    fi

    # Check Observability Stack
    if curl -s http://localhost:3001 > /dev/null 2>&1; then
        echo -e "  * Grafana Dashboard (3001)  : ${GREEN}[ONLINE] Running${RESET}"
    else
        echo -e "  * Grafana Dashboard (3001)  : ${YELLOW}[OFFLINE] Stopped${RESET}"
    fi

    if curl -s http://localhost:9091 > /dev/null 2>&1; then
        echo -e "  * Prometheus Metrics (9091) : ${GREEN}[ONLINE] Running${RESET}"
    else
        echo -e "  * Prometheus Metrics (9091) : ${YELLOW}[OFFLINE] Stopped${RESET}"
    fi
    exit 0
fi

# 1. Prerequisite Checks
echo -e "${BOLD}1. Checking Prerequisites...${RESET}"

if command -v cargo >/dev/null 2>&1; then
    echo -e "   +- Rust & Cargo : ${GREEN}[OK] Found ($(cargo --version | cut -d' ' -f2))${RESET}"
else
    echo -e "   +- Rust & Cargo : ${YELLOW}[WARN] Not found. Please install Rust via https://rustup.rs${RESET}"
fi

if command -v python3 >/dev/null 2>&1; then
    echo -e "   +- Python 3     : ${GREEN}[OK] Found ($(python3 --version))${RESET}"
else
    echo -e "   +- Python 3     : ${YELLOW}[WARN] Python 3 not found${RESET}"
fi

if command -v npm >/dev/null 2>&1; then
    echo -e "   +- Node / npm   : ${GREEN}[OK] Found ($(node --version))${RESET}"
else
    echo -e "   +- Node / npm   : ${YELLOW}[WARN] Node.js / npm not found (needed for Web Studio)${RESET}"
fi

if command -v ffmpeg >/dev/null 2>&1; then
    echo -e "   +- FFmpeg Media : ${GREEN}[OK] Found${RESET}"
else
    echo -e "   +- FFmpeg Media : ${YELLOW}[WARN] Optional (install via 'brew install ffmpeg' for video rendering)${RESET}"
fi

# 1.5. Check & Start Observability Stack (Prometheus + Grafana in Docker)
echo -e "\n${BOLD}1.5. Checking Observability Stack (Docker Prometheus & Grafana)...${RESET}"
if command -v docker >/dev/null 2>&1; then
    if [ -f "mofa-observability/docker/docker-compose.yml" ]; then
        if ! curl -s http://localhost:3001 > /dev/null 2>&1 && ! curl -s http://localhost:9091 > /dev/null 2>&1; then
            echo -e "   +- Launching Prometheus (:9091) & Grafana (:3001)..."
            (cd mofa-observability/docker && docker compose up -d > /dev/null 2>&1 || true)
            sleep 2
            echo -e "   +- ${GREEN}[ONLINE] Prometheus (:9091) & Grafana (:3001) are LIVE${RESET}"
        else
            echo -e "   +- ${GREEN}[ONLINE] Observability Stack is already running (Grafana :3001, Prometheus :9091)${RESET}"
        fi
    fi
else
    echo -e "   +- ${YELLOW}[INFO] Docker not found in PATH. Skipping Prometheus & Grafana container stack.${RESET}"
fi

# 2. Check & Start Ollama if present
echo -e "\n${BOLD}2. Checking Local Model Service (Ollama)...${RESET}"
if command -v ollama >/dev/null 2>&1; then
    if ! pgrep -x "ollama" > /dev/null 2>&1; then
        echo -e "   +- Starting Ollama daemon in background..."
        ollama serve > /tmp/ollama.log 2>&1 &
        sleep 1
    else
        echo -e "   +- Ollama daemon is already running."
    fi
else
    echo -e "   +- ${YELLOW}Ollama CLI not found in PATH. (Optional: install via 'brew install ollama')${RESET}"
fi

# 2.5. Check & Start Kokoro Neural TTS Server
echo -e "\n${BOLD}2.5. Checking Neural Voice Service (Kokoro TTS)...${RESET}"
if [ -d ".kokoro-venv" ]; then
    if ! curl -s http://127.0.0.1:8421/health > /dev/null 2>&1 && ! pgrep -f "kokoro_tts_server.py" > /dev/null 2>&1; then
        echo -e "   +- Launching Kokoro Neural TTS Server on port 8421..."
        source .kokoro-venv/bin/activate
        python3 kokoro_tts_server.py > /tmp/kokoro.log 2>&1 &
        sleep 2
        echo -e "   +- ${GREEN}[ONLINE] Kokoro Neural TTS is LIVE at http://127.0.0.1:8421${RESET}"
    else
        echo -e "   +- ${GREEN}[ONLINE] Kokoro Neural TTS is already running on port 8421.${RESET}"
    fi
fi

# 3. Build & Start MoFA Engine Core
echo -e "\n${BOLD}3. Starting MoFA Engine Core Daemon (Port 8420)...${RESET}"
if ! curl -s http://127.0.0.1:8420/health > /dev/null 2>&1; then
    echo -e "   +- Compiling & starting 'target/debug/mofa-engine'..."
    mkdir -p output
    cargo run -p mofa-engine -- --config mofa_hybrid.toml > output/mofa-engine.log 2>&1 &
    
    # Wait for health
    for i in {1..20}; do
        if curl -s http://127.0.0.1:8420/health > /dev/null 2>&1; then
            break
        fi
        sleep 1
    done
    echo -e "   +- ${GREEN}[ONLINE] Engine Core is LIVE at http://127.0.0.1:8420${RESET}"
else
    echo -e "   +- ${GREEN}[ONLINE] Engine Core is already running on http://127.0.0.1:8420${RESET}"
fi

# 4. Start Web Studio Frontend
if [ -d "mofa-frontend" ] && command -v npm >/dev/null 2>&1; then
    echo -e "\n${BOLD}4. Starting Web Studio Frontend (Port 3000)...${RESET}"
    if ! curl -s http://localhost:3000 > /dev/null 2>&1; then
        cd mofa-frontend
        if [ ! -d "node_modules" ]; then
            echo -e "   +- Installing frontend dependencies..."
            npm install --silent > /dev/null 2>&1 || true
        fi
        echo -e "   +- Launching Vite development server..."
        npx vite --port 3000 --host 0.0.0.0 > /tmp/mofa-frontend.log 2>&1 &
        cd ..
        for i in {1..10}; do
            if curl -s http://localhost:3000 > /dev/null 2>&1; then
                break
            fi
            sleep 1
        done
        echo -e "   +- ${GREEN}[ONLINE] Web Studio is LIVE at http://localhost:3000${RESET}"
    else
        echo -e "   +- ${GREEN}[ONLINE] Web Studio is already running at http://localhost:3000${RESET}"
    fi
fi

echo -e "\n${BOLD}${GREEN}==================================================================${RESET}"
echo -e "${BOLD}${GREEN}MoFA Engine Full Stack is Ready & Running!${RESET}"
echo -e "${BOLD}${GREEN}==================================================================${RESET}"
echo -e "\n${BOLD}${CYAN}1. 🌐 Web Studio Frontend:${RESET}        ${BOLD}http://localhost:3000${RESET}"
echo -e "   +- Interactive Scenarios : S4 Video, S6 Podcast, S2 Code Review, S1 Meetings"
echo -e "   +- Embedded Media Players: Video Player, Audio Player, Collapsible Thought Chain"
echo -e "\n${BOLD}${CYAN}2. 📊 Dual-Track Observability UI:${RESET} ${BOLD}http://localhost:3000${RESET} (Click 'Observability' in top bar)"
echo -e "   +- Local vs Cloud Spend  : Real-time GPU cost (\$0.00) vs Cloud Token USD"
echo -e "   +- Model Efficiency Table: TTFT latency, tokens/sec, and preflight warmup savings"
echo -e "\n${BOLD}${CYAN}3. 📈 Production Monitoring:${RESET}"
echo -e "   +- Grafana Dashboards    : ${BOLD}http://localhost:3001${RESET} (login: admin / admin)"
echo -e "   +- Prometheus Console    : ${BOLD}http://localhost:9091${RESET}"
echo -e "   +- Engine OpenMetrics    : ${BOLD}http://127.0.0.1:8420/metrics${RESET}"
echo -e "   +- Engine API Gateway    : ${BOLD}http://127.0.0.1:8420${RESET}"
echo -e "\n${YELLOW}Useful Quick Commands:${RESET}"
echo -e "  * Status check            : ${BOLD}bash quickstart.sh status${RESET}"
echo -e "  * Diagnostic doctor       : ${BOLD}bash quickstart.sh doctor${RESET}"
echo -e "  * 30-second instant demo  : ${BOLD}bash quickstart.sh demo${RESET}"
echo -e "  * Provider race benchmark : ${BOLD}bash quickstart.sh benchmark${RESET}"
echo -e "  * Stop all services       : ${BOLD}bash quickstart.sh stop${RESET}\n"
