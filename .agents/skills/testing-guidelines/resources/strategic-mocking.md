# Mock boundaries

Keep the application behavior being tested real. Stub external HTTP, browser APIs,
SDKs, clocks, and process boundaries when needed for deterministic tests. A thin
application wrapper around external I/O may be replaced at that boundary when the
test does not claim to cover the wrapper; test its mapping separately if meaningful.
Do not mock the feature hook or service whose behavior the test claims to verify.

Use typed mocks and reset them between tests. Prefer a local HTTP/WebSocket server
for protocol tests so the production parser and transport handling run together.
Inject endpoints or external clients through existing seams; do not create a
single-implementation interface just to simplify setup.

Assert requests, user-visible state, returned errors, and cleanup. Avoid asserting
every internal call unless ordering or a call budget is part of the contract.
