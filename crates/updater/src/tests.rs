use super::*;

#[test]
fn development_launches_do_not_use_the_production_feed() {
    assert!(skip_production_feed(FeedEligibility {
        isolated: true,
        fixture: false,
        debug: false,
        bundled: true,
        has_keys: true,
    }));
    assert!(skip_production_feed(FeedEligibility {
        isolated: false,
        fixture: false,
        debug: true,
        bundled: true,
        has_keys: true,
    }));
    assert!(skip_production_feed(FeedEligibility {
        isolated: false,
        fixture: false,
        debug: false,
        bundled: false,
        has_keys: true,
    }));
    assert!(skip_production_feed(FeedEligibility {
        isolated: false,
        fixture: false,
        debug: false,
        bundled: true,
        has_keys: false,
    }));
    assert!(!skip_production_feed(FeedEligibility {
        isolated: true,
        fixture: true,
        debug: true,
        bundled: false,
        has_keys: true,
    }));
    assert!(!skip_production_feed(FeedEligibility {
        isolated: false,
        fixture: false,
        debug: false,
        bundled: true,
        has_keys: true,
    }));
    assert!(!skip_production_feed(FeedEligibility {
        isolated: true,
        fixture: true,
        debug: false,
        bundled: false,
        has_keys: false,
    }));
}

#[test]
fn automatic_checks_default_on_when_sparkle_has_no_choice() {
    assert!(default_automatic_checks(None));
    assert!(default_automatic_checks(Some(true)));
    assert!(!default_automatic_checks(Some(false)));
}
