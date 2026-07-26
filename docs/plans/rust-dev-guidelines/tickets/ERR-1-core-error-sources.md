# ERR-1 — Preserve sources in core backend errors

- **Priority:** P2
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Replace source-erasing string wrappers in core backend error families with
module-owned, typed source-bearing variants. Scope includes auth routes, message
send, sandbox handlers/client/provision/providers, credentials/Codex refresh,
feedback, and skill service.

## Acceptance criteria

- [ ] Wrapped failures remain discoverable through `Error::source`.
- [ ] Each module owns meaningful variants; do not introduce one shared generic
      `InternalError`.
- [ ] Synthesized internal messages remain distinct from wrapped failures.
- [ ] HTTP status and response bodies stay stable and redacted.
- [ ] The causal error is logged once at the response/observability boundary.

## Verification

- Source-chain assertions for every migrated error family.
- Response snapshots contain no SQL, credential, gRPC, filesystem, or
  cryptographic detail.
