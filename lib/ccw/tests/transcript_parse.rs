//! Transcript usage extraction + 429 limit detection (fixture lines).

use substrate_ccw::transcript::{parse_limit_reset, parse_transcript};

const TRANSCRIPT: &str = include_str!("fixtures/transcript.jsonl");

#[test]
fn extracts_per_model_usage_from_result_event() {
    let s = parse_transcript(TRANSCRIPT);
    assert!(s.from_result_event, "prefers the authoritative modelUsage aggregate");
    assert_eq!(s.per_model.len(), 2);

    let sonnet = &s.per_model["claude-sonnet-5"];
    assert_eq!(sonnet.input_tokens, 4648);
    assert_eq!(sonnet.output_tokens, 189);
    assert_eq!(sonnet.cache_read_tokens, 23064);
    assert_eq!(sonnet.cache_creation_tokens, 5671);

    let opus = &s.per_model["claude-opus-4"];
    assert_eq!(opus.total_tokens(), 160);

    assert_eq!(s.total_input_tokens(), 4748);
    assert_eq!(s.total_output_tokens(), 239);
    assert_eq!(s.total_tokens(), 33732);
}

#[test]
fn cost_comes_from_result_total() {
    let s = parse_transcript(TRANSCRIPT);
    assert_eq!(s.cost_usd(), 0.4231);
    assert_eq!(s.session_id.as_deref(), Some("72c6566e-1111-2222-3333-444455556666"));
}

#[test]
fn detects_429_limit_hit_and_reset_prose() {
    let s = parse_transcript(TRANSCRIPT);
    assert!(s.limit_hit());
    assert_eq!(s.limit_hits.len(), 1);
    let hit = &s.limit_hits[0];
    assert!(hit.text.contains("session limit"));
    let reset = hit.reset.as_ref().expect("reset parsed");
    assert_eq!(reset.hour, 2);
    assert_eq!(reset.minute, 40);
    assert_eq!(reset.tz.as_deref(), Some("America/Chicago"));
}

#[test]
fn falls_back_to_summed_assistant_usage_without_result_event() {
    // Two assistant events, no result event → summed message.usage.
    let body = "\
{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"cache_read_input_tokens\":1,\"cache_creation_input_tokens\":0}}}\n\
{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-5\",\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}";
    let s = parse_transcript(body);
    assert!(!s.from_result_event);
    let sonnet = &s.per_model["claude-sonnet-5"];
    assert_eq!(sonnet.input_tokens, 15);
    assert_eq!(sonnet.output_tokens, 5);
}

#[test]
fn parse_limit_reset_variants() {
    let r = parse_limit_reset("You've hit your weekly limit · resets 8:40pm (America/Chicago)").unwrap();
    assert_eq!(r.hour, 20);
    assert_eq!(r.minute, 40);
}
