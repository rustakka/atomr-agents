//! Deterministic provenance / citation-coverage scoring (FR-9).
//!
//! Books-and-records and regulatory defensibility require that every
//! quantitative claim in a published thesis trace to a sourced document
//! — a retrieved [`EvidenceBundle`] with a valid `doc_hash`. This is a
//! deterministic, NON-LLM check, not a rubric/judge approximation.
//!
//! * [`EvidenceBundle`] is the retrieved-document record a citation
//!   points at, identified by a content [`compute_hash`].
//! * [`EvidenceIndex`] is the lookup the retriever zoo populates at
//!   retrieval time; [`InMemoryEvidenceIndex`] keeps the original
//!   content so a citation can be re-verified against a recomputed hash
//!   (tamper detection).
//! * [`ProvenanceScorer`] checks each extracted [`Claim`]: a claim is
//!   *covered* iff it carries a citation whose bundle exists AND whose
//!   stored content still hashes to the cited `doc_hash`. Coverage is
//!   the fraction of covered claims; the gate passes only when there are
//!   no uncovered or invalid claims.
//!
//! There is no LLM in the scoring path — given the same claim extractor
//! and index, the result is fully reproducible.

use std::collections::HashMap;

use atomr_agents_core::Value;
use serde::{Deserialize, Serialize};

use crate::scorer::{Scorer, ScorerOutcome};

/// Compute a stable content hash for evidence integrity checks.
///
/// We use the FNV-1a 64-bit hash (deterministic across runs and
/// machines, unlike `std`'s `DefaultHasher`/`RandomState`, which is
/// randomly seeded per process). The result is rendered as zero-padded
/// lowercase hex. FNV is *not* cryptographic — it detects accidental
/// drift / tampering of stored content, which is all this audit check
/// requires; it is not a defense against an adversary crafting
/// collisions.
pub fn compute_hash(content: &str) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for byte in content.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// A retrieved-document record a citation can point at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    /// Content hash (see [`compute_hash`]); the citation token.
    pub doc_hash: String,
    /// Where the document came from.
    pub source_uri: String,
    /// Unix timestamp (seconds) of retrieval.
    pub retrieved_at: i64,
    /// A short excerpt for human inspection.
    pub snippet: String,
}

/// A lookup of evidence bundles populated by the retriever zoo.
///
/// Integrity (does the stored source still hash to the cited
/// `doc_hash`?) is verified by the index itself, since only the index
/// holds the backing content — the scorer never sees raw documents.
pub trait EvidenceIndex {
    /// Fetch the bundle for `doc_hash`, if indexed.
    fn get(&self, doc_hash: &str) -> Option<&EvidenceBundle>;

    /// Whether `content` hashes to `doc_hash` — the integrity primitive.
    /// Callers that hold the candidate content (e.g. a re-retrieval)
    /// pass it directly.
    fn recompute_matches(&self, doc_hash: &str, content: &str) -> bool {
        compute_hash(content) == doc_hash
    }

    /// Whether the index's OWN stored content for `doc_hash` still
    /// recomputes to that hash. Stores that retain content (like
    /// [`InMemoryEvidenceIndex`]) detect tampering here; stores without
    /// content can only confirm presence and should document that.
    fn verify_stored(&self, doc_hash: &str) -> bool;
}

/// In-memory [`EvidenceIndex`] that retains each bundle's original
/// content so a citation can be re-verified against a recomputed hash.
#[derive(Default)]
pub struct InMemoryEvidenceIndex {
    // doc_hash -> (bundle, original content)
    bundles: HashMap<String, (EvidenceBundle, String)>,
}

impl InMemoryEvidenceIndex {
    /// New empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Index `content` from `source_uri`, computing the `doc_hash` and
    /// returning the resulting [`EvidenceBundle`] (whose `doc_hash` is
    /// the citation token to embed in claims).
    pub fn insert(
        &mut self,
        source_uri: impl Into<String>,
        retrieved_at: i64,
        content: impl Into<String>,
    ) -> EvidenceBundle {
        let content = content.into();
        let doc_hash = compute_hash(&content);
        let snippet: String = content.chars().take(120).collect();
        let bundle = EvidenceBundle {
            doc_hash: doc_hash.clone(),
            source_uri: source_uri.into(),
            retrieved_at,
            snippet,
        };
        self.bundles.insert(doc_hash, (bundle.clone(), content));
        bundle
    }

    /// Insert a pre-built bundle together with its backing content
    /// (e.g. when replaying recorded retrieval).
    pub fn insert_bundle(&mut self, bundle: EvidenceBundle, content: impl Into<String>) {
        self.bundles
            .insert(bundle.doc_hash.clone(), (bundle, content.into()));
    }

