//! Zero-cost `/usage` introspection parsing.
//!
//! `claude -p "/usage" --output-format json` returns a `result` event whose
//! `result` string is human prose (confirmed in `cc-recon.md` §5, `num_turns:0`,
//! zero cost). We parse the JSON envelope, then the prose rows:
//!
//! ```text
//! Current session: 99% used · resets Jul 25 at 1:59am (America/Chicago)
//! Current week (all models): 73% used · resets Jul 25 at 12:59pm (America/Chicago)
//! Current week (Fable): 50% used · resets Jul 25 at 1pm (America/Chicago)
//! ```
//!
//! Each row becomes a [`PoolObservation`]. Pool identity is stored **as-reported**
//! (`session`, `week (all models)`, `week (Fable)`) — we deliberately do NOT
//! collapse to a hardcoded enum, since accounts expose different model-scoped
//! pools (`cc-recon.md` §5: pool names live in the binary but vary per account).

use serde::Deserialize;

use crate::reset::{parse_reset, ResetTime};

/// One parsed `/usage` pool row.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolObservation {
    /// As-reported pool label with the leading `Current ` stripped, e.g.
    /// `session`, `week (all models)`, `week (Fable)`.
    pub pool: String,
    /// Percent of this pool consumed (0-100).
    pub pct: f64,
    /// The parsed reset descriptor, if the row carried one.
    pub reset: Option<ResetTime>,
}

impl PoolObservation {
    /// True for the rolling 5-hour session pool (the session-backoff axis).
    pub fn is_session(&self) -> bool {
        self.pool.eq_ignore_ascii_case("session")
    }

    /// True for any weekly pool (all-models or a model-scoped weekly pool).
    pub fn is_weekly(&self) -> bool {
        self.pool.to_ascii_lowercase().starts_with("week")
    }

    /// True for the all-models weekly pool specifically.
    pub fn is_weekly_all(&self) -> bool {
        let p = self.pool.to_ascii_lowercase();
        p.starts_with("week") && p.contains("all")
    }
}

/// A full parsed `/usage` snapshot for one account.
#[derive(Debug, Clone, Default)]
pub struct UsageSnapshot {
    pub pools: Vec<PoolObservation>,
    /// The `/usage` run's own session id (so callers can purge the tiny session).
    pub session_id: Option<String>,
}

impl UsageSnapshot {
    /// The session pool percent, if reported.
    pub fn session_pct(&self) -> Option<f64> {
        self.pools.iter().find(|p| p.is_session()).map(|p| p.pct)
    }

    /// The all-models weekly pool, if reported.
    pub fn weekly_all(&self) -> Option<&PoolObservation> {
        self.pools.iter().find(|p| p.is_weekly_all())
    }

    /// All reset descriptors present (for ETA selection).
    pub fn resets(&self) -> Vec<ResetTime> {
        self.pools.iter().filter_map(|p| p.reset.clone()).collect()
    }
}

#[derive(Deserialize)]
struct UsageEnvelope {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
}

/// Parse the full JSON output of `claude -p "/usage" --output-format json`.
pub fn parse_usage_json(raw: &str) -> Result<UsageSnapshot, String> {
    // The stream may prepend log noise; find the JSON object.
    let json_start = raw.find('{').ok_or("no JSON object in /usage output")?;
    let env: UsageEnvelope = serde_json::from_str(raw[json_start..].trim())
        .map_err(|e| format!("parsing /usage JSON: {e}"))?;
    let prose = env.result.unwrap_or_default();
    let pools = parse_usage_prose(&prose);
    Ok(UsageSnapshot {
        pools,
        session_id: env.session_id,
    })
}

/// Parse the prose body (the `result` string) into pool rows. Tolerant of
/// leading indentation and surrounding narrative lines.
pub fn parse_usage_prose(prose: &str) -> Vec<PoolObservation> {
    let mut out = Vec::new();
    for line in prose.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("Current ") else {
            continue;
        };
        // label up to the first ':'
        let Some((label, tail)) = rest.split_once(':') else {
            continue;
        };
        let tail = tail.trim();
        // percent: digits immediately before "% used"
        let Some(pct_idx) = tail.find('%') else {
            continue;
        };
        let pct_str: String = tail[..pct_idx]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let Ok(pct) = pct_str.parse::<f64>() else {
            continue;
        };
        // reset: everything after "resets "
        let reset = tail
            .find("resets ")
            .map(|i| tail[i + "resets ".len()..].trim())
            .and_then(parse_reset);
        out.push(PoolObservation {
            pool: label.trim().to_string(),
            pct,
            reset,
        });
    }
    out
}
