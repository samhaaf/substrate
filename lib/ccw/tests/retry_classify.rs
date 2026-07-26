//! Retry classification (recon Deliverable 5): transient vs deterministic.

use substrate_ccw::retry::{classify_message, classify_result, RetryDecision, TransientKind};

#[test]
fn transient_messages_retry() {
    assert!(matches!(
        classify_message("You have hit your session limit resets 2:40am"),
        RetryDecision::Retry { kind: TransientKind::RateLimit, .. }
    ));
    assert!(matches!(
        classify_message("API Error: 529 Overloaded"),
        RetryDecision::Retry { kind: TransientKind::Overload, .. }
    ));
    assert!(matches!(
        classify_message("API Error: Unable to connect to API (ECONNRESET)"),
        RetryDecision::Retry { kind: TransientKind::ConnectionDrop, .. }
    ));
}

#[test]
fn expired_resume_restarts_fresh_not_retried() {
    assert!(matches!(
        classify_message("No conversation found with session ID abc"),
        RetryDecision::RestartFresh { .. }
    ));
}

#[test]
fn deterministic_ceilings_are_fatal() {
    assert!(matches!(classify_result("error_max_turns"), RetryDecision::Fatal { .. }));
    assert!(matches!(classify_result("error_max_budget_usd"), RetryDecision::Fatal { .. }));
    assert!(matches!(
        classify_result("error_max_structured_output_retries"),
        RetryDecision::Fatal { .. }
    ));
}

#[test]
fn error_during_execution_retries_once() {
    assert!(matches!(
        classify_result("error_during_execution"),
        RetryDecision::Retry { kind: TransientKind::ErrorDuringExecution, .. }
    ));
}

#[test]
fn unclassified_is_fatal() {
    assert!(matches!(classify_message("bad --flag typo"), RetryDecision::Fatal { .. }));
}
