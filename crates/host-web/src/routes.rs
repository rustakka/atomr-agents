//! REST + SSE route registration. Each handler is a thin wrapper over an
//! existing `atomr-agents-host` function.

use atomr_agents_host::error::HostError;
use atomr_agents_host::layout::AgentPaths;
use atomr_agents_host::markdown::MarkdownDoc;
use atomr_agents_host::{
    branching, chat, curator, evals, gateway, mcp, registry_cache, scheduler, skills_registry,
    HostConfig,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::diskio;
use crate::dto::{concept_catalog, AgentDetail, AgentSummary, DocUpdate};
use crate::error::{WebError, WebResult};
use crate::AppState;

pub fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .route("/concepts", get(list_concepts))
        // agents
        .route("/agents", get(list_agents))
        .route("/agents/:id", get(get_agent).delete(stop_agent))
        .route("/agents/:id/spawn", post(spawn_agent))
        .route("/agents/:id/reload", post(reload_agent))
        .route("/agents/:id/chat", post(chat_agent))
        // docs
        .route("/agents/:id/docs/:doc", get(get_doc).put(put_doc))
        // skills
        .route("/agents/:id/skills", get(list_skills).post(create_skill))
        .route("/agents/:id/skills/validate", get(validate_skills))
        .route("/agents/:id/skills/:sid", put(put_skill).delete(delete_skill))
        // curator
        .route("/agents/:id/curator/proposals", get(list_proposals))
        .route("/agents/:id/curator/proposals/:sid/approve", post(approve_proposal))
        .route("/agents/:id/curator/proposals/:sid/reject", post(reject_proposal))
        .route("/agents/:id/curator/history/:sid", get(list_history))
        .route("/agents/:id/curator/revert/:sid", post(revert_skill))
        // hooks
        .route("/agents/:id/hooks", get(list_hooks))
        // branches
        .route("/agents/:id/branches", get(list_branches).post(fork_branch))
        .route("/agents/:id/branches/diff", get(diff_branches))
        .route("/agents/:id/branches/:b/switch", post(switch_branch))
        .route("/agents/:id/branches/:b", delete(delete_branch))
        // crons
        .route("/crons", get(list_crons).post(create_cron))
        .route("/crons/:id", delete(delete_cron))
        // routing + channels
        .route("/routes", get(get_routes))
        .route("/channels", get(list_channels))
        // registry
        .route("/registry", get(list_registry))
        .route("/registry/:kind/:id/:version", delete(delete_registry))
        // evals
        .route("/evals", get(list_evals))
        .route("/evals/:id", get(get_eval))
        .route("/evals/:id/run", post(run_eval))
        // mcp
        .route("/mcp", get(list_mcp).post(create_mcp))
        // config
        .route("/config", get(get_config).put(put_config))
        // events
        .route("/events", get(list_events))
        .route("/events/stream", get(crate::sse::sse_events))
        .with_state(state.clone());

    Router::new()
        .nest("/api", api)
        .route("/healthz", get(|| async { "ok" }))
        .fallback(crate::spa::serve_embedded)
        .with_state(state)
        .layer(tower_http::cors::CorsLayer::permissive())
}

// ----- helpers ----------------------------------------------------------

fn emit(state: &AppState, kind: &str, agent: Option<&str>, payload: Value) {
    if let Err(e) = state.events.emit(kind, agent.map(|s| s.to_string()), payload) {
        tracing::warn!(error = %e, "failed to append event");
    }
}

fn ensure_agent(state: &AppState, id: &str) -> WebResult<AgentPaths> {
    let apaths = state.paths.agent(id);
    if !apaths.dir().is_dir() {
        return Err(WebError::NotFound(format!("no agent `{id}`")));
    }
    Ok(apaths)
}

fn string_vec(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

// ----- concepts ---------------------------------------------------------

async fn list_concepts() -> impl IntoResponse {
    Json(json!({ "concepts": concept_catalog() }))
}

// ----- agents -----------------------------------------------------------

async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let loader = state.runtime.loader();
    let mut agents = Vec::new();
    for id in state.paths.list_agent_ids() {
        if let Ok(loaded) = loader.load(&id) {
            let running = state.runtime.lookup(&id).is_some();
            agents.push(AgentSummary::from_loaded(&loaded, running));
        }
    }
    Json(json!({ "agents": agents }))
}

