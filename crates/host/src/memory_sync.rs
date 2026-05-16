//! Stub for M3 — MemorySyncActor + RULES rendering.
//!
//! Will sync MEMORY.md/USER.md ↔ MemoryStore and assemble the system
//! prompt from persona+rules+memory+user blocks.

use crate::loader::LoadedAgent;

/// Render the persona block of a system prompt (or `None` if no persona).
pub fn render_persona_block(loaded: &LoadedAgent) -> Option<String> {
    let p = loaded.persona.as_ref()?;
    let mut out = format!("identity: {}\n", p.identity);
    if let Some(tone) = &p.style.tone {
        out.push_str(&format!("tone: {tone}\n"));
    }
    if let Some(register) = &p.style.register {
        out.push_str(&format!("register: {register}\n"));
    }
    if !p.salient_traits.is_empty() {
        out.push_str("traits:\n");
        for t in &p.salient_traits {
            out.push_str(&format!("  - {} ({:.2})\n", t.label, t.weight));
        }
    }
    Some(out)
}

pub fn render_rules_block(loaded: &LoadedAgent) -> Option<String> {
    if loaded.rules.is_empty() {
        return None;
    }
    let mut s = String::from("rules:\n");
    for r in &loaded.rules {
        s.push_str(&format!("  - {r}\n"));
    }
    Some(s)
}

pub fn render_memory_block(loaded: &LoadedAgent) -> Option<String> {
    if loaded.memory_facts.is_empty() {
        return None;
    }
    let mut s = String::from("memory facts:\n");
    for r in &loaded.memory_facts {
        s.push_str(&format!("  - {r}\n"));
    }
    Some(s)
}

pub fn render_user_block(loaded: &LoadedAgent) -> Option<String> {
    let body = loaded.user_profile.trim();
    if body.is_empty() {
        return None;
    }
    Some(format!("user profile:\n{body}\n"))
}

pub fn build_system_prompt(loaded: &LoadedAgent) -> String {
    let mut out = String::new();
    if let Some(p) = render_persona_block(loaded) {
        out.push_str(&p);
        out.push('\n');
    }
    if let Some(p) = render_rules_block(loaded) {
        out.push_str(&p);
        out.push('\n');
    }
    if let Some(p) = render_memory_block(loaded) {
        out.push_str(&p);
        out.push('\n');
    }
    if let Some(p) = render_user_block(loaded) {
        out.push_str(&p);
    }
    out
}
