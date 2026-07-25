//! Reset-time prose parsing.
//!
//! Claude Code surfaces limit reset times as LOCAL-TIME PROSE, not epochs
//! (confirmed in `cc-recon.md` §3/§5). Two surfaces produce it:
//!
//! * `/usage` rows — `resets Jul 25 at 1:59am (America/Chicago)`
//! * 429 synthetic messages — `resets 2:40am (America/Chicago)`
//!
//! We parse the prose into a structured [`ResetTime`] and resolve it to a
//! concrete instant relative to a reference "now" so budgets can pick the
//! earliest reset that unblocks (the ETA). We never fabricate a time — a string
//! that does not parse yields `None`.

use chrono::{Datelike, NaiveDate, NaiveDateTime};

/// A parsed reset descriptor. `raw` is preserved verbatim for display/ETA prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetTime {
    /// The verbatim descriptor, e.g. `Jul 25 at 1:59am (America/Chicago)`.
    pub raw: String,
    /// Month (1-12) if the prose named a date; `None` for time-only prose.
    pub month: Option<u32>,
    /// Day of month if named.
    pub day: Option<u32>,
    /// Hour in 24h form.
    pub hour: u32,
    /// Minute.
    pub minute: u32,
    /// IANA timezone label if present (kept as an opaque string).
    pub tz: Option<String>,
}

fn month_num(tok: &str) -> Option<u32> {
    let m = match tok.to_ascii_lowercase().as_str() {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    };
    Some(m)
}

/// Parse a `h:mm am/pm` / `h am/pm` clock token into 24h (hour, minute).
fn parse_clock(tok: &str) -> Option<(u32, u32)> {
    let lower = tok.to_ascii_lowercase();
    let (body, pm) = if let Some(b) = lower.strip_suffix("am") {
        (b, false)
    } else if let Some(b) = lower.strip_suffix("pm") {
        (b, true)
    } else {
        return None;
    };
    let body = body.trim();
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    if h > 12 || m > 59 {
        return None;
    }
    let hour24 = match (h % 12, pm) {
        (h12, false) => h12,      // 12am -> 0, 1am -> 1
        (h12, true) => h12 + 12,  // 12pm -> 12, 1pm -> 13
    };
    Some((hour24, m))
}

/// Parse a reset descriptor. Accepts (with or without an `at`, a date, or a tz):
/// `Jul 25 at 1:59am (America/Chicago)`, `2:40am (America/Chicago)`, `1pm`.
pub fn parse_reset(raw: &str) -> Option<ResetTime> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // Peel off a trailing "(tz)".
    let (head, tz) = match (raw.rfind('('), raw.rfind(')')) {
        (Some(o), Some(c)) if c > o => (
            raw[..o].trim().to_string(),
            Some(raw[o + 1..c].trim().to_string()),
        ),
        _ => (raw.to_string(), None),
    };

    // Tokenize; drop the connective "at".
    let toks: Vec<&str> = head
        .split_whitespace()
        .filter(|t| !t.eq_ignore_ascii_case("at"))
        .collect();

    let mut month = None;
    let mut day = None;
    let mut clock = None;

    for t in &toks {
        if clock.is_none() {
            if let Some(c) = parse_clock(t) {
                clock = Some(c);
                continue;
            }
        }
        if month.is_none() {
            if let Some(m) = month_num(t) {
                month = Some(m);
                continue;
            }
        }
        if month.is_some() && day.is_none() {
            if let Ok(d) = t.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>() {
                if (1..=31).contains(&d) {
                    day = Some(d);
                    continue;
                }
            }
        }
    }

    let (hour, minute) = clock?;
    Some(ResetTime {
        raw: raw.to_string(),
        month,
        day,
        hour,
        minute,
        tz,
    })
}

impl ResetTime {
    /// Resolve to a concrete local instant at or after `now`.
    ///
    /// * With a named date: use `now`'s year (rolling to next year if the
    ///   date already passed).
    /// * Time-only: use `now`'s date, rolling to the next day if the clock
    ///   time has already passed today.
    ///
    /// Timezone is treated as a label only — resolution is done in the same
    /// naive frame as `now` (all resets for a given account share a tz).
    pub fn resolve(&self, now: NaiveDateTime) -> Option<NaiveDateTime> {
        match (self.month, self.day) {
            (Some(mo), Some(d)) => {
                let mut year = now.year();
                let mut cand = NaiveDate::from_ymd_opt(year, mo, d)?
                    .and_hms_opt(self.hour, self.minute, 0)?;
                if cand < now {
                    year += 1;
                    cand = NaiveDate::from_ymd_opt(year, mo, d)?
                        .and_hms_opt(self.hour, self.minute, 0)?;
                }
                Some(cand)
            }
            _ => {
                let today = now.date();
                let cand = today.and_hms_opt(self.hour, self.minute, 0)?;
                if cand > now {
                    Some(cand)
                } else {
                    today
                        .succ_opt()?
                        .and_hms_opt(self.hour, self.minute, 0)
                }
            }
        }
    }
}

/// Pick the [`ResetTime`] that resolves earliest relative to `now`. Returns the
/// verbatim `raw` prose of the winner (for ETA display), or `None` if the input
/// is empty or nothing resolves.
pub fn earliest_reset(candidates: &[ResetTime], now: NaiveDateTime) -> Option<String> {
    candidates
        .iter()
        .filter_map(|r| r.resolve(now).map(|t| (t, r.raw.clone())))
        .min_by_key(|(t, _)| *t)
        .map(|(_, raw)| raw)
}