async fn get_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    ensure_agent(&state, &id)?;
    let loaded = state.runtime.loader().load(&id)?;
    let running = state.runtime.lookup(&id).is_some();
    Ok(Json(AgentDetail::from_loaded(&loaded, running)))
}

async fn spawn_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    ensure_agent(&state, &id)?;
    state.runtime.spawn_agent(&id).await?;
    emit(&state, "agent.spawned", Some(&id), json!({}));
    Ok(Json(json!({ "ok": true })))
}

async fn stop_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    state.runtime.stop_agent(&id);
    emit(&state, "agent.stopped", Some(&id), json!({}));
    StatusCode::NO_CONTENT
}

async fn reload_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    ensure_agent(&state, &id)?;
    state.runtime.reload(&id).await?;
    emit(&state, "agent.reloaded", Some(&id), json!({}));
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ChatBody {
    message: String,
}

async fn chat_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ChatBody>,
) -> WebResult<impl IntoResponse> {
    ensure_agent(&state, &id)?;
    let handle = state.runtime.spawn_agent(&id).await?;
    let reply = handle.preview(body.message).await?;
    Ok(Json(json!({ "reply": reply })))
}

// ----- docs -------------------------------------------------------------

async fn get_doc(
    State(state): State<AppState>,
    Path((id, doc)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let path = diskio::doc_path(&apaths, &doc)
        .ok_or_else(|| WebError::BadRequest(format!("unknown doc `{doc}`")))?;
    let md = MarkdownDoc::read(&path)?;
    Ok(Json(md))
}

async fn put_doc(
    State(state): State<AppState>,
    Path((id, doc)): Path<(String, String)>,
    Json(update): Json<DocUpdate>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    if diskio::doc_path(&apaths, &doc).is_none() {
        return Err(WebError::BadRequest(format!("unknown doc `{doc}`")));
    }
    let md = update.into_doc();
    diskio::write_doc(&apaths, &doc, &md)?;
    if state.runtime.lookup(&id).is_some() {
        state.runtime.reload(&id).await?;
    }
    emit(&state, "doc.saved", Some(&id), json!({ "doc": doc }));
    Ok(Json(json!({ "ok": true })))
}

// ----- skills -----------------------------------------------------------

async fn list_skills(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    ensure_agent(&state, &id)?;
    let def = state.runtime.loader().parse(&id)?;
    Ok(Json(json!({ "skills": def.skills })))
}

#[derive(Deserialize)]
struct NewSkill {
    id: String,
    name: String,
    #[serde(default)]
    priority: Option<u8>,
    #[serde(default)]
    keywords: Vec<String>,
}

async fn create_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<NewSkill>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let path = skills_registry::scaffold_skill(
        &apaths,
        &body.id,
        &body.name,
        body.priority.unwrap_or(5),
        &body.keywords,
    )?;
    emit(&state, "skill.created", Some(&id), json!({ "skill_id": body.id }));
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "path": path.display().to_string() })),
    ))
}

async fn put_skill(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
    Json(update): Json<DocUpdate>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let md = update.into_doc();
    let path = diskio::write_skill(&apaths, &sid, &md)?;
    if state.runtime.lookup(&id).is_some() {
        state.runtime.reload(&id).await?;
    }
    emit(&state, "skill.saved", Some(&id), json!({ "skill_id": sid }));
    Ok(Json(json!({ "ok": true, "path": path.display().to_string() })))
}

