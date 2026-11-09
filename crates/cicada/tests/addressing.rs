//! Percent-encoding, type names, actor addresses and typed refs.

use cicada::address::{ActorAddress, ActorRef, RefBinding, TypeName, percent};
use serde::{Deserialize, Serialize};

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
    assert_eq!(percent::encode(":"), "%3A");
    assert_eq!(percent::encode("%"), "%25");
    assert_eq!(percent::encode("café"), "caf%C3%A9");
    assert_eq!(percent::encode("😀"), "%F0%9F%98%80");
    assert_eq!(percent::encode("{}"), "%7B%7D");
}

#[test]
fn decode_is_the_inverse_of_encode() {
    for s in ["a b", "x/y", "a:b", "100%", "café", "😀", "{id}", "a+b=c&d", ""] {
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

// ---- type names ----

#[test]
fn a_type_name_is_lower_case_letters_digits_dots_underscores_and_dashes() {
    for good in ["sync", "billing.v2.invoices", "a-b_c", "x1", &"a".repeat(TypeName::MAX_LENGTH)] {
        let name = TypeName::parse(good).unwrap();
        assert_eq!(name.as_str(), good);
        assert_eq!(name.to_string(), good);
        assert_eq!(good.parse::<TypeName>().unwrap(), name);
    }
    let too_long = "a".repeat(TypeName::MAX_LENGTH + 1);
    for bad in ["", "Sync", "1a", ".a", "-a", "_a", "a/b", "a b", "a:b", "é", too_long.as_str()] {
        assert!(TypeName::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_type_name_serializes_as_its_string_and_is_checked_on_the_way_in() {
    let name = TypeName::parse("sync").unwrap();
    assert_eq!(serde_json::to_string(&name).unwrap(), "\"sync\"");
    assert_eq!(serde_json::from_str::<TypeName>("\"sync\"").unwrap(), name);
    assert!(serde_json::from_str::<TypeName>("\"Sync\"").is_err());
}

#[test]
fn type_names_are_ordered_by_text() {
    assert!(TypeName::parse("a").unwrap() < TypeName::parse("a.b").unwrap());
    assert!(TypeName::parse("a.b").unwrap() < TypeName::parse("b").unwrap());
}

// ---- addresses ----

fn name(n: &str) -> TypeName {
    TypeName::parse(n).unwrap()
}

#[test]
fn parses_and_keeps_canonical_form() {
    let a = ActorAddress::parse("sync/acme").unwrap();
    assert_eq!(a.type_name(), &name("sync"));
    assert_eq!(a.key(), "acme");
    assert_eq!(a.key_values(), ["acme"]);
    assert_eq!(a.as_str(), "sync/acme");
    assert_eq!(a.to_string(), "sync/acme");
    assert_eq!("sync/acme".parse::<ActorAddress>().unwrap(), a);
}

#[test]
fn non_canonical_escapes_normalise_to_the_same_address() {
    let a = ActorAddress::parse("a/%7e%41").unwrap();
    let b = ActorAddress::parse("a/~A").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.as_str(), "a/~A");
    assert_eq!(ActorAddress::parse("a/b%2fc").unwrap().as_str(), "a/b%2Fc");
}

#[test]
fn several_key_values_are_joined_with_a_colon() {
    let a = ActorAddress::parse("b/x:7").unwrap();
    assert_eq!(a.key(), "x:7");
    assert_eq!(a.key_values(), ["x", "7"]);
    assert_eq!(ActorAddress::of(&name("b"), ["x", "7"]).unwrap(), a);
}

#[test]
fn a_value_with_a_slash_or_a_colon_is_encoded() {
    let a = ActorAddress::of(&name("files"), ["a/b c", "d:e"]).unwrap();
    assert_eq!(a.as_str(), "files/a%2Fb%20c:d%3Ae");
    assert_eq!(a.key_values(), ["a/b c", "d:e"]);
    assert_eq!(ActorAddress::parse(a.as_str()).unwrap(), a);
    // A key never holds a raw slash.
    assert!(ActorAddress::parse("files/a/b").is_err());
}

#[test]
fn no_key_values_make_the_singleton() {
    let a = ActorAddress::of(&name("cleanup"), Vec::<String>::new()).unwrap();
    assert_eq!(a.as_str(), "cleanup/_");
    assert_eq!(a.key(), ActorAddress::SINGLETON_KEY);
    assert_eq!(ActorAddress::parse("cleanup/_").unwrap(), a);
}

#[test]
fn malformed_addresses_are_rejected() {
    for bad in ["", "sync", "sync/", "/x", "sync/a/b", "sync/a:", "sync/:a", "Sync/a", "a b/c", "a/b c", "a/%2"] {
        assert!(ActorAddress::parse(bad).is_err(), "{bad}");
        assert!(bad.parse::<ActorAddress>().is_err(), "{bad}");
    }
    assert!(ActorAddress::of(&name("x"), [""]).is_err());
}

#[test]
fn the_key_dot_or_dot_dot_is_refused() {
    for bad in ["a/.", "a/..", "a/%2E", "a/%2e%2e"] {
        assert!(ActorAddress::parse(bad).is_err(), "{bad}");
    }
    assert!(ActorAddress::of(&name("a"), ["."]).is_err());
    assert!(ActorAddress::of(&name("a"), [".."]).is_err());
    // Other keys that merely contain dots are fine.
    assert_eq!(ActorAddress::of(&name("a"), ["..x"]).unwrap().as_str(), "a/..x");
}

#[test]
fn serializes_as_string() {
    let a = ActorAddress::parse("sync/acme").unwrap();
    assert_eq!(serde_json::to_string(&a).unwrap(), "\"sync/acme\"");
    assert_eq!(serde_json::from_str::<ActorAddress>("\"sync/acme\"").unwrap(), a);
    assert!(serde_json::from_str::<ActorAddress>("\"actor://sync/acme\"").is_err());
}

#[test]
fn prefix_and_ordering_follow_the_text() {
    let a = ActorAddress::parse("sync/acme").unwrap();
    assert!(a.has_prefix("sync/"));
    assert!(a.has_prefix("sync/ac"));
    assert!(a.has_prefix(""));
    assert!(!a.has_prefix("sync/b"));
    assert!(!a.has_prefix("syn/c"));
    assert!(ActorAddress::parse("a/b").unwrap() < ActorAddress::parse("a/c").unwrap());
    // Ordering is by the text, not by type name and then key: `.` sorts before `/`.
    assert!(ActorAddress::parse("a.b/x").unwrap() < ActorAddress::parse("a/y").unwrap());

    let mut addresses: Vec<ActorAddress> =
        ["b/1", "a/z", "a/a:2", "a/a", "a.b/x"].iter().map(|s| ActorAddress::parse(s).unwrap()).collect();
    addresses.sort();
    let texts: Vec<&str> = addresses.iter().map(ActorAddress::as_str).collect();
    let mut expected = texts.clone();
    expected.sort();
    assert_eq!(texts, expected, "sorted by the text form");
}

// ---- typed refs ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Tenant {
    tenant: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type")]
enum Msg {
    Run,
}

impl ActorRef for Tenant {
    const TYPE: &'static str = "sync";
    type Message = Msg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Batch {
    tenant: String,
    n: u32,
}

impl ActorRef for Batch {
    const TYPE: &'static str = "billing.batch";
    type Message = Msg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Cleanup;

impl ActorRef for Cleanup {
    const TYPE: &'static str = "cleanup";
    type Message = Msg;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Misnamed {
    id: String,
}

impl ActorRef for Misnamed {
    const TYPE: &'static str = "Not A Name";
    type Message = Msg;
}

#[test]
fn a_ref_builds_its_address() {
    assert_eq!(Tenant { tenant: "acme".into() }.address().unwrap().as_str(), "sync/acme");
    assert_eq!(Batch { tenant: "b1".into(), n: 7 }.address().unwrap().as_str(), "billing.batch/b1:7");
    assert_eq!(Cleanup.address().unwrap().as_str(), "cleanup/_");
}

#[test]
fn a_ref_value_with_separators_is_encoded_in_its_address() {
    let r = Batch { tenant: "a/b:c".into(), n: 1 };
    let address = r.address().unwrap();
    assert_eq!(address.as_str(), "billing.batch/a%2Fb%3Ac:1");
    assert_eq!(RefBinding::<Batch>::new().unwrap().from_address(&address).unwrap(), r);
}

#[test]
fn a_ref_round_trips_through_its_address() {
    let tenant = RefBinding::<Tenant>::new().unwrap();
    let r = Tenant { tenant: "a b".into() };
    assert_eq!(tenant.from_address(&tenant.to_address(&r).unwrap()).unwrap(), r);

    let batch = RefBinding::<Batch>::new().unwrap();
    let r = Batch { tenant: "x".into(), n: 42 };
    assert_eq!(batch.from_address(&batch.to_address(&r).unwrap()).unwrap(), r);

    let cleanup = RefBinding::<Cleanup>::new().unwrap();
    assert_eq!(cleanup.from_address(&cleanup.to_address(&Cleanup).unwrap()).unwrap(), Cleanup);
}

#[test]
fn a_binding_knows_its_type_name_and_key_fields() {
    let batch = RefBinding::<Batch>::new().unwrap();
    assert_eq!(batch.type_name(), &name("billing.batch"));
    assert_eq!(batch.key_fields(), ["tenant", "n"]);
    assert!(RefBinding::<Cleanup>::new().unwrap().key_fields().is_empty());
}

#[test]
fn an_address_of_another_type_is_refused() {
    let tenant = RefBinding::<Tenant>::new().unwrap();
    let other = ActorAddress::parse("other/acme").unwrap();
    assert!(tenant.from_address(&other).is_err());
}

#[test]
fn an_address_with_the_wrong_number_of_key_values_is_refused() {
    let tenant = RefBinding::<Tenant>::new().unwrap();
    let batch = RefBinding::<Batch>::new().unwrap();
    assert!(tenant.from_address(&ActorAddress::parse("sync/a:b").unwrap()).is_err());
    assert!(batch.from_address(&ActorAddress::parse("billing.batch/a").unwrap()).is_err());
    assert!(batch.from_address(&ActorAddress::parse("billing.batch/a:1:2").unwrap()).is_err());
    // A key that does not decode as the field type.
    assert!(batch.from_address(&ActorAddress::parse("billing.batch/a:x").unwrap()).is_err());
    // A singleton has the key `_` only.
    let cleanup = RefBinding::<Cleanup>::new().unwrap();
    assert!(cleanup.from_address(&ActorAddress::parse("cleanup/other").unwrap()).is_err());
}

#[test]
fn a_ref_with_a_malformed_type_name_has_no_binding() {
    assert!(RefBinding::<Misnamed>::new().is_err());
    assert!(Misnamed { id: "x".into() }.address().is_err());
}
