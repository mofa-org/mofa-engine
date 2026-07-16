# MoFA FM

MoFA FM is a local AI podcast studio. It turns any article into a high-quality podcast script and synthesizes it into natural-sounding audio — running entirely on your machine. No cloud, no API keys, full privacy.

## Features

- **Offline-First**: Operates seamlessly against a local inference engine.
- **Visual Intelligence**: Real-time observability into the model's inner workings via the Theater view and Engine Monitor.
- **Design Excellence**: Highly polished interface with considered motion, sound design, and typography.
- **Robust Pipeline**: Hardened against intermittent network failures with intelligent retries.
- **Accessibility**: Full keyboard and screen reader support, honoring reduced-motion preferences.

## Quick Start

\`\`\`bash
npm install
npm run dev
\`\`\`

## Architecture

- **React + Vite**: Fast, modern frontend build.
- **Tailwind CSS**: Utility-first styling tied to a strict design token system.
- **Framer Motion**: Smooth, intentional animations and state transitions.

### Mock vs. Real Engine

MoFA FM is designed to run against the **MoFA Core Engine** (at `127.0.0.1:8420`). 
However, for development and demonstration without the heavy backend, the app can run against a simulated local mock engine.

To use the mock engine, create a `.env` file or set the environment variable:
\`\`\`env
VITE_USE_MOCK=true
VITE_ENGINE_URL=http://localhost:8420
\`\`\`

If `VITE_USE_MOCK=false`, the application will make real HTTP and SSE connections to your local Engine.