    /// Simulate tampering: replace the stored content for `doc_hash`
    /// without updating the hash, so [`EvidenceIndex::verify_stored`]
    /// will fail. Returns `true` if the hash was present.
    pub fn tamper(&mut self, doc_hash: &str, new_content: impl Into<String>) -> bool {
        if let Some(entry) = self.bundles.get_mut(doc_hash) {
            entry.1 = new_content.into();
            true
        } else {
            false
        }
    }
}

impl EvidenceIndex for InMemoryEvidenceIndex {
    fn get(&self, doc_hash: &str) -> Option<&EvidenceBundle> {
        self.bundles.get(doc_hash).map(|(b, _)| b)
    }

    fn verify_stored(&self, doc_hash: &str) -> bool {
        match self.bundles.get(doc_hash) {
            Some((_, content)) => compute_hash(content) == doc_hash,
            None => false,
        }
    }
}

/// A factual statement extracted from an agent's output, optionally
/// carrying a citation (the `doc_hash` of its backing evidence).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Claim {
    /// The claim text.
    pub text: String,
    /// `doc_hash` of the backing [`EvidenceBundle`], if cited.
    pub citation: Option<String>,
}

/// The provenance audit of one output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Fraction of claims with valid backing evidence, in `[0.0, 1.0]`.
    pub coverage: f64,
    /// Claims with no citation at all.
    pub uncovered: Vec<Claim>,
    /// Claims whose citation is missing from the index OR whose stored
    /// content no longer recomputes to the cited hash (tampered).
    pub invalid: Vec<Claim>,
}

/// Verifies that each cited claim in an output is backed by an
/// integrity-checked [`EvidenceBundle`]. Deterministic; no LLM.
///
/// `F` is the claim extractor — a pure function from the output `Value`
/// to the list of [`Claim`]s to verify (e.g. a regex/JSON walk that
/// pulls out quantitative statements and their citation tokens).
pub struct ProvenanceScorer<F, I>
where
    F: Fn(&Value) -> Vec<Claim>,
    I: EvidenceIndex,
{
    claim_extractor: F,
    evidence_index: I,
}

enum ClaimStatus {
    Covered,
    Uncovered,
    Invalid,
}

impl<F, I> ProvenanceScorer<F, I>
where
    F: Fn(&Value) -> Vec<Claim>,
    I: EvidenceIndex,
{
    /// Construct from a claim extractor and an evidence index.
    pub fn new(claim_extractor: F, evidence_index: I) -> Self {
        Self {
            claim_extractor,
            evidence_index,
        }
    }

    /// Classify a single claim as covered, uncovered (no citation), or
    /// invalid (citation missing/tampered).
    fn classify(&self, claim: &Claim) -> ClaimStatus {
        match &claim.citation {
            None => ClaimStatus::Uncovered,
            Some(doc_hash) => {
                // Bundle must exist AND its stored content must still
                // recompute to the cited hash.
                if self.evidence_index.get(doc_hash).is_some()
                    && self.evidence_index.verify_stored(doc_hash)
                {
                    ClaimStatus::Covered
                } else {
                    ClaimStatus::Invalid
                }
            }
        }
    }

    /// Full coverage audit of an output.
    pub fn coverage_report(&self, output: &Value) -> CoverageReport {
        let claims = (self.claim_extractor)(output);
        let total = claims.len();
        let mut uncovered = Vec::new();
        let mut invalid = Vec::new();
        let mut covered = 0usize;
        for claim in claims {
            match self.classify(&claim) {
                ClaimStatus::Covered => covered += 1,
                ClaimStatus::Uncovered => uncovered.push(claim),
                ClaimStatus::Invalid => invalid.push(claim),
            }
        }
        let coverage = if total == 0 {
            1.0
        } else {
            covered as f64 / total as f64
        };
        CoverageReport {
            coverage,
            uncovered,
            invalid,
        }
    }
}

