# snmp-core

The SNMP engine of FeroSNMP: GET, GETNEXT, GETBULK and WALK over SNMPv1 and
SNMPv2c on tokio, with timeouts, retries, a per-agent concurrency limit, an
operation log, and value formatting through the MIBs of `mib-core` (enum
labels, BITS names, DISPLAY-HINTs, units).

```rust
let engine = Engine::start(EngineConfig::default());
let target = Target::parse("192.0.2.1")?.with_community("public");
let mut running = engine.submit(target, Operation::walk("1.3.6.1.2.1.1".parse()?))?;
while let Some(event) = running.next_event().await {
    // Event::Varbinds, Event::Log, Event::Notice, then Finished or Failed
}
```

`Engine::submit` can be called from any thread, and `Running` has a
blocking and a non-blocking way to read events, so the GUI thread can use it
without being async itself (milestone M6).

## Tests

- `cargo test -p snmp-core` runs everywhere: unit tests, and engine tests
  against a small fake agent on localhost (walks, GETBULK fallback, tooBig,
  retries, late answers, non-increasing OIDs, the concurrency limit).
- `tests/agent/` is a net-snmp `snmpd` setup with fixed values
  (`FEROSNMP-TEST-MIB`), used by the golden-file tests in
  `apps/cli/tests/agent.rs`. See CONTRIBUTING.md.

## What snmp2 0.5 does and does not do

Milestone M5 started with a spike on [snmp2] 0.5.2, to see what it offers
for this engine, for SNMPv3 (M7) and for the trap receiver (M9). Findings:

| Area | snmp2 0.5.2 | Consequence |
| --- | --- | --- |
| v1/v2c sessions (`AsyncSession`) | GET, GETNEXT (one OID), GETBULK, SET | Usable API, but see the next rows |
| Timeouts | None: `recv` waits forever | Would need a wrapper and a new socket per retry |
| Late answers | A late answer to an earlier request is an error (`RequestIdMismatch`), not skipped | Retries break after a timeout |
| Value decoding | A value it cannot decode **ends the variable list silently**: a `Counter64` of 2^63 or more (9 bytes on the wire), any unknown tag (`NsapAddress`, `UInteger32`, ...) | A walk stops early **without an error** |
| Unsigned types | Read as signed and cast (works by accident for 32-bit types) | |
| Encoding | `pdu::Buf` and the `build_*` functions are private; only the sessions can build requests | Own requests need our own encoder |
| Raw bytes | Not kept | No true raw view |
| Stack use | Two 64 KiB buffers inside the session (`heap_buffers` feature moves them) | Stack overflow in tests without it |
| Notifications (M9) | Parses v1 Trap-PDU (with `V1TrapInfo`), v2 Trap and InformRequest; `Pdu::to_bytes` can build a Response for an INFORM | Usable for M9, with the same value-decoding gap |
| SNMPv3 (M7) | USM with MD5/SHA-1/SHA-2 and DES/AES (pure Rust or OpenSSL feature); only through `AsyncSession`, whose v3 encoder is private | M7 must decide: fix snmp2 upstream, fork it, or encode v3 ourselves with `rasn-snmp` |

Because silently truncated walks are exactly the kind of wrong answer
FeroSNMP must not give, the engine does not use snmp2's sessions. It has its
own small v1/v2c message codec (`src/codec.rs`): requests are encoded
directly, and responses are parsed with snmp2's public `AsnReader` (tag,
length, value), with our own value decoding that keeps anything unusual as
`Value::Other` instead of dropping it. The socket handling (`src/session.rs`)
does timeouts, retries with the same request-id, and skips late answers.

Gaps to fill later:

- **M7 (SNMPv3):** snmp2's USM code is only reachable through
  `AsyncSession`. Options: contribute fixes upstream (public encoder, value
  decoding, timeouts), or build v3 messages with `rasn-snmp` and our own USM
  on the RustCrypto crates snmp2 uses anyway.
- **M9 (traps):** decode notification variable lists with the same value
  decoder as `codec.rs`, not snmp2's `Varbinds`.

[snmp2]: https://docs.rs/snmp2/0.5.2
