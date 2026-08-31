#!/usr/bin/env bash
# ==============================================================================
# MoFA Engine — 5-Minute Quickstart Launcher
# ==============================================================================
# Starts all components (Ollama, Engine Daemon, Web Studio Frontend)
# or runs automated scenario verification, diagnostics, and benchmarks.
#
# Usage:
#   bash quickstart.sh            # Start backend daemon + frontend studio
#   bash quickstart.sh setup      # Install prerequisites automatically
#   bash quickstart.sh doctor     # Diagnose system dependencies & providers
#   bash quickstart.sh status     # Check health of running services
#   bash quickstart.sh demo       # Run 30-second multimodal golden path demo
#   bash quickstart.sh benchmark  # Run multi-provider race & latency benchmark
#   bash quickstart.sh s1..s7     # Run specific scenario (e.g. s1, s2, s3, s6)
#   bash quickstart.sh logs       # Stream real-time engine and service logs
#   bash quickstart.sh stop       # Stop all running MoFA services
# ==============================================================================

set -e

GREEN="[32m"
BLUE="[34m"
YELLOW="[33m"
CYAN="[36m"
RED="[31m"
BOLD="[1m"
RESET="[0m"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

# Auto-load .env file if present
if [ -f ".env" ]; then
    set -a
    source .env 2>/dev/null || true
    set +a
fi

echo -e "${BOLD}${BLUE}==================================================================${RESET}"
echo -e "${BOLD}${CYAN}   MoFA Engine — Multimodal Orchestration for Artifacts${RESET}"
echo -e "${BOLD}${BLUE}==================================================================${RESET}\n"

# Mode: Cloud API Keys & Hybrid Configurator
if [ "$1" = "--keys" ] || [ "$1" = "keys" ] || [ "$1" = "config" ] || [ "$1" = "--config" ] || [ "$1" = "key" ]; then
    echo -e "${BOLD}${CYAN}MoFA Engine — Cloud API Keys & Hybrid Configurator${RESET}\n"
    echo -e "Configure your API keys for Cloud acceleration & Dual-Track spend tracking."
    echo -e "Keys will be saved securely to your local '.env' file.\n"

    echo -e "${CYAN}1. Google Gemini AI (Free tier available: https://aistudio.google.com/app/apikey)${RESET}"
    read -p "   GEMINI_API_KEY [${GEMINI_API_KEY:-Unset}]: " input_gemini
    [ -n "$input_gemini" ] && GEMINI_API_KEY="$input_gemini"

    echo -e "\n${CYAN}2. OpenAI Platform (GPT-4o & Whisper: https://platform.openai.com/api-keys)${RESET}"
    read -p "   OPENAI_API_KEY [${OPENAI_API_KEY:-Unset}]: " input_openai
    [ -n "$input_openai" ] && OPENAI_API_KEY="$input_openai"

    echo -e "\n${CYAN}3. DeepSeek AI (DeepSeek-R1: https://platform.deepseek.com/api_keys)${RESET}"
    read -p "   DEEPSEEK_API_KEY [${DEEPSEEK_API_KEY:-Unset}]: " input_deepseek
    [ -n "$input_deepseek" ] && DEEPSEEK_API_KEY="$input_deepseek"

    echo -e "\n${CYAN}4. Fireworks AI (Serverless models: https://fireworks.ai/api-keys)${RESET}"
    read -p "   FIREWORKS_API_KEY [${FIREWORKS_API_KEY:-Unset}]: " input_fireworks
    [ -n "$input_fireworks" ] && FIREWORKS_API_KEY="$input_fireworks"

    cat << EOF > .env
# MoFA Engine Environment Configuration
GEMINI_API_KEY="${GEMINI_API_KEY}"
OPENAI_API_KEY="${OPENAI_API_KEY}"
DEEPSEEK_API_KEY="${DEEPSEEK_API_KEY}"
FIREWORKS_API_KEY="${FIREWORKS_API_KEY}"
EOF

    echo -e "\n${BOLD}${GREEN}[OK] API Keys successfully saved to .env!${RESET}"
    echo -e "Run 'bash quickstart.sh doctor' to verify provider connections.\n"
    exit 0
fi

# Mode: Doctor Diagnostic
if [ "$1" = "--doctor" ] || [ "$1" = "doctor" ] || [ "$1" = "-doc" ] || [ "$1" = "doc" ]; then
    python3 mofa-fm/mofa_doctor.py
    exit 0
