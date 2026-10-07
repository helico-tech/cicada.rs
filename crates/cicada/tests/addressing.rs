//! Percent-encoding, actor addresses and URI templates (Kotlin `AddressingTest`: `PercentEncodingTest`,
//! `ActorAddressTest`, `UriTemplateTest`).

use std::collections::BTreeMap;

use cicada::address::{ActorAddress, UriTemplate, percent};

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

// ---- percent-encoding ----

#[test]
fn unreserved_characters_are_kept() {
    let s = "AZaz09._~-";
    assert_eq!(percent::encode(s), s);
    assert_eq!(percent::decode(s).unwrap(), s);
}

#[test]
fn other_characters_are_encoded_as_upper_case_utf8_escapes() {
    assert_eq!(percent::encode("a b"), "a%20b");
    assert_eq!(percent::encode("/"), "%2F");
    assert_eq!(percent::encode("%"), "%25");
    assert_eq!(percent::encode("café"), "caf%C3%A9");
    assert_eq!(percent::encode("😀"), "%F0%9F%98%80");
    assert_eq!(percent::encode("{}"), "%7B%7D");
}

#[test]
fn decode_is_the_inverse_of_encode() {
    for s in ["a b", "x/y", "100%", "café", "😀", "{id}", "a+b=c&d", ""] {
        assert_eq!(percent::decode(&percent::encode(s)).unwrap(), s);
    }
    assert_eq!(percent::decode("%7e").unwrap(), "~");
}

#[test]
fn malformed_input_is_rejected() {
    for bad in ["a b", "%2", "%zz", "%C3"] {
        assert!(percent::decode(bad).is_err(), "{bad}");
    }
}

// ---- addresses ----

#[test]
fn parses_and_keeps_canonical_form() {
    let a = ActorAddress::parse("actor://sync/acme").unwrap();
    assert_eq!(a.segments(), ["sync", "acme"]);
    assert_eq!(a.as_str(), "actor://sync/acme");
    assert_eq!(a.to_string(), "actor://sync/acme");
}

#[test]
fn non_canonical_escapes_normalise_to_the_same_address() {
    let a = ActorAddress::parse("actor://a/%7e%41").unwrap();
    let b = ActorAddress::parse("actor://a/~A").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.as_str(), "actor://a/~A");
    assert_eq!(ActorAddress::parse("actor://a/b%2fc").unwrap().as_str(), "actor://a/b%2Fc");
}

#[test]
fn building_from_raw_segments_encodes_them() {
    let a = ActorAddress::of(["files", "a/b c"]).unwrap();
    assert_eq!(a.as_str(), "actor://files/a%2Fb%20c");
    assert_eq!(ActorAddress::parse(a.as_str()).unwrap().segments(), ["files", "a/b c"]);
}

#[test]
fn rejects_malformed_uris() {
    for bad in ["http://x", "actor://", "actor://a//b", "actor://a/", "actor://a b", "actor://a/%2"] {
        assert!(ActorAddress::parse(bad).is_err(), "{bad}");
        assert!(bad.parse::<ActorAddress>().is_err(), "{bad}");
    }
}

#[test]
fn http_path_mapping() {
    let a = ActorAddress::parse("actor://a/b%20c").unwrap();
    assert_eq!(a.to_http_path(), "/actors/a/b%20c");
    assert_eq!(ActorAddress::from_http_path("a/b%20c").unwrap(), a);
    // A last segment "messages" is escaped so it cannot be mistaken for the mailbox endpoint.
    let m = ActorAddress::parse("actor://a/messages").unwrap();
    assert_eq!(m.to_http_path(), "/actors/a/%6Dessages");
    assert_eq!(ActorAddress::from_http_path("a/%6Dessages").unwrap(), m);
    assert_eq!(ActorAddress::parse("actor://messages/b").unwrap().to_http_path(), "/actors/messages/b");
}

