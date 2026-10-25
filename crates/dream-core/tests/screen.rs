//! The Jev option for dream-engine (DE-01-S1-01): hypothesis screening.
//!
//! Equivalence is asserted against `plan_unscreened`, the pre-change plan:
//! input order, cut at the budget. No network: the "service" is a closure.

use std::cell::Cell;

use dream_core::screen::{
    plan_unscreened, screen_candidates, DeferReason, EvaluatorBrief, FallbackReason, Hypothesis,
    HypothesisScreen, HypothesisVerdict, JsonScreen, NotScreened, ScreenPolicy, ScreenRequest,
    MAX_SCREENED, QUESTION_SET,
};
use serde_json::{json, Value};

fn hyps(n: usize) -> Vec<Hypothesis> {
    (0..n)
        .map(|i| {
            Hypothesis::new(
                format!("h{i}"),
                format!("Given problem {i}"),
                format!("evidence {i}"),
                format!("change {i}"),
            )
        })
        .collect()
}

fn evaluators() -> Vec<EvaluatorBrief> {
    vec![EvaluatorBrief {
        name: "tests".into(),
        description: "cargo test passes".into(),
    }]
}

/// A response giving hypothesis `i` fit `fit(i)`, confidence `conf`, addresses `addr(i)`.
fn response(n: usize, fit: &dyn Fn(usize) -> f64, conf: f64, addr: &dyn Fn(usize) -> f64) -> Value {
    let mut a = serde_json::Map::new();
    for i in 0..n {
        a.insert(
            format!("fit_{i}"),
            json!({"type":"score","score":fit(i),"confidence":conf,"legend":{},"probabilities":[]}),
        );
        a.insert(
            format!("addresses_{i}"),
            json!({"type":"noul","noul":addr(i)}),
        );
    }
    json!({"model":"jev-test","usage":{"input_tokens":1,"output_tokens":1},"answers":a})
}

fn acting() -> ScreenPolicy {
    ScreenPolicy {
        min_confidence: Some(0.5),
        defer_fit_below: Some(1.0),
        ..ScreenPolicy::default()
    }
}

// --- fallback equivalence ----------------------------------------------------

#[test]
fn every_failure_mode_equals_the_unscreened_plan() {
    let h = hyps(5);
    for budget in [None, Some(0), Some(2), Some(5), Some(99)] {
        let want = plan_unscreened(&h, budget);
        assert!(want.is_partition_of(&h));
        // Pre-change order is literally the input order.
        let expect_run: Vec<String> = h
            .iter()
            .take(budget.unwrap_or(5))
            .map(|x| x.id.clone())
            .collect();
        assert_eq!(want.run_order, expect_run);

        // Off.
        assert_eq!(
            screen_candidates(&NotScreened, &h, &evaluators(), budget, &acting()),
            want
        );

        // Transport-level failures.
        for reason in [
            FallbackReason::NoEndpoint,
            FallbackReason::Timeout,
            FallbackReason::Transport,
            FallbackReason::Http(500),
            FallbackReason::Http(429),
        ] {
            let r = reason.clone();
            let s = JsonScreen::new("m", move |_| Err(r.clone()));
            let got = screen_candidates(&s, &h, &evaluators(), budget, &acting());
            assert_eq!(got.fallback.as_ref(), Some(&reason));
            assert_eq!(
                (&got.run_order, &got.deferred),
                (&want.run_order, &want.deferred),
                "{reason}"
            );
            assert!(got.annotations.is_empty());
        }

        // Malformed bodies.
        let bad: Vec<Value> = vec![
            json!("junk"),
            json!({"answers":{}}),
            json!({"answers":{"fit_0":{"type":"noul","noul":0.5}}}),
            {
                let mut v = response(5, &|_| 2.0, 0.9, &|_| 0.9);
                v["answers"]["fit_3"]["score"] = json!(7.0);
                v
            },
            {
                let mut v = response(5, &|_| 2.0, 0.9, &|_| 0.9);
                v["answers"].as_object_mut().unwrap().remove("addresses_4");
                v
            },
            {
                let mut v = response(5, &|_| 2.0, 0.9, &|_| 0.9);
                v["answers"]["addresses_1"]["noul"] = json!(-0.1);
                v
            },
        ];
        for body in bad {
            let s = JsonScreen::new("m", move |_| Ok(body.clone()));
            let got = screen_candidates(&s, &h, &evaluators(), budget, &acting());
            assert_eq!(got.fallback, Some(FallbackReason::Malformed));
            assert_eq!(
                (&got.run_order, &got.deferred),
                (&want.run_order, &want.deferred)
            );
        }

        // Low confidence against a configured threshold.
        let s = JsonScreen::new("m", |_| {
            Ok(response(5, &|i| 3.0 - i as f64 * 0.5, 0.2, &|_| 0.9))
        });
        let got = screen_candidates(&s, &h, &evaluators(), budget, &acting());
        assert_eq!(got.fallback, Some(FallbackReason::LowConfidence));
        assert_eq!(
            (&got.run_order, &got.deferred),
            (&want.run_order, &want.deferred)
        );
    }
}

