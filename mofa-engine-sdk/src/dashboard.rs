//! Embedded single-page dashboard.
//!
//! All HTML, CSS, and JS is compiled into the binary as a const string.
//! No external dependencies — vanilla JS + CSS with glassmorphism design.

/// The complete dashboard HTML page.
pub const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>MoFA Engine</title>
<style>
:root {
  --bg-primary: #0a0e17;
  --bg-secondary: #111827;
  --bg-card: rgba(17, 24, 39, 0.7);
  --bg-glass: rgba(255, 255, 255, 0.03);
  --border-glass: rgba(255, 255, 255, 0.08);
  --text-primary: #f0f4f8;
  --text-secondary: #94a3b8;
  --text-dim: #64748b;
  --accent-blue: #3b82f6;
  --accent-cyan: #06b6d4;
  --accent-green: #10b981;
  --accent-yellow: #f59e0b;
  --accent-red: #ef4444;
  --accent-purple: #8b5cf6;
  --radius: 12px;
  --radius-sm: 8px;
  --shadow: 0 4px 24px rgba(0, 0, 0, 0.3);
  --transition: 0.2s ease;
}

* { margin: 0; padding: 0; box-sizing: border-box; }

body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', sans-serif;
  background: var(--bg-primary);
  color: var(--text-primary);
  min-height: 100vh;
  line-height: 1.6;
}

/* ── Header ───────────────────────────────────── */
.header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 24px;
  border-bottom: 1px solid var(--border-glass);
  background: var(--bg-glass);
  backdrop-filter: blur(16px);
  position: sticky;
  top: 0;
  z-index: 100;
}

.header h1 {
  font-size: 20px;
  font-weight: 700;
  background: linear-gradient(135deg, var(--accent-blue), var(--accent-cyan));
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.header-meta {
  display: flex;
  align-items: center;
  gap: 12px;
}

.badge {
  font-size: 11px;
  padding: 3px 10px;
  border-radius: 20px;
  font-weight: 600;
  letter-spacing: 0.5px;
  text-transform: uppercase;
}

.badge-version {
  background: rgba(59, 130, 246, 0.15);
  color: var(--accent-blue);
  border: 1px solid rgba(59, 130, 246, 0.3);
}

.badge-uptime {
  background: rgba(16, 185, 129, 0.15);
  color: var(--accent-green);
  border: 1px solid rgba(16, 185, 129, 0.3);
}

/* ── Stats Bar ────────────────────────────────── */
.stats-bar {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 16px;
  padding: 20px 24px;
}

.stat-card {
  background: var(--bg-card);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius);
  padding: 16px 20px;
  backdrop-filter: blur(12px);
  transition: border-color var(--transition);
}

.stat-card:hover {
  border-color: rgba(255, 255, 255, 0.15);
}

.stat-label {
  font-size: 12px;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 4px;
}

.stat-value {
  font-size: 28px;
  font-weight: 700;
}

.stat-value.blue { color: var(--accent-blue); }
.stat-value.green { color: var(--accent-green); }
.stat-value.purple { color: var(--accent-purple); }
.stat-value.cyan { color: var(--accent-cyan); }

/* ── Memory Bar ───────────────────────────────── */
.memory-bar-container {
  padding: 0 24px 16px;
}

.memory-bar-track {
  height: 6px;
  background: var(--bg-secondary);
  border-radius: 3px;
  overflow: hidden;
}

.memory-bar-fill {
  height: 100%;
  border-radius: 3px;
  transition: width 0.5s ease, background 0.5s ease;
  background: linear-gradient(90deg, var(--accent-green), var(--accent-cyan));
}

.memory-bar-fill.warn { background: linear-gradient(90deg, var(--accent-yellow), var(--accent-red)); }

.memory-bar-label {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  color: var(--text-dim);
  margin-top: 4px;
}

/* ── Main Area ────────────────────────────────── */
.main {
  display: grid;
  grid-template-columns: 1fr 340px;
  gap: 20px;
  padding: 0 24px 20px;
}

/* ── Section Panels ───────────────────────────── */
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius);
  backdrop-filter: blur(12px);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-glass);
  font-weight: 600;
  font-size: 14px;
}

.panel-body { padding: 14px 18px; }

/* ── Model Grid ───────────────────────────────── */
.model-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
  padding: 14px 18px;
}