#[test]
fn serializes_as_string() {
    let a = ActorAddress::parse("actor://sync/acme").unwrap();
    assert_eq!(serde_json::to_string(&a).unwrap(), "\"actor://sync/acme\"");
    assert_eq!(serde_json::from_str::<ActorAddress>("\"actor://sync/acme\"").unwrap(), a);
}

#[test]
fn prefix_and_ordering() {
    let a = ActorAddress::parse("actor://sync/acme").unwrap();
    assert!(a.has_prefix("actor://sync/"));
    assert!(ActorAddress::parse("actor://a/b").unwrap() < ActorAddress::parse("actor://a/c").unwrap());
}

// ---- templates ----

fn t(s: &str) -> UriTemplate {
    UriTemplate::parse(s).unwrap()
}

#[test]
fn parses_literals_and_params() {
    let template = t("actor://sync/{tenant}/{job}");
    assert_eq!(template.param_names(), vec!["tenant", "job"]);
    assert_eq!(template.as_str(), "actor://sync/{tenant}/{job}");
    assert_eq!(template.shape(), "sync/{}/{}");
}

#[test]
fn matches_and_extracts_decoded_params() {
    let template = t("actor://sync/{tenant}");
    assert_eq!(template.matches(&ActorAddress::parse("actor://sync/a%20b").unwrap()), Some(params(&[("tenant", "a b")])));
    assert_eq!(template.matches(&ActorAddress::parse("actor://sync/a/b").unwrap()), None);
    assert_eq!(template.matches(&ActorAddress::parse("actor://other/a").unwrap()), None);
}

#[test]
fn literal_segments_may_be_percent_encoded() {
    let template = t("actor://my%20ns/{id}");
    assert_eq!(template.matches(&ActorAddress::of(["my ns", "1"]).unwrap()), Some(params(&[("id", "1")])));
}

#[test]
fn expand_builds_the_address() {
    let template = t("actor://sync/{tenant}/{job}");
    assert_eq!(
        template.expand(&params(&[("tenant", "acme"), ("job", "j/1")])).unwrap(),
        ActorAddress::parse("actor://sync/acme/j%2F1").unwrap()
    );
    assert!(template.expand(&params(&[("tenant", "acme")])).is_err());
    assert!(template.expand(&params(&[("tenant", "a"), ("job", "b"), ("x", "c")])).is_err());
}

#[test]
fn templates_overlap_when_some_address_matches_both() {
    let common = |a: &str, b: &str| t(a).common_address(&t(b)).map(|c| c.to_string());

    assert_eq!(common("actor://jobs/{id}", "actor://jobs/cleanup").as_deref(), Some("actor://jobs/cleanup"));
    assert_eq!(common("actor://jobs/{id}", "actor://jobs/{name}").as_deref(), Some("actor://jobs/id"), "identical shape");
    assert_eq!(common("actor://{a}/x", "actor://s/{b}").as_deref(), Some("actor://s/x"), "literals in different positions");
    assert_eq!(common("actor://jobs/{id}", "actor://jobs/{id}").as_deref(), Some("actor://jobs/id"), "an equal template");
    assert!(t("actor://{a}/x").overlaps(&t("actor://s/{b}")));

    assert_eq!(common("actor://jobs/{id}", "actor://jobs/{id}/steps/{n}"), None, "different segment counts");
    assert_eq!(common("actor://e2e/counter/{id}", "actor://e2e/slow/{id}"), None, "different literals, one namespace");
    assert_eq!(common("actor://{a}/x", "actor://{b}/y"), None);
    assert!(!t("actor://maintenance/cleanup").overlaps(&t("actor://maintenance/other")));
}

#[test]
fn rejects_malformed_templates() {
    for bad in ["actor://", "x://a", "actor://a/{}", "actor://a/{1x}", "actor://{a}/{a}", "actor://a/{b", "actor://a//b"] {
        assert!(UriTemplate::parse(bad).is_err(), "{bad}");
    }
}