#[test]
fn the_default_policy_accepts_nothing_automatically() {
    let h = hyps(4);
    // A confident, order-reversing, deferral-inviting answer...
    let s = JsonScreen::new("m", |_| {
        Ok(response(4, &|i| i as f64 * 0.9, 1.0, &|i| {
            if i == 0 {
                0.0
            } else {
                0.9
            }
        }))
    });
    let got = screen_candidates(&s, &h, &[], Some(2), &ScreenPolicy::default());
    let want = plan_unscreened(&h, Some(2));
    // ...changes neither order nor deferral, but is recorded.
    assert_eq!(
        (&got.run_order, &got.deferred),
        (&want.run_order, &want.deferred)
    );
    assert_eq!(got.annotations.len(), 4);
    assert_eq!(got.fallback, None);
}

#[test]
fn off_builds_no_request() {
    struct Spy<'a>(&'a Cell<u32>);
    impl HypothesisScreen for Spy<'_> {
        fn is_active(&self) -> bool {
            false
        }
        fn screen(&self, _: &ScreenRequest) -> Result<Vec<HypothesisVerdict>, FallbackReason> {
            self.0.set(self.0.get() + 1);
            Err(FallbackReason::FlagOff)
        }
    }
    let c = Cell::new(0);
    let h = hyps(3);
    assert_eq!(
        screen_candidates(&Spy(&c), &h, &[], Some(1), &acting()),
        plan_unscreened(&h, Some(1))
    );
    assert_eq!(c.get(), 0);
    assert_eq!(
        plan_unscreened(&h, Some(1)).fallback,
        None,
        "off is not a failure"
    );
}

#[test]
fn a_misbehaving_service_cannot_break_the_partition() {
    struct Liar(Vec<HypothesisVerdict>);
    impl HypothesisScreen for Liar {
        fn screen(&self, _: &ScreenRequest) -> Result<Vec<HypothesisVerdict>, FallbackReason> {
            Ok(self.0.clone())
        }
    }
    let h = hyps(3);
    let v = |id: &str| HypothesisVerdict {
        id: id.into(),
        fit: 3.0,
        confidence: 1.0,
        addresses: 1.0,
    };
    for verdicts in [
        vec![v("h0"), v("h1")],                // dropped one
        vec![v("h0"), v("h1"), v("h1")],       // duplicate
        vec![v("h0"), v("h1"), v("invented")], // new id
        vec![v("h2"), v("h1"), v("h0")],       // reordered ids (must echo request order)
    ] {
        let got = screen_candidates(&Liar(verdicts), &h, &[], Some(2), &acting());
        assert_eq!(got.fallback, Some(FallbackReason::Malformed));
        assert_eq!(got.run_order, plan_unscreened(&h, Some(2)).run_order);
        assert!(got.is_partition_of(&h));
    }
    // Duplicate input ids are refused before any request is built.
    let dup = vec![hyps(1)[0].clone(), hyps(1)[0].clone()];
    let calls = Cell::new(0);
    let s = JsonScreen::new("m", |_| {
        calls.set(calls.get() + 1);
        Ok(json!({}))
    });
    let got = screen_candidates(&s, &dup, &[], None, &acting());
    assert_eq!(got.fallback, Some(FallbackReason::InvariantBroken));
    assert_eq!(calls.get(), 0);
}

// --- wire shape --------------------------------------------------------------

#[test]
fn request_matches_the_system_one_contract() {
    let h = hyps(2);
    let body = ScreenRequest::new(&h, &evaluators()).to_system_one("jev-latest");
    assert_eq!(
        body.as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["model", "questions", "state"]
            .map(String::from)
            .into_iter()
            .collect()
    );
    assert_eq!(body["model"], "jev-latest");
    let qs = body["questions"].as_object().unwrap();
    assert_eq!(qs.len(), 4);
    for i in 0..2 {
        let f = &qs[&format!("fit_{i}")];
        assert_eq!(f["type"], "score");
        assert_eq!(
            f["criteria"].as_array().unwrap().len(),
            4,
            "Score criteria is an ordered list"
        );
        assert!(f["instructions"]
            .as_str()
            .unwrap()
            .contains(&format!("`hypotheses.h{i}.statement`")));
        let a = &qs[&format!("addresses_{i}")];
        assert_eq!(a["type"], "noul");
        assert!(a["criteria"]["true"].is_string() && a["criteria"]["false"].is_string());
    }
    assert_eq!(
        body["state"]["hypotheses"]["h1"]["proposed_change"],
        "change 1"
    );
    assert_eq!(body["state"]["evaluators"][0]["name"], "tests");
    assert!(!QUESTION_SET.is_empty());
}