impl<F, I> Scorer for ProvenanceScorer<F, I>
where
    F: Fn(&Value) -> Vec<Claim> + Send + Sync + 'static,
    I: EvidenceIndex + Send + Sync + 'static,
{
    fn score(&self, _expected: &Value, actual: &Value) -> ScorerOutcome {
        let report = self.coverage_report(actual);
        let passed = report.uncovered.is_empty() && report.invalid.is_empty();
        ScorerOutcome {
            passed,
            score: report.coverage as f32,
            note: format!(
                "coverage {:.4}: {} uncovered, {} invalid",
                report.coverage,
                report.uncovered.len(),
                report.invalid.len()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claim(text: &str, citation: Option<&str>) -> Claim {
        Claim {
            text: text.into(),
            citation: citation.map(|s| s.to_string()),
        }
    }

    // Extractor: read `claims` array of {text, citation?} from output.
    fn extract(output: &Value) -> Vec<Claim> {
        output
            .get("claims")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|c| Claim {
                        text: c.get("text").and_then(|t| t.as_str()).unwrap_or("").into(),
                        citation: c
                            .get("citation")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn compute_hash_is_stable_and_padded() {
        let h = compute_hash("hello world");
        assert_eq!(h, compute_hash("hello world"));
        assert_eq!(h.len(), 16);
        assert_ne!(h, compute_hash("hello worlD"));
    }

    #[test]
    fn full_coverage_when_all_cited_and_intact() {
        let mut index = InMemoryEvidenceIndex::new();
        let b = index.insert("uri://10k", 1_700_000_000, "revenue grew 12%");
        let scorer = ProvenanceScorer::new(extract, index);
        let output = json!({
            "claims": [{"text": "rev +12%", "citation": b.doc_hash}]
        });
        let report = scorer.coverage_report(&output);
        assert_eq!(report.coverage, 1.0);
        assert!(report.uncovered.is_empty());
        assert!(report.invalid.is_empty());
        assert!(scorer.score(&json!({}), &output).passed);
    }

    #[test]
    fn fails_on_missing_citation() {
        let index = InMemoryEvidenceIndex::new();
        let scorer = ProvenanceScorer::new(extract, index);
        let output = json!({
            "claims": [
                {"text": "rev +12%", "citation": null},
                {"text": "uncited claim"}
            ]
        });
        let report = scorer.coverage_report(&output);
        assert_eq!(report.coverage, 0.0);
        assert_eq!(report.uncovered.len(), 2);
        assert!(!scorer.score(&json!({}), &output).passed);
    }

    #[test]
    fn fails_on_unknown_citation() {
        let index = InMemoryEvidenceIndex::new();
        let scorer = ProvenanceScorer::new(extract, index);
        let output = json!({
            "claims": [{"text": "rev +12%", "citation": "deadbeefdeadbeef"}]
        });
        let report = scorer.coverage_report(&output);
        assert_eq!(report.coverage, 0.0);
        assert_eq!(report.invalid.len(), 1);
        assert!(!scorer.score(&json!({}), &output).passed);
    }

    #[test]
    fn fails_on_tampered_content() {
        let mut index = InMemoryEvidenceIndex::new();
        let b = index.insert("uri://10k", 1_700_000_000, "revenue grew 12%");
        let hash = b.doc_hash.clone();
        // Tamper: stored content no longer matches the recorded hash.
        assert!(index.tamper(&hash, "revenue grew 99%"));
        let scorer = ProvenanceScorer::new(extract, index);
        let output = json!({
            "claims": [{"text": "rev +12%", "citation": hash}]
        });
        let report = scorer.coverage_report(&output);
        assert_eq!(report.coverage, 0.0);
        assert_eq!(report.invalid.len(), 1);
        assert!(!scorer.score(&json!({}), &output).passed);
    }

    #[test]
    fn partial_coverage_number_correct() {
        let mut index = InMemoryEvidenceIndex::new();
        let a = index.insert("uri://a", 1, "alpha");
        let c = index.insert("uri://c", 3, "gamma");
        let cited_a = a.doc_hash.clone();
        let cited_c = c.doc_hash.clone();
        let scorer = ProvenanceScorer::new(extract, index);
        // 4 claims: 2 covered, 1 uncovered, 1 invalid => coverage 0.5
        let output = json!({
            "claims": [
                {"text": "a", "citation": cited_a},
                {"text": "c", "citation": cited_c},
                {"text": "no cite"},
                {"text": "bad", "citation": "0000000000000000"}
            ]
        });
        let report = scorer.coverage_report(&output);
        assert!((report.coverage - 0.5).abs() < 1e-9);
        assert_eq!(report.uncovered.len(), 1);
        assert_eq!(report.invalid.len(), 1);
        assert!(!scorer.score(&json!({}), &output).passed);
    }

    #[test]
    fn empty_claims_is_full_coverage() {
        let index = InMemoryEvidenceIndex::new();
        let scorer = ProvenanceScorer::new(extract, index);
        let report = scorer.coverage_report(&json!({"claims": []}));
        assert_eq!(report.coverage, 1.0);
        assert!(scorer.score(&json!({}), &json!({"claims": []})).passed);
    }

    #[test]
    fn recompute_matches_default_works() {
        let index = InMemoryEvidenceIndex::new();
        let h = compute_hash("doc body");
        assert!(index.recompute_matches(&h, "doc body"));
        assert!(!index.recompute_matches(&h, "other body"));
        // Silence unused-binding lint on `claim` helper across configs.
        let _ = claim("x", None);
    }
}