fi

# Mode: Setup / Automatic Installation
if [ "$1" = "--setup" ] || [ "$1" = "setup" ] || [ "$1" = "install" ] || [ "$1" = "--install" ]; then
    echo -e "${BOLD}${CYAN}MoFA Engine — Automated Dependency Installer${RESET}
"
    
    # 1. Rust & Cargo
    if ! command -v cargo >/dev/null 2>&1; then
        echo -e "${YELLOW}Installing Rust & Cargo toolchain via rustup...${RESET}"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env" 2>/dev/null || export PATH="$HOME/.cargo/bin:$PATH"
    else
        echo -e "  ${GREEN}[OK]${RESET} Rust & Cargo already installed ($(cargo --version | cut -d' ' -f2))."
    fi

    # 2. Package Managers (macOS Homebrew / Linux apt)
    if [[ "$(uname)" == "Darwin" ]]; then
        if command -v brew >/dev/null 2>&1; then
            if ! command -v npm >/dev/null 2>&1; then
                echo -e "${YELLOW}Installing Node.js via Homebrew...${RESET}"
                brew install node
            fi
            if ! command -v ollama >/dev/null 2>&1; then
                echo -e "${YELLOW}Installing Ollama via Homebrew...${RESET}"
                brew install ollama
            fi
            if ! command -v ffmpeg >/dev/null 2>&1; then
                echo -e "${YELLOW}Installing FFmpeg via Homebrew...${RESET}"
                brew install ffmpeg
            fi
        fi
    elif [[ "$(uname)" == "Linux" ]]; then
        if command -v apt-get >/dev/null 2>&1; then
            echo -e "${YELLOW}Updating Debian/Ubuntu packages...${RESET}"
            sudo apt-get update -qq && sudo apt-get install -y -qq build-essential nodejs npm ffmpeg curl || true
        fi
    fi

    # 3. Kokoro TTS environment
    if [ ! -d ".kokoro-venv" ]; then
        echo -e "${YELLOW}Configuring Kokoro Neural TTS virtualenv...${RESET}"
        python3 -m venv .kokoro-venv
        source .kokoro-venv/bin/activate
        pip install --upgrade pip > /dev/null 2>&1
        pip install kokoro-onnx soundfile > /dev/null 2>&1 || true
        deactivate
        echo -e "  ${GREEN}[OK]${RESET} Kokoro Neural TTS environment configured."
    fi

    # 4. Check & Pull Starter Local Model (if none installed)
    if command -v ollama >/dev/null 2>&1; then
        echo -e "\n${BOLD}4. Checking Local LLM Models in Ollama...${RESET}"
        if ! curl -s http://127.0.0.1:11434/api/tags > /dev/null 2>&1 && ! pgrep -x "ollama" > /dev/null 2>&1; then
            echo -e "   +- Starting Ollama daemon..."
            ollama serve > /tmp/ollama.log 2>&1 &
            sleep 2
        fi
        
        model_count=$(ollama list 2>/dev/null | tail -n +2 | grep -v '^[[:space:]]*$' | wc -l | tr -d ' ')
        if [ "$model_count" -eq 0 ]; then
            echo -e "   +- ${YELLOW}[INFO] No local LLM found.${RESET} Pulling lightweight starter model (${BOLD}qwen2.5:1.5b${RESET} ~980MB)..."
            if ollama pull qwen2.5:1.5b 2>/dev/null; then
                echo -e "   +- ${GREEN}[OK]${RESET} Starter model 'qwen2.5:1.5b' installed for zero-cost offline chat."
            else
                echo -e "   +- ${YELLOW}[WARN]${RESET} Could not auto-download model. (Run 'ollama pull qwen2.5:1.5b' or use Cloud keys)"
            fi
        else
            echo -e "   +- ${GREEN}[OK]${RESET} Found $model_count local Ollama model(s) installed."
        fi
    fi

    echo -e "\n${BOLD}${GREEN}Setup complete! Run 'bash quickstart.sh' to launch MoFA Engine.${RESET}\n"
    exit 0
fi

# Mode: Scenario Direct Launchers (S1 - S7)
if [ "$1" = "s1" ] || [ "$1" = "meeting" ]; then
    echo -e "${CYAN}Running Scenario S1: Meeting Audio to Brief Pipeline...${RESET}
"
    python3 examples/meeting_brief.py "${@:2}"
    exit 0
