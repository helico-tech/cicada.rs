# 0021. A key is one name: how an address is written

Date: 2026-11-11

## Status

Accepted

## Context

ADR 0013 addresses an actor by a type name and a key, written `type/key`. There a ref is a struct whose fields form
the key: one value, or several joined with `:`, each percent-encoded, so that a `:` is always a separator. A contract
lists the names of those fields, and the views of an actor and of a type show them.

ADR 0018 decides that a key is one name and that this supersedes the keys of several fields. It does not say how a
name is written in an address, what a ref looks like in code, or what becomes of the addresses and registrations that
exist. The change `one-name-keys` needs those rules, and they are a wire format that others must follow.

## Decision

- **A ref is written as one string, or as a unit.** What counts is what serde does with the ref: a struct of which
  serde writes one field, a string; a type that serde writes as a string itself, such as a newtype around a `String`
  or a type that names `String` as its serde form; or a unit struct, which is a singleton with the key `_`. Only a
  string counts: a number, a boolean, a single character and an enum variant are refused. The core knows no fields
  of a key, no field names and no field types. A caller that needs two parts joins them itself, or tells serde how
  to write its ref as one string.
- **A name is any text and is not empty.** The names `.` and `..` are refused.
- **In an address a name is percent-encoded.** The characters `A-Z a-z 0-9 . _ ~ -` and `:` stay as they are; every
  other character becomes `%XX` with upper-case hex digits. There is one spelling of each name.
- **`:` is an ordinary character of a name.** It separates nothing. An address that was written for a key of two
  fields, such as `account/acme:7`, stays valid and is the address of the one name `acme:7`.
- **A contract holds no key fields**, and they do not count in its hash. The view of an actor has no `params` and the
  view of a type has no `key`.
- **A database written before this is upgraded in place.** The migration removes every registration, and each node
  registers its types again when it starts. It rewrites `%3A` to `:` in the address columns, so that stored addresses
  have the one spelling.

## Consequences

- Every contract hash changes. Nodes before and after must not share a database: every old node is stopped first.
- Until a node that hosts a type starts on an upgraded database, that type is not registered. Its actors can be read
  and deleted and messages already in a mailbox wait; a create of one and a send to one from outside a turn are
  refused, and a turn on another node that creates one fails without a retry. A host whose nodes all host the same
  types has no such window.
- The core cannot tell the parts of a name apart: (`a:b`, `c`) and (`a`, `b:c`) are the same name `a:b:c`. A host
  that joins parts picks a separator that its parts cannot hold. Two old keys that become one name stop the
  migration.
- A ref that is not one string compiles. A node that hosts its type reports it at start; elsewhere a send to it
  fails at the call.
- A client of an earlier version cannot decode the views of a new node.
- This replaces these parts of ADR 0013: that the fields of a ref form the key and are joined with `:`; that a `:`
  is always a separator; that a contract holds the names of the key's fields; and `params` and `key` in the views.
  The rest of ADR 0013 stands: the form `type/key`, the rules for type names, the singleton key `_`, the HTTP paths
  and the listing by prefix.