async fn delete_skill(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    diskio::delete_skill(&apaths, &sid)?;
    if state.runtime.lookup(&id).is_some() {
        state.runtime.reload(&id).await?;
    }
    emit(&state, "skill.deleted", Some(&id), json!({ "skill_id": sid }));
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_skills(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let reports = skills_registry::validate_skills(&apaths)?;
    let out: Vec<Value> = reports
        .iter()
        .map(|r| {
            json!({
                "skill_id": r.skill_id,
                "path": r.path.display().to_string(),
                "errors": r.errors,
                "warnings": r.warnings,
            })
        })
        .collect();
    Ok(Json(json!({ "reports": out })))
}

// ----- curator ----------------------------------------------------------

async fn list_proposals(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let proposals = curator::list_proposals(&apaths)?;
    Ok(Json(json!({ "proposals": proposals })))
}

fn read_proposal(
    apaths: &AgentPaths,
    agent_id: &str,
    sid: &str,
) -> WebResult<curator::SkillProposal> {
    let path = apaths
        .skills_dir()
        .join(".proposed")
        .join(sid)
        .join("SKILL.md");
    if !path.is_file() {
        return Err(WebError::NotFound(format!("no proposal `{sid}`")));
    }
    let doc = MarkdownDoc::read(&path)?;
    let name = doc
        .frontmatter
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(sid)
        .to_string();
    let priority = doc
        .frontmatter
        .get("priority")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as u8;
    Ok(curator::SkillProposal {
        agent_id: agent_id.to_string(),
        skill_id: sid.to_string(),
        name,
        body: doc.body,
        keywords: string_vec(doc.frontmatter.get("keywords")),
        tool_overlay: string_vec(doc.frontmatter.get("tool_overlay")),
        priority,
        rationale: None,
        success_rate: None,
    })
}

async fn approve_proposal(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let proposal = read_proposal(&apaths, &id, &sid)?;
    let path = curator::promote_proposal(&apaths, &proposal, 20)?;
    let _ = curator::reject_proposal(&apaths, &sid)?; // clear the .proposed entry
    if state.runtime.lookup(&id).is_some() {
        state.runtime.reload(&id).await?;
    }
    emit(&state, "skill.promoted", Some(&id), json!({ "skill_id": sid }));
    Ok(Json(json!({ "ok": true, "path": path.display().to_string() })))
}

async fn reject_proposal(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let removed = curator::reject_proposal(&apaths, &sid)?;
    emit(&state, "skill.rejected", Some(&id), json!({ "skill_id": sid }));
    Ok(Json(json!({ "ok": true, "removed": removed })))
}

async fn list_history(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let hist = curator::list_history(&apaths, &sid)?;
    let paths: Vec<String> = hist.iter().map(|p| p.display().to_string()).collect();
    Ok(Json(json!({ "history": paths })))
}

async fn revert_skill(
    State(state): State<AppState>,
    Path((id, sid)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let reverted = curator::revert_skill(&apaths, &sid)?;
    if state.runtime.lookup(&id).is_some() {
        state.runtime.reload(&id).await?;
    }
    emit(&state, "skill.reverted", Some(&id), json!({ "skill_id": sid }));
    Ok(Json(json!({
        "ok": true,
        "reverted": reverted.map(|p| p.display().to_string()),
    })))
}

// ----- hooks ------------------------------------------------------------

async fn list_hooks(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    ensure_agent(&state, &id)?;
    let def = state.runtime.loader().parse(&id)?;
    Ok(Json(json!({ "hooks": def.hooks })))
}

// ----- branches ---------------------------------------------------------

async fn list_branches(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let current = branching::current_branch(&apaths)?;
    let branches = branching::list_branches(&apaths)?;
    Ok(Json(json!({ "current": current, "branches": branches })))
}

#[derive(Deserialize)]
struct ForkBody {
    #[serde(default)]
    source: Option<String>,
    new: String,
}

async fn fork_branch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ForkBody>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let source = body.source.as_deref().unwrap_or(branching::DEFAULT_BRANCH);
    let checkpoint = branching::fork_branch(&apaths, source, &body.new)?;
    emit(&state, "branch.forked", Some(&id), json!({ "branch": body.new }));
    Ok((StatusCode::CREATED, Json(json!({ "ok": true, "checkpoint": checkpoint }))))
}

async fn switch_branch(
    State(state): State<AppState>,
    Path((id, b)): Path<(String, String)>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    branching::switch_branch(&apaths, &b)?;
    emit(&state, "branch.switched", Some(&id), json!({ "branch": b }));
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct DiffQuery {
    a: String,
    b: String,
}

async fn diff_branches(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    let diff = branching::diff_branches(&apaths, &q.a, &q.b)?;
    Ok(Json(diff))
}

#[derive(Deserialize)]
struct ForceQuery {
    #[serde(default)]
    force: bool,
}

async fn delete_branch(
    State(state): State<AppState>,
    Path((id, b)): Path<(String, String)>,
    Query(q): Query<ForceQuery>,
) -> WebResult<impl IntoResponse> {
    let apaths = ensure_agent(&state, &id)?;
    branching::delete_branch(&apaths, &b, q.force)?;
    emit(&state, "branch.deleted", Some(&id), json!({ "branch": b }));
    Ok(StatusCode::NO_CONTENT)
}

// ----- crons ------------------------------------------------------------

async fn list_crons(State(state): State<AppState>) -> WebResult<impl IntoResponse> {
    let crons = scheduler::load_crons(&state.paths.crons_dir())?;
    Ok(Json(json!({ "crons": crons })))
}

#[derive(Deserialize)]
struct NewCron {
    id: String,
    expression: String,
    #[serde(default)]
    call: Value,
}

async fn create_cron(
    State(state): State<AppState>,
    Json(body): Json<NewCron>,
) -> WebResult<impl IntoResponse> {
    // Validate the expression before writing.
    scheduler::parse_expression(&body.expression)
        .map_err(|e| WebError::BadRequest(e.to_string()))?;
    let call = if body.call.is_null() {
        json!({})
    } else {
        body.call
    };
    let path = scheduler::scaffold_cron(&state.paths.crons_dir(), &body.id, &body.expression, call)?;
    emit(&state, "cron.created", None, json!({ "cron_id": body.id }));
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "path": path.display().to_string() })),
    ))
}