.model-card {
  background: var(--bg-glass);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-sm);
  padding: 14px 16px;
  cursor: pointer;
  transition: all var(--transition);
}

.model-card:hover {
  border-color: rgba(59, 130, 246, 0.4);
  transform: translateY(-1px);
  box-shadow: 0 4px 16px rgba(59, 130, 246, 0.1);
}

.model-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}

.model-name {
  font-weight: 600;
  font-size: 14px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 70%;
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.status-dot.hot { background: var(--accent-green); box-shadow: 0 0 6px var(--accent-green); }
.status-dot.cold { background: var(--text-dim); opacity: 0.5; }
.status-dot.warming { background: var(--accent-yellow); animation: pulse 1.2s infinite; }
.status-dot.busy { background: var(--accent-blue); animation: pulse 0.8s infinite; }
.status-dot.failed { background: var(--accent-red); }

@keyframes pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.5; transform: scale(1.3); }
}

.model-meta {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
  margin-bottom: 6px;
}

.cap-badge {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.3px;
}

.cap-badge.chat { background: rgba(59, 130, 246, 0.15); color: var(--accent-blue); }
.cap-badge.tts { background: rgba(139, 92, 246, 0.15); color: var(--accent-purple); }
.cap-badge.asr { background: rgba(6, 182, 212, 0.15); color: var(--accent-cyan); }
.cap-badge.imagegen { background: rgba(245, 158, 11, 0.15); color: var(--accent-yellow); }
.cap-badge.embedding { background: rgba(16, 185, 129, 0.15); color: var(--accent-green); }

.model-provider {
  font-size: 11px;
  color: var(--text-dim);
}

/* ── Provider Panel ───────────────────────────── */
.provider-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
}

.provider-item:last-child { border-bottom: none; }

.provider-name {
  font-size: 13px;
  font-weight: 500;
}

.provider-status {
  display: flex;
  align-items: center;
  gap: 6px;
}

.circuit-label {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  text-transform: uppercase;
}

.circuit-label.closed { background: rgba(16, 185, 129, 0.15); color: var(--accent-green); }
.circuit-label.open { background: rgba(239, 68, 68, 0.15); color: var(--accent-red); }
.circuit-label.half_open { background: rgba(245, 158, 11, 0.15); color: var(--accent-yellow); }

/* ── Try It Panel ─────────────────────────────── */
.try-it {
  margin-top: 16px;
}

.try-it select, .try-it textarea, .try-it button {
  width: 100%;
  font-family: inherit;
  font-size: 13px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-glass);
  background: var(--bg-secondary);
  color: var(--text-primary);
  padding: 10px 12px;
  margin-bottom: 10px;
  outline: none;
  transition: border-color var(--transition);
}

.try-it select:focus, .try-it textarea:focus {
  border-color: var(--accent-blue);
}

.try-it textarea {
  min-height: 80px;
  resize: vertical;
}

.try-it button {
  background: linear-gradient(135deg, var(--accent-blue), var(--accent-cyan));
  border: none;
  font-weight: 600;
  cursor: pointer;
  transition: opacity var(--transition);
}

.try-it button:hover { opacity: 0.9; }
.try-it button:disabled { opacity: 0.4; cursor: not-allowed; }

.try-it-response {
  background: var(--bg-secondary);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-sm);
  padding: 12px;
  font-size: 13px;
  white-space: pre-wrap;
  max-height: 200px;
  overflow-y: auto;
  color: var(--text-secondary);
}

/* ── Request Log ──────────────────────────────── */
.log-area {
  padding: 0 24px 24px;
}

.log-list {
  max-height: 220px;
  overflow-y: auto;
}

.log-entry {
  display: grid;
  grid-template-columns: 90px 1fr 80px 60px;
  gap: 12px;
  padding: 8px 12px;
  font-size: 12px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  align-items: center;
}

.log-time { color: var(--text-dim); font-family: monospace; }
.log-model { color: var(--text-primary); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.log-duration { color: var(--accent-cyan); text-align: right; font-family: monospace; }
.log-status-ok { color: var(--accent-green); text-align: right; }
.log-status-err { color: var(--accent-red); text-align: right; }

/* ── Modal ────────────────────────────────────── */
.modal-overlay {
  display: none;
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  backdrop-filter: blur(4px);
  z-index: 200;
  justify-content: center;
  align-items: center;
}

.modal-overlay.active { display: flex; }

.modal {
  background: var(--bg-secondary);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius);
  padding: 24px;
  max-width: 500px;
  width: 90%;
  box-shadow: var(--shadow);
}

