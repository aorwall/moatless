# ERR-2 — Preserve sources in adapter and binding errors

- **Priority:** P2
- **Status:** Ready for agent
- **Blocked by:** None

## What to build

Replace source-erasing string wrappers in adapter, provider, pairing, and binding
error families with module-owned source-bearing variants. Distinguish real
wrapping from synthesized messages rather than mechanically replacing every
`Internal(String)`.

## Acceptance criteria

- [ ] Wrapped adapter and binding failures remain available through
      `Error::source`.
- [ ] Module-specific variants preserve useful classification.
- [ ] Provider-facing and HTTP responses remain stable and redacted.
- [ ] Each causal failure is logged once.
- [ ] No shared catch-all internal-error abstraction is added.

## Verification

- Adapter-specific source-chain assertions.
- Provider/HTTP response snapshots.
- Regression coverage for GitHub, Linear, Slack, Telegram, pairing, and binding
  error paths.