async fn delete_cron(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    let path = state.paths.crons_dir().join(format!("{id}.yaml"));
    let removed = diskio::remove_file(&path)?;
    if !removed {
        return Err(WebError::NotFound(format!("no cron `{id}`")));
    }
    emit(&state, "cron.deleted", None, json!({ "cron_id": id }));
    Ok(StatusCode::NO_CONTENT)
}

// ----- routing + channels ----------------------------------------------

async fn get_routes(State(state): State<AppState>) -> WebResult<impl IntoResponse> {
    let rules = gateway::load_agents_md(&state.paths.agents_md())?;
    let channel_pins: serde_json::Map<String, Value> = rules
        .channel_pins
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let mut peer_pins = serde_json::Map::new();
    for ((ch, peer), ag) in &rules.peer_pins {
        peer_pins.insert(format!("{ch}:{peer}"), Value::String(ag.clone()));
    }
    Ok(Json(json!({
        "default_agent": rules.default_agent,
        "channel_pins": channel_pins,
        "peer_pins": peer_pins,
    })))
}

async fn list_channels(State(state): State<AppState>) -> impl IntoResponse {
    let dir = state.paths.channels_dir();
    let mut channels = Vec::new();
    if dir.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if let Some(stem) = e.path().file_stem().and_then(|s| s.to_str()) {
                    channels.push(stem.to_string());
                }
            }
        }
    }
    channels.sort();
    Json(json!({ "channels": channels }))
}

// ----- registry ---------------------------------------------------------

#[derive(Deserialize)]
struct KindQuery {
    #[serde(default)]
    kind: Option<String>,
}

async fn list_registry(
    State(state): State<AppState>,
    Query(q): Query<KindQuery>,
) -> WebResult<impl IntoResponse> {
    let artifacts = registry_cache::list_artifacts(&state.paths, q.kind.as_deref())?;
    Ok(Json(json!({ "artifacts": artifacts })))
}

