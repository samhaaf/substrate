//! Reset-prose parsing + resolution + earliest-reset (ETA) selection.

use chrono::{NaiveDate, NaiveDateTime};
use substrate_ccw::reset::{earliest_reset, parse_reset};

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
}

#[test]
fn parses_dated_prose() {
    let r = parse_reset("Jul 25 at 1:59am (America/Chicago)").unwrap();
    assert_eq!(r.month, Some(7));
    assert_eq!(r.day, Some(25));
    assert_eq!(r.hour, 1);
    assert_eq!(r.minute, 59);
    assert_eq!(r.tz.as_deref(), Some("America/Chicago"));
}

#[test]
fn parses_time_only_prose() {
    let r = parse_reset("2:40am (America/Chicago)").unwrap();
    assert_eq!(r.month, None);
    assert_eq!(r.hour, 2);
    assert_eq!(r.minute, 40);
}

#[test]
fn parses_bare_hour_pm() {
    let r = parse_reset("1pm").unwrap();
    assert_eq!(r.hour, 13);
    assert_eq!(r.minute, 0);
    // 12pm -> noon, 12am -> midnight.
    assert_eq!(parse_reset("12pm").unwrap().hour, 12);
    assert_eq!(parse_reset("12am").unwrap().hour, 0);
}

#[test]
fn rejects_garbage() {
    assert!(parse_reset("").is_none());
    assert!(parse_reset("no time here").is_none());
}

#[test]
fn resolves_time_only_rolling_to_next_day() {
    let now = dt(2026, 7, 25, 15, 0); // 3pm
    // 2:40am today already passed → resolves to tomorrow.
    let r = parse_reset("2:40am (America/Chicago)").unwrap();
    assert_eq!(r.resolve(now).unwrap(), dt(2026, 7, 26, 2, 40));
    // 5pm is later today.
    let r2 = parse_reset("5pm").unwrap();
    assert_eq!(r2.resolve(now).unwrap(), dt(2026, 7, 25, 17, 0));
}

#[test]
fn resolves_dated_rolling_to_next_year() {
    let now = dt(2026, 7, 25, 15, 0);
    // Jan 3 already passed this year → next year.
    let r = parse_reset("Jan 3 at 9am (America/Chicago)").unwrap();
    assert_eq!(r.resolve(now).unwrap(), dt(2027, 1, 3, 9, 0));
}

#[test]
fn earliest_reset_picks_soonest() {
    let now = dt(2026, 7, 25, 15, 0);
    let candidates = vec![
        parse_reset("Jul 25 at 11pm (America/Chicago)").unwrap(),
        parse_reset("Jul 25 at 4pm (America/Chicago)").unwrap(),
        parse_reset("Jul 26 at 1am (America/Chicago)").unwrap(),
    ];
    let winner = earliest_reset(&candidates, now).unwrap();
    assert!(winner.contains("4pm"), "got {winner}");
}