.modal h2 {
  font-size: 18px;
  margin-bottom: 16px;
  background: linear-gradient(135deg, var(--accent-blue), var(--accent-cyan));
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.modal-field {
  margin-bottom: 12px;
}

.modal-field label {
  display: block;
  font-size: 11px;
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 2px;
}

.modal-field .val {
  font-size: 14px;
  color: var(--text-primary);
}

.modal-close {
  margin-top: 16px;
  padding: 8px 20px;
  border: 1px solid var(--border-glass);
  background: transparent;
  color: var(--text-secondary);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
}

.modal-close:hover { border-color: var(--accent-blue); color: var(--text-primary); }

/* ── Footer ───────────────────────────────────── */
.footer {
  padding: 12px 24px;
  border-top: 1px solid var(--border-glass);
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  color: var(--text-dim);
}

.footer code {
  font-family: 'SF Mono', Monaco, Consolas, monospace;
  background: var(--bg-secondary);
  padding: 2px 8px;
  border-radius: 4px;
  color: var(--text-secondary);
}

/* ── Empty State ──────────────────────────────── */
.empty-state {
  text-align: center;
  padding: 40px 20px;
  color: var(--text-dim);
  font-size: 14px;
}

/* ── Scrollbar ────────────────────────────────── */
::-webkit-scrollbar { width: 6px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: rgba(255, 255, 255, 0.1); border-radius: 3px; }
::-webkit-scrollbar-thumb:hover { background: rgba(255, 255, 255, 0.2); }

/* ── Responsive ───────────────────────────────── */
@media (max-width: 900px) {
  .main { grid-template-columns: 1fr; }
  .stats-bar { grid-template-columns: repeat(2, 1fr); }
}

@media (max-width: 600px) {
  .stats-bar { grid-template-columns: 1fr; }
  .log-entry { grid-template-columns: 1fr 1fr; }
}
</style>
</head>
<body>

<!-- Header -->
<div class="header">
  <h1>MoFA Engine</h1>
  <div class="header-meta">
    <span class="badge badge-uptime" id="uptime-badge">--</span>
    <span class="badge badge-version" id="version-badge">--</span>
  </div>
</div>

<!-- Stats Bar -->
<div class="stats-bar">
  <div class="stat-card">
    <div class="stat-label">Total Models</div>
    <div class="stat-value blue" id="stat-total">0</div>
  </div>
  <div class="stat-card">
    <div class="stat-label">Loaded</div>
    <div class="stat-value green" id="stat-loaded">0</div>
  </div>
  <div class="stat-card">
    <div class="stat-label">Providers</div>
    <div class="stat-value purple" id="stat-providers">0</div>
  </div>
  <div class="stat-card">
    <div class="stat-label">Memory</div>
    <div class="stat-value cyan" id="stat-memory">0%</div>
  </div>
</div>

<!-- Memory Bar -->
<div class="memory-bar-container">
  <div class="memory-bar-track">
    <div class="memory-bar-fill" id="memory-fill" style="width: 0%"></div>
  </div>
  <div class="memory-bar-label">
    <span id="memory-used">0 MB</span>
    <span id="memory-total">0 MB</span>
  </div>
</div>

<!-- Main 2-column -->
<div class="main">
  <!-- Left: Models -->
  <div class="panel">
    <div class="panel-header">
      <span>Models</span>
      <span id="model-count" style="color: var(--text-dim); font-size: 12px">0 models</span>
    </div>
    <div class="model-grid" id="model-grid">
      <div class="empty-state">No models discovered yet</div>
    </div>
  </div>

  <!-- Right: Providers + Try It -->
  <div style="display: flex; flex-direction: column; gap: 16px;">
    <div class="panel">
      <div class="panel-header">Providers</div>
      <div class="panel-body" id="provider-list">
        <div class="empty-state">No providers configured</div>
      </div>
    </div>

    <div class="panel">
      <div class="panel-header">Try It</div>
      <div class="panel-body try-it">
        <select id="try-cap">
          <option value="chat">Chat</option>
          <option value="tts">TTS</option>
          <option value="asr">ASR</option>
          <option value="imagegen">Image Gen</option>
          <option value="embedding">Embedding</option>
        </select>
        <textarea id="try-input" placeholder="Type your message here..."></textarea>
        <button id="try-btn" onclick="tryInvoke()">Send Request</button>
        <div class="try-it-response" id="try-output">Response will appear here</div>
      </div>
    </div>
  </div>
</div>

<!-- Request Log -->
<div class="log-area">
  <div class="panel">
    <div class="panel-header">
      <span>Request Log</span>
      <span id="log-count" style="color: var(--text-dim); font-size: 12px">0 requests</span>
    </div>
    <div class="log-list" id="log-list">
      <div class="empty-state">No requests yet</div>
    </div>
  </div>
</div>

<!-- Footer -->
<div class="footer">
  <span>MoFA Engine &mdash; Multimodal AI Orchestration</span>
  <span>API: <code>POST /v1/invoke</code> &nbsp; Events: <code>GET /v1/events</code></span>
</div>

<!-- Modal -->
<div class="modal-overlay" id="modal-overlay" onclick="closeModal(event)">
  <div class="modal" id="modal">
    <h2 id="modal-title">Model Details</h2>
    <div id="modal-body"></div>
    <button class="modal-close" onclick="document.getElementById('modal-overlay').classList.remove('active')">Close</button>
  </div>
</div>

<script>
const logEntries = [];
const MAX_LOG = 100;

function formatUptime(secs) {
  if (secs < 60) return secs + 's';
  if (secs < 3600) return Math.floor(secs / 60) + 'm ' + (secs % 60) + 's';
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return h + 'h ' + m + 'm';
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 B';
  const mb = bytes / (1024 * 1024);
  if (mb < 1024) return mb.toFixed(0) + ' MB';
  return (mb / 1024).toFixed(1) + ' GB';
}

function statusClass(status) {
  return status || 'cold';
}

async function refresh() {
  try {
    const [capsRes, statusRes, healthRes] = await Promise.all([
      fetch('/v1/capabilities'),
      fetch('/v1/status'),
      fetch('/health'),
    ]);

    const caps = await capsRes.json();
    const status = await statusRes.json();
    const health = await healthRes.json();

    // Update header
    document.getElementById('version-badge').textContent = 'v' + health.version;
    document.getElementById('uptime-badge').textContent = formatUptime(health.uptime_secs);

    // Update stats
    document.getElementById('stat-total').textContent = status.total_models;
    document.getElementById('stat-loaded').textContent = status.loaded_models;
    document.getElementById('stat-providers').textContent = status.providers;

    const memPct = status.memory_budget_bytes > 0
      ? Math.round((status.memory_used_bytes / status.memory_budget_bytes) * 100)
      : 0;
    document.getElementById('stat-memory').textContent = memPct + '%';

    const fill = document.getElementById('memory-fill');
    fill.style.width = memPct + '%';
    fill.className = 'memory-bar-fill' + (memPct > 80 ? ' warn' : '');

    document.getElementById('memory-used').textContent = formatBytes(status.memory_used_bytes);
    document.getElementById('memory-total').textContent = formatBytes(status.memory_budget_bytes);

    // Update models
    const grid = document.getElementById('model-grid');
    document.getElementById('model-count').textContent = caps.length + ' models';
    if (caps.length === 0) {
      grid.innerHTML = '<div class="empty-state">No models discovered yet</div>';
    } else {
      grid.innerHTML = caps.map(m => `
        <div class="model-card" onclick='showModel(${JSON.stringify(m).replace(/'/g, "&#39;")})'>
          <div class="model-card-head">
            <div class="model-name" title="${m.name}">${m.name}</div>
            <div class="status-dot ${statusClass(m.status)}" title="${m.status}"></div>
          </div>
          <div class="model-meta">
            <span class="cap-badge ${m.capability}">${m.capability}</span>
          </div>
          <div class="model-provider">${m.provider} &middot; ${m.cost_tier}</div>
        </div>
      `).join('');
    }

    // Update providers
    const provList = document.getElementById('provider-list');
    if (status.provider_health && status.provider_health.length > 0) {
      provList.innerHTML = status.provider_health.map(p => `
        <div class="provider-item">
          <span class="provider-name">${p.name}</span>
          <div class="provider-status">
            <span class="circuit-label ${p.circuit_state}">${p.circuit_state}</span>
            <div class="status-dot ${p.healthy ? 'hot' : 'failed'}"></div>
          </div>
        </div>
      `).join('');
    } else {
      provList.innerHTML = '<div class="empty-state">No providers configured</div>';
    }
  } catch (e) {
    console.warn('refresh failed:', e);
  }
}

function showModel(m) {
  document.getElementById('modal-title').textContent = m.name;
  document.getElementById('modal-body').innerHTML = `
    <div class="modal-field"><label>ID</label><div class="val">${m.id}</div></div>
    <div class="modal-field"><label>Provider</label><div class="val">${m.provider}</div></div>
    <div class="modal-field"><label>Capability</label><div class="val">${m.capability}</div></div>
    <div class="modal-field"><label>Status</label><div class="val">${m.status}</div></div>
    <div class="modal-field"><label>Cost Tier</label><div class="val">${m.cost_tier}</div></div>
    <div class="modal-field"><label>Context Window</label><div class="val">${m.context_window.toLocaleString()} tokens</div></div>
    <div class="modal-field"><label>Memory</label><div class="val">${formatBytes(m.memory_estimate_bytes)}</div></div>
  `;
  document.getElementById('modal-overlay').classList.add('active');
}

function closeModal(e) {
  if (e.target.id === 'modal-overlay') {
    e.target.classList.remove('active');
  }
}

async function tryInvoke() {
  const cap = document.getElementById('try-cap').value;
  const input = document.getElementById('try-input').value.trim();
  const btn = document.getElementById('try-btn');
  const output = document.getElementById('try-output');

  if (!input) { output.textContent = 'Please enter a message'; return; }

  btn.disabled = true;
  btn.textContent = 'Sending...';
  output.textContent = 'Waiting for response...';

  try {
    const body = {
      capability: cap,
      messages: [{ role: 'user', content: input }],
    };
    const res = await fetch('/v1/invoke', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    const data = await res.json();
    if (res.ok) {
      output.textContent = (data.text || data.file || 'No output') +
        '\n\n--- ' + data.model_used + ' via ' + data.provider + ' (' + data.duration_ms + 'ms) ---';
    } else {
      output.textContent = 'Error: ' + (data.error || res.statusText);
    }
  } catch (e) {
    output.textContent = 'Error: ' + e.message;
  } finally {
    btn.disabled = false;
    btn.textContent = 'Send Request';
  }
}

function addLogEntry(entry) {
  logEntries.unshift(entry);
  if (logEntries.length > MAX_LOG) logEntries.pop();
  renderLog();
}

function renderLog() {
  const list = document.getElementById('log-list');
  document.getElementById('log-count').textContent = logEntries.length + ' requests';
  if (logEntries.length === 0) {
    list.innerHTML = '<div class="empty-state">No requests yet</div>';
    return;
  }
  list.innerHTML = logEntries.map(e => `
    <div class="log-entry">
      <span class="log-time">${e.time}</span>
      <span class="log-model">${e.model}</span>
      <span class="log-duration">${e.duration}ms</span>
      <span class="${e.ok ? 'log-status-ok' : 'log-status-err'}">${e.ok ? 'OK' : 'FAIL'}</span>
    </div>
  `).join('');
}

// SSE for live events
function connectSSE() {
  const es = new EventSource('/v1/events');
  es.onmessage = function(e) {
    try {
      const evt = JSON.parse(e.data);
      if (evt.type === 'request_completed') {
        addLogEntry({
          time: new Date().toLocaleTimeString(),
          model: evt.request_id ? evt.request_id.substring(0, 8) : '?',
          duration: evt.duration_ms || 0,
          ok: evt.success,
        });
      }
      // Refresh data on any event
      refresh();
    } catch (err) { /* ignore parse errors */ }
  };
  es.onerror = function() {
    es.close();
    setTimeout(connectSSE, 3000);
  };
}

// Init
refresh();
setInterval(refresh, 2000);
connectSSE();
</script>

</body>
</html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_html_is_valid() {
        assert!(DASHBOARD_HTML.contains("<!DOCTYPE html>"));
        assert!(DASHBOARD_HTML.contains("MoFA Engine"));
        assert!(DASHBOARD_HTML.contains("/v1/capabilities"));
        assert!(DASHBOARD_HTML.contains("/v1/invoke"));
        assert!(DASHBOARD_HTML.contains("/v1/events"));
    }

    #[test]
    fn dashboard_html_has_css_variables() {
        assert!(DASHBOARD_HTML.contains("--bg-primary"));
        assert!(DASHBOARD_HTML.contains("--accent-blue"));
    }
}