async fn delete_registry(
    State(state): State<AppState>,
    Path((kind, id, version)): Path<(String, String, String)>,
) -> WebResult<impl IntoResponse> {
    let removed = registry_cache::delete_artifact(&state.paths, &kind, &id, &version)?;
    if !removed {
        return Err(WebError::NotFound(format!("no artifact {kind}:{id}@{version}")));
    }
    emit(&state, "registry.deleted", None, json!({ "kind": kind, "id": id, "version": version }));
    Ok(StatusCode::NO_CONTENT)
}

// ----- evals ------------------------------------------------------------

async fn list_evals(State(state): State<AppState>) -> WebResult<impl IntoResponse> {
    let suites = evals::list_suites(&state.paths)?;
    Ok(Json(json!({ "suites": suites })))
}

async fn get_eval(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> WebResult<impl IntoResponse> {
    let suite = evals::load_suite(&state.paths, &id)
        .map_err(|e| WebError::NotFound(e.to_string()))?;
    Ok(Json(suite))
}

#[derive(Deserialize)]
struct AgentQuery {
    agent: String,
}

async fn run_eval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<AgentQuery>,
) -> WebResult<impl IntoResponse> {
    let suite = evals::load_suite(&state.paths, &id)
        .map_err(|e| WebError::NotFound(e.to_string()))?;
    ensure_agent(&state, &q.agent)?;
    let loaded = state.runtime.loader().load(&q.agent)?;
    let run = evals::run_suite_sync(&suite, &q.agent, |input| {
        chat::render_chat_preview(&loaded, input)
    })?;
    emit(
        &state,
        "eval.ran",
        Some(&q.agent),
        json!({ "suite": id, "passed": run.passed, "total": run.total }),
    );
    Ok(Json(run))
}

// ----- mcp --------------------------------------------------------------

async fn list_mcp(State(state): State<AppState>) -> WebResult<impl IntoResponse> {
    let servers = mcp::load_mcp_servers(&state.paths.mcp_dir())?;
    Ok(Json(json!({ "servers": servers })))
}

#[derive(Deserialize)]
struct NewMcp {
    id: String,
    #[serde(default)]
    command: Vec<String>,
}

async fn create_mcp(
    State(state): State<AppState>,
    Json(body): Json<NewMcp>,
) -> WebResult<impl IntoResponse> {
    let path = mcp::scaffold_mcp_server(&state.paths.mcp_dir(), &body.id, body.command)?;
    emit(&state, "mcp.created", None, json!({ "id": body.id }));
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "path": path.display().to_string() })),
    ))
}

// ----- config -----------------------------------------------------------

async fn get_config(State(state): State<AppState>) -> WebResult<impl IntoResponse> {
    let yaml = state.runtime.config().to_yaml_string()?;
    let parsed: Value = serde_yaml::from_str(&yaml).unwrap_or(Value::Null);
    Ok(Json(json!({ "yaml": yaml, "parsed": parsed })))
}

#[derive(Deserialize)]
struct ConfigBody {
    yaml: String,
}

async fn put_config(
    State(state): State<AppState>,
    Json(body): Json<ConfigBody>,
) -> WebResult<impl IntoResponse> {
    // Validate: must parse as a mapping and a valid HostConfig.
    let raw: serde_yaml::Value = serde_yaml::from_str(&body.yaml)?;
    HostConfig::from_yaml_value(raw, state.paths.clone())?;
    let path = state.paths.config_yaml();
    std::fs::write(&path, &body.yaml).map_err(|e| HostError::io(&path, e))?;
    emit(&state, "config.saved", None, json!({}));
    Ok(Json(json!({ "ok": true })))
}

// ----- events -----------------------------------------------------------

#[derive(Deserialize)]
struct LimitQuery {
    #[serde(default)]
    limit: Option<usize>,
}

async fn list_events(
    State(state): State<AppState>,
    Query(q): Query<LimitQuery>,
) -> WebResult<impl IntoResponse> {
    let mut all = state.events.read_all()?;
    let limit = q.limit.unwrap_or(200);
    if all.len() > limit {
        all = all.split_off(all.len() - limit);
    }
    Ok(Json(json!({ "events": all })))
}
