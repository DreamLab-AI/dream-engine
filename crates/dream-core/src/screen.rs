//! Optional **hypothesis screening** before the measured improvement loop
//! (the *Jev option*, item DE-01-S1-01).
//!
//! A night's measured loop is expensive: build a worktree, run every required
//! evaluator, judge the receipts. When a host has more candidate hypotheses
//! than it can afford to measure, this module lets a typed-decision service
//! *order* them (so the likeliest-to-matter ones get the budget) or *defer*
//! the ones that clearly do not address their own stated problem.
//!
//! # Authority stays in the gate
//!
//! Screening is advice about **where to spend measurement**, never a verdict
//! about a change:
//!
//! * It can only reorder and defer. A deferred hypothesis is kept, with a
//!   reason, for a later night; nothing is rejected, accepted or promoted here.
//! * The measured pairwise gate, fault treatment, protected-path guard,
//!   evaluator receipts and promotion rules are untouched and still decide
//!   every candidate that does run.
//! * The result is always a **partition** of the input ids: every input id
//!   appears exactly once, in either [`Screening::run_order`] or
//!   [`Screening::deferred`], and no id is invented. [`Screening::is_partition_of`]
//!   checks it, and the screen refuses (falls back) if a judge's output would
//!   break it.
//!
//! # Off is the identity
//!
//! The default screen, [`NotScreened`], builds no request and yields exactly
//! [`plan_unscreened`]: the input order, cut at the budget. The same plan is
//! produced when a judge is configured but errs, times out, answers
//! malformed, or answers below the acceptance threshold. The default
//! [`ScreenPolicy`] accepts nothing automatically: verdicts are recorded as
//! annotations but neither order nor deferral changes until thresholds fitted
//! on labelled data are configured.
//!
//! # No network
//!
//! This crate links no HTTP client. A host implements [`HypothesisScreen`] or
//! hands a transport closure to [`JsonScreen`]. The wire body is a System One
//! (`POST /v1/systemone`) request: per hypothesis, one `score` question for
//! evidence fit (ordered `criteria` list) and one `noul` question for whether
//! the proposed change addresses the stated problem. The question-set version
//! is [`QUESTION_SET`].
//!
//! # Example
//!
//! ```
//! use dream_core::screen::{
//!     plan_unscreened, screen_candidates, Hypothesis, NotScreened, ScreenPolicy,
//! };
//!
//! let hyps = vec![
//!     Hypothesis::new("h1", "Given slow parsing, cache the parse tree.", "profile: parse 80%", "add parse cache"),
//!     Hypothesis::new("h2", "Given flaky CI, rename a variable.", "ci: 3 timeouts", "rename var"),
//! ];
//! let off = screen_candidates(&NotScreened, &hyps, &[], Some(1), &ScreenPolicy::default());
//! assert_eq!(off, plan_unscreened(&hyps, Some(1)));
//! assert_eq!(off.run_order, ["h1"]);
//! assert_eq!(off.deferred[0].id, "h2");
//! ```

use std::collections::BTreeSet;
use std::fmt;

use serde_json::{json, Map, Value};

/// Version string of the question set built here. Log it with every call.
pub const QUESTION_SET: &str = "dream-engine.de-01-s1-01.v1";

/// Most hypotheses sent in one request; later ones keep their input position
/// after the screened head and are never reordered by the service.
pub const MAX_SCREENED: usize = 16;
/// Characters kept of each text field placed in a request.
pub const MAX_FIELD_CHARS: usize = 1200;
/// Evaluator descriptions listed in a request.
pub const MAX_EVALUATORS: usize = 12;

/// Why a screen produced nothing. Every variant means "use the unscreened
/// plan"; [`FallbackReason::as_str`] is the only thing to log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    /// The option is off (the [`NotScreened`] default).
    FlagOff,
    /// The host has no endpoint.
    NoEndpoint,
    /// The host's deadline elapsed.
    Timeout,
    /// The host could not reach the service.
    Transport,
    /// A non-2xx status.
    Http(u16),
    /// Unparsable body, missing answer, wrong type or out-of-range value.
    Malformed,
    /// A confidence below [`ScreenPolicy::min_confidence`].
    LowConfidence,
    /// The judge's output would have broken the partition invariant.
    InvariantBroken,
}

impl FallbackReason {
    /// Stable log token: `flag-off`, `no-endpoint`, `timeout`, `transport`,
    /// `http-<code>`, `malformed`, `low-confidence` or `invariant-broken`.
    #[must_use]
    pub fn as_str(&self) -> String {
        match self {
            Self::FlagOff => "flag-off".to_owned(),
            Self::NoEndpoint => "no-endpoint".to_owned(),
            Self::Timeout => "timeout".to_owned(),
            Self::Transport => "transport".to_owned(),
            Self::Http(c) => format!("http-{c}"),
            Self::Malformed => "malformed".to_owned(),
            Self::LowConfidence => "low-confidence".to_owned(),
            Self::InvariantBroken => "invariant-broken".to_owned(),
        }
    }
}

