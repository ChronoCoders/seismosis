# ADR-0001: WebSocket Service Kafka Offset Management — Manual Store with Auto-Commit Timer

**Date:** 2026-04-13
**Status:** Accepted
**Deciders:** Phase 2 engineering lead, rust-reviewer agent

## Context

The `seismosis-websocket` service (`services/websocket/`) consumes from `earthquakes.enriched` and `earthquakes.alerts`. It is a real-time fan-out service: its contract is to deliver events to connected clients as quickly as possible, not to guarantee exactly-once processing or provide replay.

The initial implementation used `enable.auto.commit=true` without any further offset store configuration. In librdkafka, auto-commit without disabling auto-store means the library stores (marks for commit) an offset as soon as the message is delivered to the application consumer — before the application has processed it. If the service crashes between message delivery and the broadcast fan-out, the offset is already committed and the message is silently lost on restart.

This was identified as BLOCKER-1 by the rust-reviewer agent.

## Decision

Configure the Kafka consumer with:

```
enable.auto.commit=true
enable.auto.offset.store=false
auto.commit.interval.ms=5000
```

After each message is fully processed (broadcast attempted for all clients, or skipped on decode error), the service calls `consumer.store_offset_from_message(&msg)`. librdkafka's background commit thread flushes stored offsets to Kafka every 5 seconds.

## Rationale

This is the canonical rdkafka at-least-once delivery pattern. It provides two properties:

1. **Offset only advances after processing.** A crash before `store_offset_from_message` leaves the offset uncommitted; the message is redelivered on restart.

2. **Per-message commit overhead is zero.** Storing an offset is a local operation (writing to a thread-local queue inside the rdkafka client). The actual Kafka RPC happens at the 5 s timer interval, not per message. For a real-time fan-out service under high message rates, per-message synchronous commits would add latency and broker load.

The 5 s commit interval means that on a clean restart the service re-processes up to 5 s of events. For the WebSocket use case, this causes at-most 5 s of duplicate events to clients that remain connected across the restart window. This is acceptable: the frontend deduplicates incoming events by `source_id`.

An alternative considered was manual commit (`enable.auto.commit=false`) with a synchronous `commit_message()` call per message. This provides stronger guarantees but adds a Kafka round-trip to every message's hot path. Given the real-time nature of the service and the frontend's deduplication, the synchronous commit overhead is not justified.

## Consequences

- At-least-once delivery: messages processed at most twice across a service restart within a 5 s window.
- Frontend `Dashboard.tsx` deduplicates events by `source_id` on arrival — at-least-once is handled cleanly.
- No per-message Kafka RPC on the hot path.
- A consumer group sharing the same `group.id` across multiple WebSocket instances would partition-split message delivery: each instance sees only a subset of events. For Phase 2 single-instance deployment this is not a concern. If Phase 3 requires horizontal scaling of the WebSocket service for fan-out, each instance must use a unique `group.id` (e.g. `seismosis-websocket-{hostname}`). This is documented in a code comment in `services/websocket/src/main.rs`.

## Alternatives Considered

**Manual commit per message (`enable.auto.commit=false` + `commit_message`):** Stronger delivery guarantee (offset advances only after confirmed Kafka acknowledgement). Rejected because it adds a synchronous Kafka RPC to every message on the critical path, adding latency with no user-visible benefit for a real-time service that already handles deduplication client-side.

**Full auto-store + auto-commit (original):** Zero application code for offset management. Rejected because it silently commits offsets for messages not yet processed, violating at-least-once semantics.
