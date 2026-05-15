//! Writer role + deterministic [`ConcatWriter`] default.

use async_trait::async_trait;
use atomr_agents_deep_research_core::{
    DraftSection, NodeKind, NodeStep, OutputFormat, Plan, ResearchRequest,
};

use crate::error::Result;
use crate::handle::ResearchHandle;

/// Role: stitch evidence into draft sections and a final report body.
#[async_trait]
pub trait Writer: Send + Sync + 'static {
    async fn write(&self, plan: &Plan, handle: &ResearchHandle) -> Result<()>;
}

#[async_trait]
impl Writer for Box<dyn Writer> {
    async fn write(&self, plan: &Plan, handle: &ResearchHandle) -> Result<()> {
        (**self).write(plan, handle).await
    }
}

/// Deterministic default. For each outline heading, gathers the
/// citations whose `supports` references a sub-question assigned to
/// that heading and emits a markdown body of the form:
///
/// ```text
/// ## Background
///
/// **what is X?** Lorem ipsum… [1] [2]
/// **how does Y compare?** Dolor sit… [3]
/// ```
///
/// The full report is the headings concatenated together, prefixed by
/// a title derived from the request.
#[derive(Default)]
pub struct ConcatWriter;

impl ConcatWriter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Writer for ConcatWriter {
    async fn write(&self, plan: &Plan, handle: &ResearchHandle) -> Result<()> {
        let snap = handle.snapshot();
        let request: ResearchRequest = (*handle.request()).clone();

        let prefer_markdown = matches!(request.output_format, OutputFormat::Markdown { .. });

        // Map sub_question_id → headings.
        let section_for_sq = |sq_id: &str| -> Option<&str> {
            plan.sub_questions
                .iter()
                .find(|s| s.id == sq_id)
                .and_then(|s| s.section.as_deref())
        };

        // Group citations by heading.
        let headings: Vec<String> = if plan.outline.is_empty() {
            vec!["Findings".into()]
        } else {
            plan.outline.clone()
        };

        let mut body = String::new();
        if prefer_markdown {
            body.push_str(&format!("# {}\n\n", title_of(&request.query)));
        } else {
            body.push_str(&format!("{}\n\n", title_of(&request.query)));
        }

        for heading in &headings {
            if prefer_markdown {
                body.push_str(&format!("## {heading}\n\n"));
            } else {
                body.push_str(&format!("{heading}\n\n"));
            }
            let mut section_body = String::new();
            let mut answered_ids: Vec<String> = Vec::new();
            for sq in plan
                .sub_questions
                .iter()
                .filter(|s| section_for_sq(&s.id) == Some(heading.as_str()))
            {
                let cites: Vec<u32> = snap
                    .citations
                    .iter()
                    .filter(|c| c.supports.iter().any(|id| id == &sq.id))
                    .map(|c| c.number)
                    .collect();
                if cites.is_empty() {
                    section_body.push_str(&format!("**{}** No corroborating sources found.\n\n", sq.text));
                } else {
                    let markers: String = cites
                        .iter()
                        .map(|n| format!("[{n}]"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let snippet_preview = snap
                        .citations
                        .iter()
                        .find(|c| c.number == cites[0])
                        .map(|c| c.snippet.clone())
                        .unwrap_or_default();
                    section_body.push_str(&format!("**{}** {} {markers}\n\n", sq.text, snippet_preview));
                }
                answered_ids.push(sq.id.clone());
            }
            if section_body.is_empty() {
                section_body.push_str("_No content for this section yet._\n\n");
            }
            handle.append_draft_section(DraftSection {
                heading: heading.clone(),
                body: section_body.clone(),
                answers_sub_questions: answered_ids,
            });
            body.push_str(&section_body);
        }

        handle.set_final_report(body);
        handle.push_transcript(NodeStep::new(
            NodeKind::Writer,
            "writer",
            format!("wrote {} sections", headings.len()),
        ));
        Ok(())
    }
}

fn title_of(query: &str) -> String {
    let t = query.trim();
    let mut chars = t.chars();
    match chars.next() {
        Some(c) => format!("{}{}", c.to_uppercase().next().unwrap_or(c), chars.as_str()),
        None => "Research report".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atomr_agents_deep_research_core::{Citation, Plan, ResearchRequest, ResearchResult, SubQuestion};
    use atomr_agents_web_search_core::MockWebSearch;
    use parking_lot::Mutex;
    use std::sync::Arc;
    use url::Url;

    #[tokio::test]
    async fn concat_writer_produces_markdown_with_citations() {
        let req = ResearchRequest::new("compare frameworks");
        let result = Arc::new(Mutex::new(ResearchResult::new(req.query.clone(), "test")));
        let h = ResearchHandle::new(result.clone(), Arc::new(req), Arc::new(MockWebSearch::new()));

        let mut plan = Plan::new();
        plan.outline = vec!["Background".into(), "Findings".into()];
        let mut sq1 = SubQuestion::new("sq-1", "what frameworks exist?");
        sq1.section = Some("Background".into());
        let mut sq2 = SubQuestion::new("sq-2", "how do they compare?");
        sq2.section = Some("Findings".into());
        plan.sub_questions.push(sq1);
        plan.sub_questions.push(sq2);
        h.set_plan(plan.clone());

        let mut c1 = Citation::new(1, Url::parse("https://a.test/").unwrap(), "A", "snip a");
        c1.supports.push("sq-1".into());
        let mut c2 = Citation::new(2, Url::parse("https://b.test/").unwrap(), "B", "snip b");
        c2.supports.push("sq-2".into());
        h.append_citation(c1);
        h.append_citation(c2);

        ConcatWriter.write(&plan, &h).await.unwrap();
        let report = h.snapshot().final_report.unwrap();
        assert!(report.contains("# Compare frameworks"));
        assert!(report.contains("## Background"));
        assert!(report.contains("[1]"));
        assert!(report.contains("[2]"));
        assert_eq!(h.snapshot().artifacts.drafts.len(), 2);
    }
}