impl fmt::Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_str())
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// One candidate hypothesis, as the host holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hypothesis {
    /// Stable id; unique within a screening call.
    pub id: String,
    /// The hypothesis statement (for example the `Given ...` line).
    pub statement: String,
    /// The source problem evidence the hypothesis is about.
    pub problem_evidence: String,
    /// A short description of the proposed change.
    pub proposed_change: String,
}

impl Hypothesis {
    /// Construct a hypothesis.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        statement: impl Into<String>,
        problem_evidence: impl Into<String>,
        proposed_change: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            statement: statement.into(),
            problem_evidence: problem_evidence.into(),
            proposed_change: proposed_change.into(),
        }
    }
}

/// A registered evaluator, described for the model so it can judge whether a
/// change is the kind the evaluators could notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluatorBrief {
    /// Evaluator name.
    pub name: String,
    /// What it measures, in a sentence.
    pub description: String,
}

/// What the service said about one hypothesis.
#[derive(Clone, Debug, PartialEq)]
pub struct HypothesisVerdict {
    /// The hypothesis id this is about.
    pub id: String,
    /// Expected evidence-fit level in `[0, 3]` (3 = fits directly).
    pub fit: f64,
    /// Confidence of `fit`, in `[0, 1]`.
    pub confidence: f64,
    /// Probability that the proposed change addresses the stated problem.
    pub addresses: f64,
}

/// A built request over the screened head of the candidates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenRequest {
    head: Vec<Hypothesis>,
    evaluators: Vec<EvaluatorBrief>,
}

impl ScreenRequest {
    /// Build a request from the first [`MAX_SCREENED`] hypotheses.
    #[must_use]
    pub fn new(hypotheses: &[Hypothesis], evaluators: &[EvaluatorBrief]) -> Self {
        Self {
            head: hypotheses.iter().take(MAX_SCREENED).cloned().collect(),
            evaluators: evaluators.iter().take(MAX_EVALUATORS).cloned().collect(),
        }
    }

    /// The hypotheses this request covers, in input order.
    #[must_use]
    pub fn hypotheses(&self) -> &[Hypothesis] {
        &self.head
    }

    /// The System One body for `model`. Question IDs are bookkeeping only, so
    /// each instruction names the state record it judges (`hypotheses.h0`).
    #[must_use]
    pub fn to_system_one(&self, model: &str) -> Value {
        let mut hyps = Map::new();
        let mut questions = Map::new();
        for (i, h) in self.head.iter().enumerate() {
            hyps.insert(
                format!("h{i}"),
                json!({
                    "statement": clip(&h.statement, MAX_FIELD_CHARS),
                    "problem_evidence": clip(&h.problem_evidence, MAX_FIELD_CHARS),
                    "proposed_change": clip(&h.proposed_change, MAX_FIELD_CHARS),
                }),
            );
            questions.insert(
                format!("fit_{i}"),
                json!({
                    "type": "score",
                    "instructions": format!(
                        "How well does `hypotheses.h{i}.problem_evidence` support the claim made in `hypotheses.h{i}.statement`? Judge only the evidence given, not whether the change would work."
                    ),
                    "criteria": [
                        "The evidence contradicts the statement or is unrelated to it",
                        "The evidence is weakly related; the statement mostly goes beyond it",
                        "The evidence supports the statement in part",
                        "The evidence directly supports the statement"
                    ]
                }),
            );
            questions.insert(
                format!("addresses_{i}"),
                json!({
                    "type": "noul",
                    "instructions": format!(
                        "The change described in `hypotheses.h{i}.proposed_change` would address the problem shown in `hypotheses.h{i}.problem_evidence`, as opposed to an unrelated or cosmetic change."
                    ),
                    "criteria": {
                        "true": "The change targets the cause or symptom the evidence shows.",
                        "false": "The change is unrelated to the evidence, or only cosmetic."
                    }
                }),
            );
        }
        json!({
            "model": model,
            "state": {
                "hypotheses": Value::Object(hyps),
                "evaluators": self.evaluators.iter().map(|e| json!({
                    "name": clip(&e.name, 80),
                    "description": clip(&e.description, 300),
                })).collect::<Vec<_>>(),
            },
            "questions": Value::Object(questions),
        })
    }