fi
if [ "$1" = "s2" ] || [ "$1" = "review" ]; then
    echo -e "${CYAN}Running Scenario S2: Code Review Reasoning Agent...${RESET}
"
    python3 examples/code_review.py "${@:2}"
    exit 0
fi
if [ "$1" = "s3" ] || [ "$1" = "doc" ] || [ "$1" = "docai" ]; then
    echo -e "${CYAN}Running Scenario S3: Document AI Receipt Extraction...${RESET}
"
    python3 examples/doc_ai.py "${@:2}"
    exit 0
fi
if [ "$1" = "s4" ] || [ "$1" = "video" ]; then
    echo -e "${CYAN}Running Scenario S4: Explainer Video Composition...${RESET}
"
    python3 examples/explainer_video.py "${@:2}"
    exit 0
fi
if [ "$1" = "s6" ] || [ "$1" = "podcast" ]; then
    echo -e "${CYAN}Running Scenario S6: Article to Podcast Studio...${RESET}
"
    python3 mofa-fm/article_to_podcast.py "${@:2}"
    exit 0
fi
if [ "$1" = "s7" ] || [ "$1" = "race" ]; then
    echo -e "${CYAN}Running Scenario S7: Provider Race Latency Benchmark...${RESET}
"
    python3 examples/01_provider_race.py "${@:2}"
    exit 0
fi

# Mode: Logs Streamer
if [ "$1" = "logs" ] || [ "$1" = "log" ] || [ "$1" = "-l" ]; then
    echo -e "${CYAN}Streaming MoFA Engine Daemon Logs (output/mofa-engine.log)...${RESET}"
    echo -e "${YELLOW}(Press Ctrl+C to exit)${RESET}
"
    if [ -f "output/mofa-engine.log" ]; then
        tail -n 50 -f output/mofa-engine.log
    else
        echo -e "${YELLOW}Log file output/mofa-engine.log does not exist yet. Start the engine first.${RESET}"
    fi
    exit 0
fi

# Mode: Golden Path Demo Runner
if [ "$1" = "--demo" ] || [ "$1" = "demo" ] || [ "$1" = "-d" ]; then
    echo -e "${CYAN}Launching MoFA 30-Second Multimodal Golden Path Demo...${RESET}
"
    python3 examples/quickstart_demo.py
    exit 0
fi

# Mode: Benchmark Runner
if [ "$1" = "--benchmark" ] || [ "$1" = "benchmark" ] || [ "$1" = "-b" ] || [ "$1" = "bench" ]; then
    echo -e "${YELLOW}Running Real-Time Multi-Provider Race & Warmup Benchmark...${RESET}
"
    python3 examples/01_provider_race.py
    exit 0
fi

# Mode: Test Runner
if [ "$1" = "--test" ] || [ "$1" = "test" ] || [ "$1" = "-t" ]; then
    echo -e "${YELLOW}Running MoFA End-to-End Scenario Integration Tests...${RESET}
"
    python3 -m unittest tests/integration/test_e2e_scenarios.py
    exit 0
fi

# Mode: Stop / Kill Services
if [ "$1" = "--stop" ] || [ "$1" = "stop" ] || [ "$1" = "-k" ] || [ "$1" = "kill" ]; then
    echo -e "${YELLOW}Stopping MoFA services...${RESET}"
    pkill -f "target.*mofa-engine" > /dev/null 2>&1 && echo -e "  +- MoFA Engine Core stopped" || true
    pkill -f "vite" > /dev/null 2>&1 && echo -e "  +- Web Studio Frontend stopped" || true
    pkill -f "kokoro_tts_server.py" > /dev/null 2>&1 && echo -e "  +- Kokoro TTS Server stopped" || true
    echo -e "${GREEN}All MoFA services stopped.${RESET}
"
    exit 0
fi

# Mode: Help
if [ "$1" = "--help" ] || [ "$1" = "help" ] || [ "$1" = "-h" ]; then
    echo -e "Usage: bash quickstart.sh [COMMAND]
