// atomr-agents · coding-cli harness · vanilla SPA

const $ = (s) => document.querySelector(s);
const eventLog = $('#event-log');
const sessionList = $('#session-list');
const runStatus = $('#run-status');

// --- Vendors --------------------------------------------------------
async function loadVendors() {
  const r = await fetch('/api/cli/vendors').then((r) => r.json());
  const sel = $('#vendor-select');
  sel.innerHTML = '';
  for (const v of r.vendors) {
    const opt = document.createElement('option');
    opt.value = v;
    opt.textContent = v;
    sel.appendChild(opt);
  }
}

// --- SSE event log --------------------------------------------------
function tagFor(kind) {
  switch (kind) {
    case 'assistant_text_delta': return 'delta';
    case 'tool_call_started':
    case 'tool_call_finished': return 'tool';
    case 'system_init': return 'init';
    case 'usage': return 'usage';
    case 'run_finished': return 'done';
    case 'api_retry':
    case 'note': return 'error';
    default: return 'raw';
  }
}

function startSse() {
  const es = new EventSource('/api/cli/runs/events');
  es.addEventListener('coding_cli_event', (e) => {
    let ev;
    try { ev = JSON.parse(e.data); } catch { return; }
    const li = document.createElement('li');
    const tag = document.createElement('span');
    tag.className = `tag ${tagFor(ev.kind)}`;
    tag.textContent = ev.kind;
    li.appendChild(tag);
    li.appendChild(document.createTextNode(summarize(ev)));
    eventLog.prepend(li);
  });
  es.onerror = () => {
    // Browser will auto-reconnect.
  };
}

function summarize(ev) {
  switch (ev.kind) {
    case 'assistant_text_delta':
      return JSON.stringify(ev.text);
    case 'tool_call_started':
      return `${ev.name}(${JSON.stringify(ev.input).slice(0, 80)})`;
    case 'tool_call_finished':
      return ev.error ? `error: ${ev.error}` : 'ok';
    case 'system_init':
      return `${ev.tools.length} tools, ${ev.mcp_servers.length} mcp`;
    case 'usage':
      return `${ev.input_tokens} in / ${ev.output_tokens} out`;
    case 'run_finished':
      return `${ev.reason}: ${ev.result_text ? ev.result_text.slice(0, 80) : ''}`;
    case 'note':
      return ev.message;
    case 'api_retry':
      return `attempt ${ev.attempt} (+${ev.delay_ms}ms): ${ev.reason}`;
    default:
      return JSON.stringify(ev).slice(0, 120);
  }
}

// --- Submit form ----------------------------------------------------
function buildRequest(form) {
  return {
    vendor: form.vendor.value,
    mode: form.mode.value,
    prompt: form.prompt.value || '',
    workdir: form.workdir.value,
    model: form.model.value ? form.model.value : null,
    allowed_tools: [],
    project: {},
    isolation: { kind: 'local' },
    budget: {},
    metadata: {},
  };
}

$('#run-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const req = buildRequest(e.target);
  if (req.mode === 'interactive') {
    const r = await fetch('/api/cli/sessions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(req),
    });
    if (!r.ok) {
      runStatus.textContent = `failed: ${await r.text()}`;
      return;
    }
    const { session_id } = await r.json();
    runStatus.textContent = `interactive session ${session_id} attached`;
    openTerminal(session_id);
    refreshSessions();
  } else {
    const r = await fetch('/api/cli/runs', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(req),
    });
    if (!r.ok) {
      runStatus.textContent = `failed: ${await r.text()}`;
      return;
    }
    const { run_id } = await r.json();
    runStatus.textContent = `headless run ${run_id} started — events streaming below`;
  }
});

// --- Sessions list --------------------------------------------------
async function refreshSessions() {
  const { sessions } = await fetch('/api/cli/sessions').then((r) => r.json());
  sessionList.innerHTML = '';
  for (const s of sessions) {
    const li = document.createElement('li');
    li.innerHTML = `<code>${s.id}</code> · ${s.vendor} · ${s.tmux_session}`;
    const open = document.createElement('button');
    open.textContent = 'attach';
    open.style.marginLeft = '8px';
    open.onclick = () => openTerminal(s.id);
    li.appendChild(open);
    const stop = document.createElement('button');
    stop.textContent = 'stop';
    stop.style.marginLeft = '4px';
    stop.onclick = async () => {
      await fetch(`/api/cli/sessions/${s.id}`, { method: 'DELETE' });
      refreshSessions();
    };
    li.appendChild(stop);
    sessionList.appendChild(li);
  }
}

// --- Terminal -------------------------------------------------------
let activeTerm = null;
let activeWs = null;

function openTerminal(sessionId) {
  $('#terminal').classList.remove('hidden');
  $('#term-meta').textContent = `session ${sessionId}`;

  if (activeWs) { activeWs.close(); }
  if (activeTerm) { activeTerm.dispose(); activeTerm = null; }

  const term = new Terminal({ convertEol: true, fontFamily: 'ui-monospace, monospace' });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open($('#xterm'));
  fit.fit();
  activeTerm = term;

  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(`${proto}//${location.host}/api/cli/sessions/${sessionId}/io`);
  ws.binaryType = 'arraybuffer';
  activeWs = ws;

  ws.onopen = () => {
    ws.send(JSON.stringify({ kind: 'resize', cols: term.cols, rows: term.rows }));
  };
  ws.onmessage = (evt) => {
    if (typeof evt.data === 'string') {
      // control frames (e.g. {"kind":"exited",...})
      try {
        const f = JSON.parse(evt.data);
        if (f.kind === 'exited') {
          term.write(`\r\n\x1b[33m[session exited: ${f.code}]\x1b[0m\r\n`);
        }
      } catch {}
      return;
    }
    const bytes = new Uint8Array(evt.data);
    term.write(bytes);
  };
  ws.onclose = () => {
    term.write('\r\n\x1b[31m[disconnected]\x1b[0m\r\n');
  };

  term.onData((data) => {
    if (ws.readyState === 1) {
      ws.send(new TextEncoder().encode(data));
    }
  });

  window.addEventListener('resize', () => {
    fit.fit();
    if (ws.readyState === 1) {
      ws.send(JSON.stringify({ kind: 'resize', cols: term.cols, rows: term.rows }));
    }
  });

  $('#detach-btn').onclick = () => {
    if (ws.readyState === 1) {
      ws.send(JSON.stringify({ kind: 'detach' }));
    }
    ws.close();
    $('#terminal').classList.add('hidden');
  };
}

// --- Boot -----------------------------------------------------------
loadVendors();
startSse();
refreshSessions();
setInterval(refreshSessions, 5000);
