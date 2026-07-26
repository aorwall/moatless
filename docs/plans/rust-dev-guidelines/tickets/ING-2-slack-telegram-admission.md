# ING-2 — Migrate Slack and Telegram inbound routing

- **Priority:** P1
- **Status:** Ready for agent
- **Blocked by:** ING-1

## What to build

Route required Slack and Telegram inbound-message work through the durable
adapter admission seam. Remove acknowledgment paths that only spawn
`route_inbound_message` and log a later failure.

## Acceptance criteria

- [ ] Both providers complete or durably enqueue required routing before
      acknowledgment.
- [ ] Provider event identifiers make processing idempotent.
- [ ] Existing response deadlines and signature/authentication behavior remain
      unchanged.
- [ ] Failures are retryable and observable through the shared worker policy.

## Verification

- Injected route failure is retried.
- Duplicate delivery creates no duplicate task or message.
- Restart recovery for both providers.