#[test]
fn requests_are_bounded() {
    let big = "z".repeat(5_000);
    let h: Vec<Hypothesis> = (0..40)
        .map(|i| Hypothesis::new(format!("h{i}"), &big, &big, &big))
        .collect();
    let b = ScreenRequest::new(
        &h,
        &vec![
            EvaluatorBrief {
                name: big.clone(),
                description: big.clone()
            };
            50
        ],
    )
    .to_system_one("m");
    assert_eq!(
        b["state"]["hypotheses"].as_object().unwrap().len(),
        MAX_SCREENED
    );
    assert_eq!(
        b["state"]["hypotheses"]["h0"]["statement"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        1200
    );
    assert_eq!(b["state"]["evaluators"].as_array().unwrap().len(), 12);
    assert_eq!(b["questions"].as_object().unwrap().len(), MAX_SCREENED * 2);
}

// --- accept path -------------------------------------------------------------

#[test]
fn confident_scores_order_the_budget_and_defer_off_target_ones() {
    let h = hyps(5);
    // Fit favours h3 > h1 > h2 > h4; h0 is weakly supported AND off-target;
    // h4 is weakly supported but on-target (ordered last, not deferred).
    let fit = |i: usize| [0.2, 2.5, 1.8, 2.9, 0.4][i];
    let addr = |i: usize| if i == 0 { 0.05 } else { 0.9 };
    let s = JsonScreen::new("m", move |_| Ok(response(5, &fit, 0.9, &addr)));
    let got = screen_candidates(&s, &h, &evaluators(), Some(3), &acting());
    assert_eq!(got.fallback, None);
    assert_eq!(
        got.run_order,
        ["h3", "h1", "h2"],
        "best three get the budget, in fit order"
    );
    let by: std::collections::HashMap<_, _> = got
        .deferred
        .iter()
        .map(|d| (d.id.as_str(), d.reason.clone()))
        .collect();
    assert_eq!(
        by["h0"],
        DeferReason::Screened,
        "off-target and unsupported: held, not rejected"
    );
    assert_eq!(
        by["h4"],
        DeferReason::OverBudget,
        "on-target but beyond the budget"
    );
    assert!(
        got.is_partition_of(&h),
        "every hypothesis is still accounted for"
    );
    assert_eq!(got.annotations.len(), 5);
}

#[test]
fn ties_keep_input_order_and_unscreened_tail_stays_in_place() {
    let h = hyps(MAX_SCREENED + 3);
    let s = JsonScreen::new("m", |_| Ok(response(MAX_SCREENED, &|_| 2.0, 0.9, &|_| 0.9)));
    let got = screen_candidates(&s, &h, &[], None, &acting());
    assert_eq!(
        got.run_order,
        plan_unscreened(&h, None).run_order,
        "all ties ⇒ identical order"
    );
    // Tail beyond the screened head keeps its position after the head.
    assert_eq!(&got.run_order[MAX_SCREENED..], ["h16", "h17", "h18"]);

    // A single candidate has nothing to order: no request is made.
    let one = hyps(1);
    let s = JsonScreen::new("m", |_| panic!("must not be called"));
    assert_eq!(
        screen_candidates(&s, &one, &[], Some(1), &acting()),
        plan_unscreened(&one, Some(1))
    );
}

#[test]
fn deferral_needs_both_weak_fit_and_off_target() {
    let h = hyps(3);
    // h0 weak fit but on-target; h1 strong fit but off-target; h2 both weak.
    let fit = |i: usize| [0.1, 2.9, 0.1][i];
    let addr = |i: usize| [0.9, 0.1, 0.1][i];
    let s = JsonScreen::new("m", move |_| Ok(response(3, &fit, 0.9, &addr)));
    let got = screen_candidates(&s, &h, &[], None, &acting());
    assert_eq!(
        got.deferred
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        ["h2"]
    );
    assert_eq!(
        got.run_order,
        ["h0", "h1"],
        "on-target first, then off-target by fit"
    );
}