    /// Parse a response into one verdict per hypothesis, in request order.
    ///
    /// # Errors
    /// [`FallbackReason::Malformed`] unless every `fit_i` (score in `[0, 3]`
    /// with a `[0, 1]` confidence) and `addresses_i` (noul in `[0, 1]`) is
    /// present with the right type.
    pub fn parse_response(&self, body: &Value) -> Result<Vec<HypothesisVerdict>, FallbackReason> {
        let answers = body
            .get("answers")
            .and_then(Value::as_object)
            .ok_or(FallbackReason::Malformed)?;
        let num = |a: &Value, key: &str, lo: f64, hi: f64| {
            a.get(key)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && (lo..=hi).contains(v))
                .ok_or(FallbackReason::Malformed)
        };
        self.head
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let f = answers
                    .get(&format!("fit_{i}"))
                    .ok_or(FallbackReason::Malformed)?;
                let a = answers
                    .get(&format!("addresses_{i}"))
                    .ok_or(FallbackReason::Malformed)?;
                if f.get("type").and_then(Value::as_str) != Some("score")
                    || a.get("type").and_then(Value::as_str) != Some("noul")
                {
                    return Err(FallbackReason::Malformed);
                }
                Ok(HypothesisVerdict {
                    id: h.id.clone(),
                    fit: num(f, "score", 0.0, 3.0)?,
                    confidence: num(f, "confidence", 0.0, 1.0)?,
                    addresses: num(a, "noul", 0.0, 1.0)?,
                })
            })
            .collect()
    }
}

/// A source of screening verdicts. Implemented by the host.
pub trait HypothesisScreen {
    /// Whether this screen will ever answer. When `false`, no request is
    /// built. Defaults to `true`.
    fn is_active(&self) -> bool {
        true
    }

    /// Judge the hypotheses of `request`.
    ///
    /// # Errors
    /// A [`FallbackReason`] when no verdicts could be obtained.
    fn screen(&self, request: &ScreenRequest) -> Result<Vec<HypothesisVerdict>, FallbackReason>;
}

/// The deterministic default: screening is off. Builds nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NotScreened;

impl HypothesisScreen for NotScreened {
    fn is_active(&self) -> bool {
        false
    }

    fn screen(&self, _request: &ScreenRequest) -> Result<Vec<HypothesisVerdict>, FallbackReason> {
        Err(FallbackReason::FlagOff)
    }
}

/// Adapter from a host transport closure to a [`HypothesisScreen`]. The
/// closure takes the request body and returns the response body or the reason
/// the transport failed; this crate opens no connection.
pub struct JsonScreen<F> {
    model: String,
    transport: F,
}

impl<F> JsonScreen<F>
where
    F: Fn(&Value) -> Result<Value, FallbackReason>,
{
    /// A screen that sends bodies for `model` through `transport`.
    pub fn new(model: impl Into<String>, transport: F) -> Self {
        Self {
            model: model.into(),
            transport,
        }
    }
}

impl<F> HypothesisScreen for JsonScreen<F>
where
    F: Fn(&Value) -> Result<Value, FallbackReason>,
{
    fn screen(&self, request: &ScreenRequest) -> Result<Vec<HypothesisVerdict>, FallbackReason> {
        let body = (self.transport)(&request.to_system_one(&self.model))?;
        request.parse_response(&body)
    }
}

/// Thresholds for acting on verdicts. The default acts on **nothing**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPolicy {
    /// Least `fit` confidence at which any verdict is acted on. `None`
    /// (default): verdicts are recorded as annotations, order and deferral
    /// are unchanged. Below it the whole screening falls back.
    pub min_confidence: Option<f64>,
    /// A hypothesis whose `addresses` probability is below this is ranked
    /// behind those that clear it.
    pub addresses_floor: f64,
    /// Defer a hypothesis only when `fit` is below this **and** `addresses`
    /// is below `addresses_floor`. `None` (default): never defer on judgement.
    pub defer_fit_below: Option<f64>,
}

impl Default for ScreenPolicy {
    fn default() -> Self {
        Self {
            min_confidence: None,
            addresses_floor: 0.5,
            defer_fit_below: None,
        }
    }
}

/// Why a hypothesis was not run tonight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeferReason {
    /// It fell beyond the evaluation budget.
    OverBudget,
    /// The screen judged it both poorly supported and off-target.
    Screened,
}

/// A hypothesis kept for a later night.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deferral {
    /// The hypothesis id.
    pub id: String,
    /// Why it was deferred.
    pub reason: DeferReason,
}

/// The outcome: which hypotheses to measure, in order, and which to hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Screening {
    /// Ids to run through the measured loop, in order.
    pub run_order: Vec<String>,
    /// Ids held for a later night; none is rejected.
    pub deferred: Vec<Deferral>,
    /// Recorded verdicts (empty unless a screen answered and was accepted or
    /// annotation-only).
    pub annotations: Vec<HypothesisVerdict>,
    /// Why the unscreened plan was used, when it was because of a failure.
    pub fallback: Option<FallbackReason>,
}

