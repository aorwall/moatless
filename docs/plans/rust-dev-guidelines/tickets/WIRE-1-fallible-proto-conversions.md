# WIRE-1 — Make partial protobuf conversions fallible

- **Priority:** P2
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Replace partial `From` mappings with `TryFrom` or a narrower transport enum.
Reject the UI-only `EventType::Message` value instead of mapping it to
`Resources`, and reject unknown file-node discriminants instead of treating them
as files.

## Acceptance criteria

- [ ] Every representable event type round-trips exactly.
- [ ] `EventType::Message` cannot silently cross the sandbox protobuf boundary.
- [ ] Unknown event and file-node integer values return contextual errors.
- [ ] Recursive file-tree conversion propagates nested conversion failures.
- [ ] Conversion code remains at the most specific adapter/domain boundary.

## Verification

- Exhaustive event-type round trips.
- Rejection tests for `Message` and unknown protobuf integers.
- Nested file-tree failure propagation.