"
    echo -e "Core Commands:"
    echo -e "  (no args)     Start full stack (Engine Core, Kokoro TTS, Web Studio UI)"
    echo -e "  setup         Automatically install prerequisites (Rust, Kokoro, Node)"
    echo -e "  keys          Configure Cloud API keys (Gemini, OpenAI, DeepSeek)"
    echo -e "  doctor        Diagnose system dependencies, provider gateways & scenarios"
    echo -e "  status        Check health and port status of all running services"
    echo -e "  demo          Run the 30-second multimodal golden path demo suite"
    echo -e "  benchmark     Run real-time multi-provider race & latency benchmark"
    echo -e "  test          Run end-to-end scenario integration test suite"
    echo -e "  logs          Stream live engine gateway logs"
    echo -e "  stop          Stop all running MoFA services (Engine, Frontend, Kokoro)"
    echo -e "Scenario Shortcuts:"
    echo -e "  s1            Run Meeting Audio Brief Pipeline"
    echo -e "  s2            Run Code Review Reasoning Agent"
    echo -e "  s3            Run Document AI Receipt Extractor"
    echo -e "  s4            Run Explainer Video Composition"
    echo -e "  s6            Run Podcast Studio (Article -> Multi-Voice Audio)"
    echo -e "  s7            Run Provider Race Benchmark
"
    exit 0
fi

# Mode: Status Check
if [ "$1" = "--status" ] || [ "$1" = "status" ] || [ "$1" = "-s" ] || [ "$1" = "ps" ]; then
    echo -e "${CYAN}Checking MoFA stack status...${RESET}
"
    
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

# 1. Prerequisite Checks & Hardware Inspection
echo -e "${BOLD}1. Checking Prerequisites & Hardware Profile...${RESET}"

# Hardware Memory Detection
if [[ "$(uname)" == "Darwin" ]]; then
    mem_bytes=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
    mem_gb=$((mem_bytes / 1024 / 1024 / 1024))
    arch=$(uname -m)
    echo -e "   +- Hardware     : ${GREEN}[OK]${RESET} ${arch} (${mem_gb} GB RAM)"
fi

if command -v cargo >/dev/null 2>&1; then
    echo -e "   +- Rust & Cargo : ${GREEN}[OK]${RESET} Found ($(cargo --version | cut -d' ' -f2))"
else
    echo -e "   +- Rust & Cargo : ${YELLOW}[WARN]${RESET} Not found. (Run 'bash quickstart.sh setup' to install)"
fi

if command -v python3 >/dev/null 2>&1; then
    echo -e "   +- Python 3     : ${GREEN}[OK]${RESET} Found ($(python3 --version))"
else
    echo -e "   +- Python 3     : ${RED}[ERROR]${RESET} Python 3 not found"
fi

if command -v npm >/dev/null 2>&1; then
    echo -e "   +- Node / npm   : ${GREEN}[OK]${RESET} Found ($(node --version))"
else
    echo -e "   +- Node / npm   : ${YELLOW}[WARN]${RESET} Node.js / npm not found (needed for Web Studio)"
fi

if command -v ffmpeg >/dev/null 2>&1; then
    echo -e "   +- FFmpeg Media : ${GREEN}[OK]${RESET} Found"
else
    echo -e "   +- FFmpeg Media : ${YELLOW}[WARN]${RESET} Optional (install via 'brew install ffmpeg' for video rendering)"
fi

# 1.5. Check & Start Observability Stack (Prometheus + Grafana in Docker)
echo -e "\n${BOLD}1.5. Checking Observability Stack (Docker Prometheus & Grafana)...${RESET}"
if command -v docker >/dev/null 2>&1; then
    if [ -f "mofa-observability/docker/docker-compose.yml" ]; then
        if ! curl -s http://localhost:3001 > /dev/null 2>&1 && ! curl -s http://localhost:9091 > /dev/null 2>&1; then
            echo -e "   +- Launching Prometheus (:9091) & Grafana (:3001)..."
            (cd mofa-observability/docker && docker compose up -d > /dev/null 2>&1 || true)
            sleep 2
        fi
        
        if curl -s http://localhost:3001 > /dev/null 2>&1 || curl -s http://localhost:9091 > /dev/null 2>&1; then
            echo -e "   +- ${GREEN}[ONLINE]${RESET} Prometheus (:9091) & Grafana (:3001) are LIVE"
        else
            echo -e "   +- ${YELLOW}[OFFLINE]${RESET} Docker Observability Stack is stopped. (Optional)"
        fi
    fi
else
    echo -e "   +- ${YELLOW}[INFO]${RESET} Docker not found in PATH. Skipping Prometheus & Grafana container stack."
fi

