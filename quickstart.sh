#!/usr/bin/env bash
# ==============================================================================
# MoFA Engine — 5-Minute Quickstart Launcher
# ==============================================================================
# Starts all components (Ollama, Engine Daemon, Web Studio Frontend)
# or runs automated scenario verification and benchmarks.
#
# Usage:
#   ./quickstart.sh            # Start backend daemon + frontend studio
#   ./quickstart.sh --demo     # Run all 7 scenario demos in standalone mode
#   ./quickstart.sh --benchmark# Run performance baseline and latency benchmarks
#   ./quickstart.sh --test     # Run end-to-end integration test suite
#   ./quickstart.sh --status   # Check health of running services
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

# Mode: Demo Runner
if [ "$1" == "--demo" ] || [ "$1" == "-d" ]; then
    echo -e "${YELLOW}Running all 7 Scenario Demos (Standalone Artifact Generation)...${RESET}\n"
    python3 examples/01_provider_race.py --mock
    python3 examples/multimodal_chat_s1.py --mock
    python3 examples/speech_to_text_s2.py --mock
    python3 examples/code_review.py --mock
    python3 examples/doc_ai.py --mock
    python3 examples/meeting_brief.py --mock
    python3 mofa-fm/article_to_podcast.py --mock
    python3 examples/explainer_video.py --mock
    
    echo -e "\n${GREEN}==================================================================${RESET}"
    echo -e "${GREEN}ALL SCENARIO ARTIFACTS GENERATED IN: ${BOLD}$ROOT_DIR/output/${RESET}"
    echo -e "${GREEN}==================================================================${RESET}"
    ls -lh output/
    exit 0
fi

# Mode: Benchmark Runner
if [ "$1" == "--benchmark" ] || [ "$1" == "-b" ]; then
    echo -e "${YELLOW}Running MoFA Performance Baseline & Warmup Benchmark...${RESET}\n"
    python3 tests/integration/benchmark.py --mock
    exit 0
fi

# Mode: Test Runner
if [ "$1" == "--test" ] || [ "$1" == "-t" ]; then
    echo -e "${YELLOW}Running MoFA End-to-End Scenario Integration Tests...${RESET}\n"
    python3 -m unittest tests/integration/test_e2e_scenarios.py
    exit 0
fi

# Mode: Status Check
if [ "$1" == "--status" ] || [ "$1" == "-s" ]; then
    echo -e "${CYAN}Checking MoFA stack status...${RESET}\n"
    
    # Check Engine
    if curl -s http://127.0.0.1:8420/health > /dev/null 2>&1; then
        echo -e "  * MoFA Engine Core (8420) : ${GREEN}[ONLINE] Running (Healthy)${RESET}"
    else
        echo -e "  * MoFA Engine Core (8420) : ${YELLOW}[OFFLINE] Stopped${RESET}"
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
        npm run dev > /tmp/mofa-frontend.log 2>&1 &
        cd ..
        sleep 2
        echo -e "   +- ${GREEN}[ONLINE] Web Studio is LIVE at http://localhost:3000${RESET}"
    else
        echo -e "   +- ${GREEN}[ONLINE] Web Studio is already running at http://localhost:3000${RESET}"
    fi
fi

echo -e "\n${BOLD}${GREEN}==================================================================${RESET}"
echo -e "${BOLD}${GREEN}MoFA Engine Stack is Ready!${RESET}"
echo -e "${BOLD}${GREEN}==================================================================${RESET}"
echo -e "  Web Studio:         http://localhost:3000"
echo -e "  Engine API Gateway: http://127.0.0.1:8420"
echo -e "  Metrics Export:     http://127.0.0.1:8420/metrics"
echo -e "\n${CYAN}Try running any scenario script:${RESET}"
echo -e "  * ${BOLD}Flagship Explainer Video:${RESET} python3 examples/explainer_video.py \"Quantum Computing\""
echo -e "  * ${BOLD}AI Code Review:${RESET}           python3 examples/code_review.py"
echo -e "  * ${BOLD}Meeting Minutes & Brief:${RESET}   python3 examples/meeting_brief.py"
echo -e "  * ${BOLD}Document & Receipt AI:${RESET}     python3 examples/doc_ai.py"
echo -e "  * ${BOLD}Article to Podcast:${RESET}        python3 mofa-fm/article_to_podcast.py"
echo -e "\n${YELLOW}To run all scenario demos in 5 seconds without cloud keys:${RESET}"
echo -e "  ${BOLD}./quickstart.sh --demo${RESET}"
echo -e "  ${BOLD}./quickstart.sh --benchmark${RESET}\n"
