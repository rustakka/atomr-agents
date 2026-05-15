// Deep Research Harness — embedded SPA (vanilla JS).
//
// Talks to the same axum server it is served from. Routes:
//   GET  /api/research              → list summaries
//   POST /api/research              → start a run, returns { id }
//   GET  /api/research/:id          → full result
//   GET  /api/research/events       → SSE stream of every event
//   GET  /api/strategies            → list of strategy ids
//
// The dashboard polls the active run every 1s for the full snapshot
// (cheaper than recomputing UI state from the event stream). The SSE
// stream is used to render a live event log.

const $ = (q) => document.querySelector(q);
const $$ = (q) => document.querySelectorAll(q);

let activeId = null;
let pollHandle = null;

async function loadStrategies() {
  const sel = $("#strategy-select");
  try {
    const r = await fetch("/api/strategies");
    const list = await r.json();
    sel.innerHTML = list
      .map((s) => `<option value="${s}">${s}</option>`)
      .join("");
  } catch (e) {
    sel.innerHTML = `<option value="clarify-plan-search-verify">clarify-plan-search-verify</option>`;
  }
}

async function loadRuns() {
  const ul = $("#runs-list");
  try {
    const r = await fetch("/api/research");
    const rows = await r.json();
    if (!Array.isArray(rows) || rows.length === 0) {
      ul.innerHTML = `<li class="muted">No runs yet — start one above.</li>`;
      return;
    }
    ul.innerHTML = rows
      .map(
        (row) => `
        <li data-id="${row.id}" class="${row.id === activeId ? "active" : ""}">
          <div class="query">${escapeHtml(row.query)}</div>
          <div class="meta">${row.strategy} <span class="state-pill ${stateClass(row.state)}">${row.state}</span></div>
        </li>`
      )
      .join("");
    $$("#runs-list li").forEach((li) =>
      li.addEventListener("click", () => openRun(li.dataset.id))
    );
  } catch (e) {
    ul.innerHTML = `<li class="muted">Error: ${escapeHtml(e.message)}</li>`;
  }
}

function stateClass(s) {
  if (s === "done") return "done";
  if (s === "failed") return "failed";
  return "running";
}

async function openRun(id) {
  activeId = id;
  $("#details").hidden = false;
  if (pollHandle) clearInterval(pollHandle);
  await refreshDetails();
  pollHandle = setInterval(refreshDetails, 1000);
  loadRuns();
}

async function refreshDetails() {
  if (!activeId) return;
  const r = await fetch(`/api/research/${activeId}`);
  if (!r.ok) return;
  const result = await r.json();
  renderDetails(result);
  // If the run is terminal, stop polling.
  if (result.state === "done" || result.state === "failed") {
    if (pollHandle) {
      clearInterval(pollHandle);
      pollHandle = null;
    }
  }
}

function renderDetails(r) {
  $("#detail-query").textContent = r.query || "—";
  $("#detail-meta").innerHTML =
    `${escapeHtml(r.strategy)} <span class="state-pill ${stateClass(r.state)}">${r.state}</span>` +
    ` &middot; ${r.citations?.length || 0} citations` +
    ` &middot; iter ${r.transcript?.length || 0}` +
    (r.coverage?.sub_questions_answered != null
      ? ` &middot; answered ${r.coverage.sub_questions_answered}/${(r.coverage.sub_questions_answered || 0) + (r.coverage.sub_questions_unresolved || 0)}`
      : "");

  const plan = r.plan || { sub_questions: [] };
  $("#plan-list").innerHTML = (plan.sub_questions || [])
    .map(
      (s) =>
        `<li>${escapeHtml(s.text)} <span class="state-pill ${s.status === "answered" ? "done" : ""}">${s.status}</span></li>`
    )
    .join("");

  $("#citations-list").innerHTML = (r.citations || [])
    .map(
      (c) =>
        `<li>[${c.number}] <a href="${c.url}" target="_blank">${escapeHtml(c.title)}</a> &middot; ${escapeHtml(c.source)}</li>`
    )
    .join("");

  $("#report-body").textContent = r.final_report || "(no final report yet)";
}

let sseSource = null;
function startSse() {
  if (sseSource) return;
  sseSource = new EventSource("/api/research/events");
  sseSource.addEventListener("deep_research_event", (e) => {
    try {
      const ev = JSON.parse(e.data);
      appendEvent(ev);
    } catch {}
  });
  sseSource.onerror = () => {
    // The browser auto-retries; nothing to do.
  };
}

function appendEvent(ev) {
  const ul = $("#events-list");
  const li = document.createElement("li");
  li.innerHTML = `<b>${escapeHtml(ev.kind)}</b>${escapeHtml(JSON.stringify(redact(ev)).slice(0, 200))}`;
  ul.prepend(li);
  while (ul.children.length > 200) ul.removeChild(ul.lastChild);
  // Drive a list refresh on terminal events.
  if (ev.kind === "finalized" || ev.kind === "failed") {
    loadRuns();
  }
}

function redact(ev) {
  const { kind, ...rest } = ev;
  return rest;
}

function escapeHtml(s) {
  return String(s || "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

$("#run-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = new FormData(e.target);
  const body = {
    request: {
      query: form.get("query"),
      depth: Number(form.get("depth")),
      breadth: Number(form.get("breadth")),
    },
    strategy: form.get("strategy"),
    max_iterations: Number(form.get("max_iterations")),
  };
  const status = $("#run-status");
  status.textContent = "Starting…";
  try {
    const r = await fetch("/api/research", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!r.ok) throw new Error(await r.text());
    const { id } = await r.json();
    status.textContent = `Run ${id} started.`;
    await loadRuns();
    openRun(id);
  } catch (err) {
    status.textContent = `Error: ${err.message}`;
  }
});

loadStrategies();
loadRuns();
startSse();
setInterval(loadRuns, 4000);