# 2. Check & Start Ollama if present
echo -e "\n${BOLD}2. Checking Local Model Service (Ollama)...${RESET}"
if command -v ollama >/dev/null 2>&1; then
    if ! curl -s http://127.0.0.1:11434/api/tags > /dev/null 2>&1; then
        echo -e "   +- Starting Ollama daemon in background..."
        ollama serve > /tmp/ollama.log 2>&1 &
        sleep 2
    fi

    if curl -s http://127.0.0.1:11434/api/tags > /dev/null 2>&1; then
        echo -e "   +- ${GREEN}[ONLINE]${RESET} Ollama LLM Service is LIVE on port 11434."
    else
        echo -e "   +- ${YELLOW}[OFFLINE]${RESET} Ollama service is stopped. (Run 'ollama serve' or use Cloud keys)"
    fi
else
    echo -e "   +- ${YELLOW}[INFO]${RESET} Ollama CLI not found in PATH. (Optional: install via 'brew install ollama')"
fi

# 2.5. Check & Start Kokoro Neural TTS Server
echo -e "\n${BOLD}2.5. Checking Neural Voice Service (Kokoro TTS)...${RESET}"
if [ -d ".kokoro-venv" ]; then
    if ! curl -s http://127.0.0.1:8421/health > /dev/null 2>&1 && ! pgrep -f "kokoro_tts_server.py" > /dev/null 2>&1; then
        echo -e "   +- Launching Kokoro Neural TTS Server on port 8421..."
        source .kokoro-venv/bin/activate
        python3 kokoro_tts_server.py > /tmp/kokoro.log 2>&1 &
        sleep 2
    fi

    if curl -s http://127.0.0.1:8421/health > /dev/null 2>&1; then
        echo -e "   +- ${GREEN}[ONLINE]${RESET} Kokoro Neural TTS is LIVE at http://127.0.0.1:8421"
    else
        echo -e "   +- ${YELLOW}[OFFLINE]${RESET} Kokoro Neural TTS is offline. (Zero-config macOS 'say' / Gemini TTS will be used)"
    fi
else
    echo -e "   +- ${YELLOW}[INFO]${RESET} Kokoro venv not set up. (Zero-config macOS 'say' / Gemini TTS fallback will be used)"
    echo -e "      (To install local Kokoro neural voice: bash quickstart.sh setup)"
fi

# 3. Build & Start MoFA Engine Core
echo -e "
${BOLD}3. Starting MoFA Engine Core Daemon (Port 8420)...${RESET}"
engine_started=false
if ! curl -s http://127.0.0.1:8420/health > /dev/null 2>&1; then
    mkdir -p output
    lsof -ti :8420 | xargs kill -9 2>/dev/null || true
    if command -v cargo >/dev/null 2>&1; then
        echo -e "   +- Compiling & starting via cargo..."
        cargo run -p mofa-engine -- --config mofa_hybrid.toml > output/mofa-engine.log 2>&1 &
    elif [ -f "target/debug/mofa-engine" ]; then
        echo -e "   +- Launching precompiled 'target/debug/mofa-engine'..."
        target/debug/mofa-engine -c mofa_hybrid.toml > output/mofa-engine.log 2>&1 &
    elif [ -f "target/release/mofa-engine" ]; then
        echo -e "   +- Launching precompiled 'target/release/mofa-engine'..."
        target/release/mofa-engine -c mofa_hybrid.toml > output/mofa-engine.log 2>&1 &
    else
        echo -e "   +- ${RED}[ERROR]${RESET} Rust & Cargo not found and no precompiled binary exists in target/."
        echo -e "      Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    fi
    
    # Wait for health
    for i in {1..20}; do
        if curl -s http://127.0.0.1:8420/health > /dev/null 2>&1; then
            engine_started=true
            break
        fi
        sleep 1
    done

    if [ "$engine_started" = true ]; then
        echo -e "   +- ${GREEN}[ONLINE]${RESET} Engine Core is LIVE at http://127.0.0.1:8420"
    else
        echo -e "   +- ${RED}[OFFLINE]${RESET} Engine Core failed to start. (Check output/mofa-engine.log)"
    fi
else
    engine_started=true
    echo -e "   +- ${GREEN}[ONLINE]${RESET} Engine Core is already running on http://127.0.0.1:8420"
fi

# 4. Start Web Studio Frontend
if [ -d "mofa-frontend" ] && command -v npm >/dev/null 2>&1; then
    echo -e "
