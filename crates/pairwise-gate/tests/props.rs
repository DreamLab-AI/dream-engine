//! Property tests: symmetry, monotonicity, fault-independence.
use pairwise_gate::*;
use proptest::prelude::*;

fn outcome() -> impl Strategy<Value = Outcome> {
    prop_oneof![
        4 => (0u8..=4).prop_map(|x| Outcome::score(f64::from(x) / 4.0).unwrap()),
        1 => Just(Outcome::fault("down")),
    ]
}
fn pairings() -> impl Strategy<Value = Vec<Pairing>> {
    prop::collection::vec((outcome(), outcome()), 0..40).prop_map(|v| {
        v.into_iter()
            .map(|(b, c)| Pairing::new("t", b, c))
            .collect()
    })
}
fn scored() -> impl Strategy<Value = Vec<(u8, u8)>> {
    prop::collection::vec((0u8..=4, 0u8..=4), 1..40)
}
fn build(v: &[(u8, u8)]) -> Vec<Pairing> {
    v.iter()
        .map(|&(b, c)| {
            Pairing::new(
                "t",
                Outcome::score(f64::from(b)).unwrap(),
                Outcome::score(f64::from(c)).unwrap(),
            )
        })
        .collect()
}
fn lax() -> Guard {
    Guard::new(0, 1.0).unwrap()
}

proptest! {
    #[test]
    fn symmetry_swaps_wins_and_losses(v in pairings()) {
        let swapped: Vec<Pairing> = v.iter()
            .map(|p| Pairing::new(p.task.clone(), p.candidate.clone(), p.baseline.clone()))
            .collect();
        let a = Tally::from_pairings(&v);
        let b = Tally::from_pairings(&swapped);
        prop_assert_eq!((a.wins, a.losses, a.ties, a.faults), (b.losses, b.wins, b.ties, b.faults));
    }

    #[test]
    fn faults_never_change_wins_or_losses(v in scored(), extra in 0usize..10) {
        let base = build(&v);
        let mut with = base.clone();
        with.extend((0..extra).map(|_| Pairing::new("f", Outcome::fault("x"), Outcome::score(1.0).unwrap())));
        let a = Tally::from_pairings(&base);
        let b = Tally::from_pairings(&with);
        prop_assert_eq!((a.wins, a.losses, a.ties), (b.wins, b.losses, b.ties));
        prop_assert_eq!(b.faults, extra);
        // Floor is candidate-only, so a baseline fault legitimately adds a
        // candidate score there; the pairwise policies must not move.
        let pols: Vec<Box<dyn Policy>> = vec![
            Box::new(Margin::new(1)),
            Box::new(SignTest::new(0.05).unwrap()),
            Box::new(WilsonLower::new(1.96).unwrap()),
            Box::new(NoRegression::new(2.0).unwrap()),
        ];
        for p in pols {
            let x = p.decide(&Evidence::new(&base).with_guard(lax()));
            let y = p.decide(&Evidence::new(&with).with_guard(lax()));
            prop_assert_eq!(x.verdict, y.verdict);
        }
    }

    #[test]
    fn improving_candidate_is_monotone(v in scored(), idx in any::<prop::sample::Index>(), bump in 1u8..4) {
        let mut better = v.clone();
        let i = idx.index(v.len());
        better[i].1 = better[i].1.saturating_add(bump);
        let (a, b) = (build(&v), build(&better));
        let pols: Vec<Box<dyn Policy>> = vec![
            Box::new(Margin::new(1)),
            Box::new(SignTest::new(0.05).unwrap()),
            Box::new(WilsonLower::new(1.96).unwrap()),
            Box::new(NoRegression::new(2.0).unwrap()),
            Box::new(Floor::new(2.0).unwrap()),
        ];
        for p in pols {
            let x = p.decide(&Evidence::new(&a).with_guard(lax())).verdict;
            let y = p.decide(&Evidence::new(&b).with_guard(lax())).verdict;
            if x == Verdict::Select {
                prop_assert_eq!(y, Verdict::Select);
            }
        }
    }

    #[test]
    fn p_value_monotone_in_wins(n in 1usize..300, k in 0usize..300) {
        let k = k.min(n);
        let p0 = sign_test_p_value(k, n);
        prop_assert!((0.0..=1.0).contains(&p0));
        if k < n {
            prop_assert!(sign_test_p_value(k + 1, n) <= p0 * (1.0 + 1e-12));
        }
    }

    #[test]
    fn never_panics_on_arbitrary_input(v in pairings()) {
        let pols: Vec<Box<dyn Policy>> = vec![
            Box::new(Margin::new(0)),
            Box::new(Floor::new(0.5).unwrap()),
            Box::new(SignTest::new(0.5).unwrap()),
            Box::new(WilsonLower::new(0.0).unwrap()),
            Box::new(NoRegression::new(0.5).unwrap()),
            Box::new(TrainDev::new(Box::new(Margin::new(0)), Box::new(NoRegression::new(0.5).unwrap()))),
        ];
        for p in pols {
            let _ = p.decide(&Evidence::new(&v).with_heldout(&v));
        }
    }
}
