//! Percent-encoding, type names, actor addresses and typed refs.

use std::marker::PhantomData;

use cicada::address::{ActorAddress, ActorRef, RefBinding, TypeName, percent};
use serde::{Deserialize, Serialize};

// ---- percent-encoding ----

#[test]
fn unreserved_characters_and_the_colon_are_kept() {
    let s = "AZaz09._~-:";
    assert_eq!(percent::encode(s), s);
    assert_eq!(percent::decode(s).unwrap(), s);
}

#[test]
fn other_characters_are_encoded_as_upper_case_utf8_escapes() {
    assert_eq!(percent::encode("a b"), "a%20b");
    assert_eq!(percent::encode("/"), "%2F");
    assert_eq!(percent::encode("%"), "%25");
    assert_eq!(percent::encode("@"), "%40");
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
    assert_eq!(percent::decode("%3A").unwrap(), ":");
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
    assert_eq!(a.name(), "acme");
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
    // An encoded colon is not the one spelling: it reads as the colon.
    assert_eq!(ActorAddress::parse("a/x%3Ay").unwrap().as_str(), "a/x:y");
}

#[test]
fn a_name_holds_any_text_and_is_percent_encoded() {
    let a = ActorAddress::of(&name("users"), "jan@example.com").unwrap();
    assert_eq!(a.as_str(), "users/jan%40example.com");
    assert_eq!(a.name(), "jan@example.com");
}

/// Scenario: A value holds a separator.
#[test]
fn a_name_with_a_slash_and_a_colon_encodes_the_slash_and_keeps_the_colon() {
    let a = ActorAddress::of(&name("files"), "a/b:c").unwrap();
    assert_eq!(a.as_str(), "files/a%2Fb:c");
    assert_eq!(a.key(), "a%2Fb:c");
    assert_eq!(a.name(), "a/b:c");
    assert_eq!(ActorAddress::parse(a.as_str()).unwrap(), a);
    // A key never holds a raw slash.
    assert!(ActorAddress::parse("files/a/b").is_err());
}

/// Scenario: An address of an old key with two parts.
#[test]
fn an_address_of_an_old_key_with_two_parts_is_the_address_of_one_name() {
    let old = ActorAddress::parse("account/acme:7").unwrap();
    assert_eq!(old.name(), "acme:7");
    assert_eq!(Account { name: "acme:7".into() }.address().unwrap(), old);
    for colons in ["a/x:", "a/:x", "a/:", "a/x::y"] {
        assert_eq!(ActorAddress::parse(colons).unwrap().as_str(), colons);
    }
}

#[test]
fn the_singleton_key() {
    let a = ActorAddress::of(&name("cleanup"), ActorAddress::SINGLETON_KEY).unwrap();
    assert_eq!(a.as_str(), "cleanup/_");
    assert_eq!(a.key(), ActorAddress::SINGLETON_KEY);
    assert_eq!(ActorAddress::parse("cleanup/_").unwrap(), a);
}

/// Scenario: An invalid address.
#[test]
fn malformed_addresses_are_rejected() {
    for bad in ["", "sync", "sync/", "/x", "sync/a/b", "Sync/a", "a b/c", "a/b c", "a/%2", "a/.", "a/.."] {
        assert!(ActorAddress::parse(bad).is_err(), "{bad}");
        assert!(bad.parse::<ActorAddress>().is_err(), "{bad}");
    }
    assert!(ActorAddress::of(&name("x"), "").is_err());
}