${BOLD}4. Starting Web Studio Frontend (Port 3000)...${RESET}"
    if ! curl -s http://localhost:3000 > /dev/null 2>&1; then
        cd mofa-frontend
        if [ ! -d "node_modules" ]; then
            echo -e "   +- Installing frontend dependencies..."
            npm install --silent > /dev/null 2>&1 || true
        fi
        echo -e "   +- Launching Vite development server..."
        npx vite --port 3000 --host 0.0.0.0 > /tmp/mofa-frontend.log 2>&1 &
        cd ..
        frontend_started=false
        for i in {1..10}; do
            if curl -s http://localhost:3000 > /dev/null 2>&1; then
                frontend_started=true
                break
            fi
            sleep 1
        done

        if [ "$frontend_started" = true ]; then
            echo -e "   +- ${GREEN}[ONLINE]${RESET} Web Studio is LIVE at http://localhost:3000"
        else
            echo -e "   +- ${YELLOW}[WARN]${RESET} Web Studio took longer to start. (Check /tmp/mofa-frontend.log)"
        fi
    else
        echo -e "   +- ${GREEN}[ONLINE]${RESET} Web Studio is already running at http://localhost:3000"
    fi
fi

if [ "$engine_started" = true ]; then
    echo -e "
${BOLD}${GREEN}==================================================================${RESET}"
    echo -e "${BOLD}${GREEN}MoFA Engine Full Stack is Ready & Running!${RESET}"
    echo -e "${BOLD}${GREEN}==================================================================${RESET}"
    echo -e "
${BOLD}${CYAN}[UI] Web Studio Frontend:${RESET}        ${BOLD}http://localhost:3000${RESET}"
    echo -e "   +- Interactive Scenarios : S4 Video, S6 Podcast, S2 Code Review, S1 Meetings"
    echo -e "   +- Embedded Media Players: Video Player, Audio Player, Collapsible Thought Chain"
    echo -e "
${BOLD}${CYAN}[METRICS] Dual-Track Observability:${RESET} ${BOLD}http://localhost:3000${RESET} (Click 'Observability')"
    echo -e "   +- Local vs Cloud Spend  : Real-time GPU cost (\$0.00 USD) vs Cloud Token USD"
    echo -e "   +- Model Efficiency Table: TTFT latency, tokens/sec, and preflight warmup savings"
    echo -e "
${BOLD}${CYAN}[MONITORING] Production Monitoring:${RESET}"
    echo -e "   +- Grafana Dashboards    : ${BOLD}http://localhost:3001${RESET} (login: admin / admin)"
    echo -e "   +- Prometheus Console    : ${BOLD}http://localhost:9091${RESET}"
    echo -e "   +- Engine OpenMetrics    : ${BOLD}http://127.0.0.1:8420/metrics${RESET}"
    echo -e "   +- Engine API Gateway    : ${BOLD}http://127.0.0.1:8420${RESET}"
else
    echo -e "
${BOLD}${YELLOW}==================================================================${RESET}"
    echo -e "${BOLD}${YELLOW}MoFA Engine Setup Required (Backend Offline)${RESET}"
    echo -e "${BOLD}${YELLOW}==================================================================${RESET}"
    echo -e "  To start the backend daemon, install Rust and run setup:
"
    echo -e "  1. Install Rust Toolchain : ${BOLD}curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh${RESET}"
    echo -e "  2. Run Auto-Installer     : ${BOLD}bash quickstart.sh setup${RESET}"
    echo -e "  3. Launch MoFA Stack      : ${BOLD}bash quickstart.sh${RESET}
"
fi

echo -e "\n${YELLOW}Useful Quick Commands:${RESET}"
echo -e "  * Auto setup & install    : ${BOLD}bash quickstart.sh setup${RESET}"
echo -e "  * Configure Cloud keys    : ${BOLD}bash quickstart.sh keys${RESET}"
echo -e "  * Status check            : ${BOLD}bash quickstart.sh status${RESET}"
echo -e "  * Diagnostic doctor       : ${BOLD}bash quickstart.sh doctor${RESET}"
echo -e "  * 30-second instant demo  : ${BOLD}bash quickstart.sh demo${RESET}"
echo -e "  * Stream engine logs      : ${BOLD}bash quickstart.sh logs${RESET}"
echo -e "  * Direct scenario launch  : ${BOLD}bash quickstart.sh s1${RESET} (or s2, s3, s4, s6, s7)"
echo -e "  * Stop all services       : ${BOLD}bash quickstart.sh stop${RESET}\n"