impl Screening {
    /// Whether `run_order` and `deferred` together contain exactly the ids of
    /// `input`, each once.
    #[must_use]
    pub fn is_partition_of(&self, input: &[Hypothesis]) -> bool {
        let mut seen = BTreeSet::new();
        let all = self
            .run_order
            .iter()
            .map(String::as_str)
            .chain(self.deferred.iter().map(|d| d.id.as_str()));
        let mut n = 0;
        for id in all {
            n += 1;
            if !seen.insert(id) {
                return false;
            }
        }
        n == input.len() && input.iter().all(|h| seen.contains(h.id.as_str()))
    }
}

/// The pre-change plan: input order, cut at `budget` (`None` = no cut). The
/// remainder is deferred as [`DeferReason::OverBudget`].
#[must_use]
pub fn plan_unscreened(hypotheses: &[Hypothesis], budget: Option<usize>) -> Screening {
    plan_from_order(
        hypotheses.iter().map(|h| h.id.clone()).collect(),
        Vec::new(),
        budget,
        Vec::new(),
        None,
    )
}

fn plan_from_order(
    order: Vec<String>,
    screened_out: Vec<String>,
    budget: Option<usize>,
    annotations: Vec<HypothesisVerdict>,
    fallback: Option<FallbackReason>,
) -> Screening {
    let cut = budget.map_or(order.len(), |b| b.min(order.len()));
    let mut deferred: Vec<Deferral> = screened_out
        .into_iter()
        .map(|id| Deferral {
            id,
            reason: DeferReason::Screened,
        })
        .collect();
    deferred.extend(order[cut..].iter().map(|id| Deferral {
        id: id.clone(),
        reason: DeferReason::OverBudget,
    }));
    Screening {
        run_order: order[..cut].to_vec(),
        deferred,
        annotations,
        fallback,
    }
}

/// Screen `hypotheses` and plan tonight's measured runs under `budget`.
///
/// Fail-open: an inactive screen, any [`FallbackReason`], a low-confidence
/// verdict, duplicate ids, or an output that would break the partition
/// invariant all yield [`plan_unscreened`] (with `fallback` set where a
/// failure, not the off state, was the cause).
#[must_use]
pub fn screen_candidates(
    screen: &dyn HypothesisScreen,
    hypotheses: &[Hypothesis],
    evaluators: &[EvaluatorBrief],
    budget: Option<usize>,
    policy: &ScreenPolicy,
) -> Screening {
    let unscreened = plan_unscreened(hypotheses, budget);
    if !screen.is_active() || hypotheses.len() < 2 {
        return unscreened;
    }
    let fail = |reason: FallbackReason| Screening {
        fallback: Some(reason),
        ..unscreened.clone()
    };
    let ids: BTreeSet<&str> = hypotheses.iter().map(|h| h.id.as_str()).collect();
    if ids.len() != hypotheses.len() {
        return fail(FallbackReason::InvariantBroken);
    }

    let request = ScreenRequest::new(hypotheses, evaluators);
    let verdicts = match screen.screen(&request) {
        Ok(v) => v,
        Err(r) => return fail(r),
    };
    // The service may not invent, drop or repeat ids.
    let expected: Vec<&str> = request.hypotheses().iter().map(|h| h.id.as_str()).collect();
    if verdicts.len() != expected.len()
        || verdicts
            .iter()
            .map(|v| v.id.as_str())
            .ne(expected.iter().copied())
    {
        return fail(FallbackReason::Malformed);
    }

    let Some(min) = policy.min_confidence else {
        // Nothing fitted: record, do not act.
        return Screening {
            annotations: verdicts,
            ..unscreened
        };
    };
    if verdicts.iter().any(|v| v.confidence < min) {
        return fail(FallbackReason::LowConfidence);
    }

    // Composite in code: clear the address floor first, then evidence fit;
    // stable, so ties keep input order. The unscreened tail stays in place.
    let mut head: Vec<&HypothesisVerdict> = verdicts.iter().collect();
    head.sort_by(|a, b| {
        let (ao, bo) = (
            a.addresses >= policy.addresses_floor,
            b.addresses >= policy.addresses_floor,
        );
        bo.cmp(&ao).then(
            b.fit
                .partial_cmp(&a.fit)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let mut order = Vec::new();
    let mut screened_out = Vec::new();
    for v in head {
        let off_target = v.addresses < policy.addresses_floor;
        if policy.defer_fit_below.is_some_and(|t| v.fit < t) && off_target {
            screened_out.push(v.id.clone());
        } else {
            order.push(v.id.clone());
        }
    }
    order.extend(hypotheses.iter().skip(MAX_SCREENED).map(|h| h.id.clone()));

    let plan = plan_from_order(order, screened_out, budget, verdicts, None);
    if plan.is_partition_of(hypotheses) {
        plan
    } else {
        fail(FallbackReason::InvariantBroken)
    }
}