#[test]
fn the_key_dot_or_dot_dot_is_refused() {
    for bad in ["a/.", "a/..", "a/%2E", "a/%2e%2e"] {
        assert!(ActorAddress::parse(bad).is_err(), "{bad}");
    }
    assert!(ActorAddress::of(&name("a"), ".").is_err());
    assert!(ActorAddress::of(&name("a"), "..").is_err());
    // Other keys that merely contain dots are fine.
    assert_eq!(ActorAddress::of(&name("a"), "..x").unwrap().as_str(), "a/..x");
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

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type")]
enum Msg {
    Run,
}

/// Declares `ActorRef` for `$ref` with the type name `$name`.
macro_rules! actor_ref {
    ($ref:ty, $name:literal) => {
        impl ActorRef for $ref {
            const TYPE: &'static str = $name;
            type Message = Msg;
        }
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Tenant {
    tenant: String,
}
actor_ref!(Tenant, "sync");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Account {
    name: String,
}
actor_ref!(Account, "account");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Pricing;
actor_ref!(Pricing, "pricing");

/// A ref with a field that serde skips, as Chorus's `Named<A>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Cron<A> {
    name: String,
    #[serde(skip)]
    _kind: PhantomData<A>,
}

#[derive(Debug, Clone, PartialEq)]
struct Nightly;

impl ActorRef for Cron<Nightly> {
    const TYPE: &'static str = "cron";
    type Message = Msg;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TenantId(String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Wrapped {
    tenant: TenantId,
}
actor_ref!(Wrapped, "sync");

/// A newtype ref.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Bare(String);
actor_ref!(Bare, "sync");

/// A ref with typed parts that names `String` as its serde form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
struct BatchItem {
    batch: String,
    index: i32,
}
actor_ref!(BatchItem, "batch-items");

impl From<BatchItem> for String {
    fn from(item: BatchItem) -> Self {
        format!("{}.{}", item.batch, item.index)
    }
}

impl TryFrom<String> for BatchItem {
    type Error = String;

    fn try_from(name: String) -> Result<Self, String> {
        let (batch, index) = name.rsplit_once('.').ok_or_else(|| format!("'{name}' is not batch.index"))?;
        Ok(Self { batch: batch.to_owned(), index: index.parse().map_err(|_| format!("'{index}' is not an index"))? })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TwoFields {
    batch: String,
    index: i32,
}
actor_ref!(TwoFields, "batch");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Num {
    id: i32,
}
actor_ref!(Num, "num");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Region {
    Eu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Regional {
    region: Region,
}
actor_ref!(Regional, "regional");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Letter {
    c: char,
}
actor_ref!(Letter, "letter");

/// A ref whose one field serde skips when it is empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkipEmpty {
    #[serde(skip_serializing_if = "String::is_empty")]
    name: String,
}
actor_ref!(SkipEmpty, "skip-empty");

/// A ref whose one field is an `Option` that serde skips when it is `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkipNone {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}
actor_ref!(SkipNone, "skip-none");

/// A ref whose one field serde never writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct NeverWritten {
    #[serde(skip_serializing)]
    #[allow(dead_code)] // read only by serde, which never writes it: the point of this ref
    name: String,
}
actor_ref!(NeverWritten, "never-written");

/// A struct with braces and no fields: a singleton.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Braces {}
actor_ref!(Braces, "braces");

/// A struct whose only field has `#[serde(skip)]`: a singleton, as in 0.5.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Marker<A> {
    #[serde(skip)]
    kind: PhantomData<A>,
}

impl ActorRef for Marker<Nightly> {
    const TYPE: &'static str = "marker";
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

/// Scenario: A ref with one field; a ref round trips through its address.
#[test]
fn a_ref_with_one_field_builds_its_address_and_reads_back() {
    let r = Tenant { tenant: "acme".into() };
    assert_eq!(r.address().unwrap().as_str(), "sync/acme");
    let binding = RefBinding::<Tenant>::new().unwrap();
    assert_eq!(binding.type_name(), &name("sync"));
    let odd = Tenant { tenant: "a b/c:é".into() };
    assert_eq!(binding.from_address(&binding.to_address(&odd).unwrap()).unwrap(), odd);
}

/// Scenario: A ref without fields.
#[test]
fn a_unit_struct_is_the_singleton() {
    assert_eq!(Pricing.address().unwrap().as_str(), "pricing/_");
    let binding = RefBinding::<Pricing>::new().unwrap();
    assert_eq!(binding.from_address(&ActorAddress::parse("pricing/_").unwrap()).unwrap(), Pricing);
    let other = binding.from_address(&ActorAddress::parse("pricing/other").unwrap()).unwrap_err();
    assert!(other.to_string().contains("singleton"), "{other}");
}

/// Scenario: A field that serde skips.
#[test]
fn a_field_that_serde_skips_does_not_count() {
    let r = Cron::<Nightly> { name: "nightly".into(), _kind: PhantomData };
    assert_eq!(r.address().unwrap().as_str(), "cron/nightly");
    let binding = RefBinding::<Cron<Nightly>>::new().expect("a node that hosts the type starts");
    assert_eq!(binding.from_address(&r.address().unwrap()).unwrap(), r);
}

/// Scenario: A field that is a newtype of a string.
#[test]
fn a_field_that_is_a_newtype_of_a_string() {
    let r = Wrapped { tenant: TenantId("acme".into()) };
    assert_eq!(r.address().unwrap().as_str(), "sync/acme");
    assert_eq!(RefBinding::<Wrapped>::new().unwrap().from_address(&r.address().unwrap()).unwrap(), r);
}

/// Scenario: A newtype ref.
#[test]
fn a_newtype_ref() {
    let r = Bare("acme".into());
    assert_eq!(r.address().unwrap().as_str(), "sync/acme");
    assert_eq!(RefBinding::<Bare>::new().unwrap().from_address(&r.address().unwrap()).unwrap(), r);
}

/// Scenarios: A ref with typed parts that writes itself as a string; A ref that joins two parts.
#[test]
fn a_ref_with_typed_parts_that_writes_itself_as_a_string() {
    let r = BatchItem { batch: "b1".into(), index: 7 };
    assert_eq!(r.address().unwrap().as_str(), "batch-items/b1.7");
    let binding = RefBinding::<BatchItem>::new().expect("a node that hosts the type starts");
    assert_eq!(binding.from_address(&ActorAddress::parse("batch-items/b1.7").unwrap()).unwrap(), r);
    // Its own reader refuses a name it does not accept.
    let refused = binding.from_address(&ActorAddress::parse("batch-items/b1").unwrap()).unwrap_err();
    assert!(refused.to_string().contains("not batch.index"), "{refused}");
}

/// Scenario: A ref with several fields; and its use, where it has no address.
#[test]
fn a_ref_with_several_fields_is_refused() {
    let e = RefBinding::<TwoFields>::new().err().unwrap().to_string();
    assert!(e.contains("TwoFields") && e.contains("the fields batch, index"), "{e}");
    let e = TwoFields { batch: "b1".into(), index: 7 }.address().unwrap_err().to_string();
    assert!(e.contains("TwoFields has no address") && e.contains("a second field index"), "{e}");
}

/// Scenarios: A ref with a number; A ref with an enum variant; a character is not a string either.
#[test]
fn a_ref_whose_field_is_not_a_string_is_refused() {
    let e = RefBinding::<Num>::new().err().unwrap().to_string();
    assert!(e.contains("Num") && e.contains("field id asks for a number"), "{e}");
    let e = Num { id: 7 }.address().unwrap_err().to_string();
    assert!(e.contains("Num has no address") && e.contains("a number for its field id"), "{e}");

    let e = RefBinding::<Regional>::new().err().unwrap().to_string();
    assert!(e.contains("Regional") && e.contains("field region asks for an enum"), "{e}");
    let e = Regional { region: Region::Eu }.address().unwrap_err().to_string();
    assert!(e.contains("an enum variant for its field region"), "{e}");

    let e = RefBinding::<Letter>::new().err().unwrap().to_string();
    assert!(e.contains("field c asks for a character"), "{e}");
    assert!(Letter { c: 'x' }.address().is_err());
}

#[test]
fn a_ref_whose_field_serde_skips_when_empty_has_no_address_then() {
    let e = SkipEmpty { name: String::new() }.address().unwrap_err().to_string();
    assert!(e.contains("SkipEmpty has no address") && e.contains("serde skips its field name"), "{e}");
    assert_eq!(SkipEmpty { name: "x".into() }.address().unwrap().as_str(), "skip-empty/x");
}

#[test]
fn a_ref_whose_option_serde_skips_when_none_has_no_address_then() {
    let e = SkipNone { name: None }.address().unwrap_err().to_string();
    assert!(e.contains("SkipNone has no address") && e.contains("serde skips its field name"), "{e}");
}

#[test]
fn a_ref_whose_field_serde_never_writes_has_no_address() {
    let e = NeverWritten { name: "x".into() }.address().unwrap_err().to_string();
    assert!(e.contains("NeverWritten has no address") && e.contains("writes none of the fields it declares"), "{e}");
}

/// Scenario: A struct that declares no field to serde.
#[test]
fn a_struct_with_braces_and_no_fields_is_the_singleton() {
    assert_eq!(Braces {}.address().unwrap().as_str(), "braces/_");
    let binding = RefBinding::<Braces>::new().expect("a node that hosts the type starts");
    assert_eq!(binding.from_address(&ActorAddress::parse("braces/_").unwrap()).unwrap(), Braces {});
    let other = binding.from_address(&ActorAddress::parse("braces/other").unwrap()).unwrap_err();
    assert!(other.to_string().contains("singleton"), "{other}");
}

#[test]
fn a_struct_whose_only_field_serde_skips_is_the_singleton() {
    let r = Marker::<Nightly> { kind: PhantomData };
    assert_eq!(r.address().unwrap().as_str(), "marker/_");
    let binding = RefBinding::<Marker<Nightly>>::new().expect("a node that hosts the type starts");
    assert_eq!(binding.from_address(&ActorAddress::parse("marker/_").unwrap()).unwrap(), r);
    let other = binding.from_address(&ActorAddress::parse("marker/other").unwrap()).unwrap_err();
    assert!(other.to_string().contains("singleton"), "{other}");
}

#[test]
fn an_empty_name_has_no_address() {
    let e = Tenant { tenant: String::new() }.address().unwrap_err().to_string();
    assert!(e.contains("Tenant has no address") && e.contains("empty"), "{e}");
}

#[test]
fn an_address_of_another_type_is_refused() {
    let tenant = RefBinding::<Tenant>::new().unwrap();
    let other = ActorAddress::parse("other/acme").unwrap();
    assert!(tenant.from_address(&other).is_err());
}

#[test]
fn a_ref_with_a_malformed_type_name_has_no_binding() {
    assert!(RefBinding::<Misnamed>::new().is_err());
    assert!(Misnamed { id: "x".into() }.address().is_err());
}
