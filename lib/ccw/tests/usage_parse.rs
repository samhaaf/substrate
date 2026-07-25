//! `/usage` JSON + prose parsing (fixture built from cc-recon.md §5).

use substrate_ccw::usage::{parse_usage_json, parse_usage_prose};

const USAGE_JSON: &str = include_str!("fixtures/usage.json");

#[test]
fn parses_three_pools_from_fixture() {
    let snap = parse_usage_json(USAGE_JSON).expect("parse");
    assert_eq!(snap.pools.len(), 3, "session + week-all + week-Fable");
    assert_eq!(snap.session_id.as_deref(), Some("f597bc03-1234-5678-9abc-def012345678"));

    let session = snap.pools.iter().find(|p| p.is_session()).unwrap();
    assert_eq!(session.pct, 99.0);
    assert!(session.reset.is_some());

    let weekly_all = snap.weekly_all().unwrap();
    assert_eq!(weekly_all.pct, 73.0);
    assert_eq!(weekly_all.pool, "week (all models)");

    let fable = snap.pools.iter().find(|p| p.pool.contains("Fable")).unwrap();
    assert_eq!(fable.pct, 50.0);
    assert!(fable.is_weekly() && !fable.is_weekly_all());
}

#[test]
fn session_and_weekly_accessors() {
    let snap = parse_usage_json(USAGE_JSON).unwrap();
    assert_eq!(snap.session_pct(), Some(99.0));
    assert_eq!(snap.resets().len(), 3);
}

#[test]
fn prose_pool_labels_stored_as_reported() {
    let prose = "Current session: 12% used · resets Jul 25 at 1:59am (America/Chicago)\n\
                 Current week (all models): 8% used · resets Jul 26 at 12pm (America/Chicago)";
    let pools = parse_usage_prose(prose);
    assert_eq!(pools[0].pool, "session");
    assert_eq!(pools[1].pool, "week (all models)");
    assert_eq!(pools[0].pct, 12.0);
    assert_eq!(pools[1].pct, 8.0);
}

#[test]
fn tolerates_indentation_and_narrative() {
    let prose = "You are currently using your subscription\n\n   \
                 Current session: 5% used · resets 3pm (America/Chicago)\n   \
                 blah blah";
    let pools = parse_usage_prose(prose);
    assert_eq!(pools.len(), 1);
    assert_eq!(pools[0].pct, 5.0);
}
