//! `--redact` promises that no user name survives in a path it prints. The CLI targets check
//! that end to end on an error text (`cli_ergonomics`, `json_schema_contracts`); this checks
//! the scrub itself: the user names are gone, and scrubbing again changes nothing, for a path
//! string and for the strings of a JSON value.

use rvt::redact::{redact_json_strings, redact_path_str};

#[test]
fn scrubbing_twice_gives_what_scrubbing_once_gives() {
    let once = redact_path_str("/home/alice/a.rvt and C:\\Users\\bob\\b.rvt");
    // A scrub that did nothing would pass the second assertion alone.
    assert!(!once.contains("alice") && !once.contains("bob"), "{once}");
    assert_eq!(redact_path_str(&once), once);
}

#[test]
fn scrubbing_a_value_twice_gives_what_scrubbing_once_gives() {
    let mut once = serde_json::json!([
        "/home/alice/a.rvt",
        { "path": "C:\\Users\\bob\\OneDrive - Autodesk\\m.rfa" },
    ]);
    redact_json_strings(&mut once);
    // A walk that did nothing would pass the last assertion alone.
    let text = once.to_string();
    assert!(!text.contains("alice") && !text.contains("bob"), "{text}");
    let mut twice = once.clone();
    redact_json_strings(&mut twice);
    assert_eq!(twice, once);
}
