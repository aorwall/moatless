# Spec - T3 uses Moatless Auth on a remote host

> **Status:** approved
> **Updated:** 2026-07-28
> **Vocabulary:** CONTEXT.md

## Current solution

T3 Code is being adapted to run against a Moatless deployment through the T3
compatibility surface in `backend/src/ui_rpc`. That backend surface already has
the important auth boundary: discovery is public, while session bootstrap,
shell bootstrap, and socket traffic require a validated Moatless identity.

The missing piece is the T3 browser experience. Today the T3 adapter path still
assumes T3's own environment token pairing model. For non-loopback Moatless
targets, the T3 dev proxy can inject a Moatless API key and strip browser
cookies. That lets the UI talk to Moatless, but it authenticates the proxy or
task environment rather than the person using the UI.

Moatless already has the desired browser auth model in its frontend:

| Step | Current Moatless frontend behaviour |
|---|---|
| Session check | Probe the current Moatless session before showing authenticated routes. |
| Unauthenticated user | Redirect to a login route and preserve the requested destination. |
| Login mode | Read the deployment auth mode before rendering password or OAuth choices. |
| Login success | Establish the Moatless session, clear cached user data, and return to the saved destination. |
| Browser credential | Use a protected session cookie, optionally scoped to a shared parent host. |

T3 should adopt that model directly. This is a T3 UI/runtime rewrite, not a T3
proxy feature.

## Planned solution

T3 gains a Moatless-backed auth mode in its own client runtime. When running
against Moatless, T3 probes the Moatless session, routes unauthenticated people
to a Moatless-style login route, reads the deployment auth mode there, performs
password or OAuth login against Moatless, clears cached user data on login, and
returns to the saved T3 destination after the Moatless session is established.

The seam sits in T3's client-runtime primary auth layer. Route components should
consume one environment-auth state rather than each route knowing Moatless
endpoints and redirects.

The remote deployment gives T3 and Moatless a shared parent host. Moatless
scopes the session cookie to that parent, and T3 sends credentialed browser
requests to Moatless. T3 environment pairing and browser API-key authentication
are removed from this branch.

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-remote-auth-boundary) | Remote auth boundary | T3 UI/runtime auth support | A T3 proxy would hide the auth model instead of making the UI understand Moatless Auth. |
| [2](#2-user-auth-source) | User auth source | Moatless auth session | T3 pairing creates a second trust system with no Moatless counterpart. |
| [3](#3-browser-credential) | Browser credential | Session cookie | API keys authenticate the proxy or task environment, not the person using the UI. |
| [4](#4-remote-cookie-strategy) | Remote cookie strategy | Shared parent host | Same exact origin couples deployment; cross-site cookies add avoidable browser and security risk. |
| [5](#5-auth-ui-surface) | Auth UI surface | T3 owns the Moatless-style login flow | Bouncing to Moatless or showing only a link makes T3 feel unauthenticated rather than Moatless-backed. |
| [6](#6-implementation-seam) | Implementation seam | T3 client-runtime primary auth | Route-local logic spreads auth rules; generated clients add a cross-repo build dependency. |
| [7](#7-pairing-outside-moatless-mode) | Pairing outside Moatless mode | Remove it from this branch | Keeping a second auth path leaves the fork with two login models. |
| [8](#8-testing-seam) | Testing seam | T3 browser/runtime tests plus one Moatless backend contract check | Unit-only tests miss redirects and cookies; full remote E2E is too brittle for the planning baseline. |

### 1. Remote auth boundary

T3 owns Moatless auth in its UI/runtime. It should not depend on a T3 proxy that
turns the browser into an API-key-backed caller.

The backend compatibility surface remains protected by Moatless auth. T3's job
is to establish and reuse that Moatless browser session.

### 2. User auth source

The authenticated user is the Moatless session user. T3 pairing is not a
fallback for Moatless mode because Moatless does not expose T3 pairing endpoints
and does not need a second trust system.

### 3. Browser credential

The browser credential is the Moatless session cookie. API keys stay available
for API-key use cases, but they are not the browser credential for T3 UI auth.

### 4. Remote cookie strategy

Remote T3 and Moatless deployments share a parent host. Moatless sets the
session cookie for that parent host, so T3 can make credentialed browser
requests without cross-site cookie requirements.

Same exact origin deployment is rejected because it couples the T3 host and
Moatless host too tightly. Cross-site cookies are rejected because they add
browser policy and security risk that the shared-parent-host model avoids.

### 5. Auth UI surface

T3 renders the Moatless-style login flow itself. It reads the Moatless auth mode
and presents password or OAuth login in the T3 UI, matching the Moatless
frontend behaviour.

This avoids a UI where T3 looks logged out and only points the user elsewhere.

### 6. Implementation seam

The implementation belongs in T3's client-runtime primary auth layer. The rest
of the UI should ask that layer whether the primary environment is authenticated,
loading, unauthenticated, or in login recovery.

Route-local checks are rejected because they spread auth rules through the app.
A generated Moatless client is rejected for this branch because it adds a
cross-repo build dependency where the needed auth surface is small.

### 7. Pairing outside Moatless mode

This branch removes T3 environment pairing instead of keeping it beside
Moatless Auth. Breaking changes are allowed, and the branch should have one
browser login model.

### 8. Testing seam

T3 should have browser/runtime tests against a fake Moatless-compatible auth
server. Those tests cover the user-facing auth flow: session probe, redirect to
login, auth mode discovery, password/OAuth path selection, successful login,
cache reset, return target, and credentialed requests.

Moatless should keep one backend contract check that pins the T3 auth surfaces:
discovery stays public, and session bootstrap, shell bootstrap, and socket
traffic require a Moatless identity.

Full remote-host E2E can be added later, but it is not the baseline test seam
for this change.

## Behaviour

**As someone opening T3 against a Moatless deployment:**

When I already have a valid Moatless session for the shared parent host, T3 opens
directly and its authenticated calls use that session.

When I do not have a valid Moatless session, T3 sends me to its login route and
keeps the destination I originally requested.

When I sign in with the configured Moatless login method, T3 establishes the
Moatless session and returns me to the saved destination.

When my session expires, T3 treats the primary environment as unauthenticated
and sends me through the same login flow.

**As an operator:**

When I deploy remote T3 for Moatless, I configure T3 and Moatless under a shared
parent host and configure the Moatless cookie domain accordingly.

When I inspect browser traffic, T3 authenticated calls carry the Moatless session
cookie and do not use T3 pairing tokens or injected Moatless API keys.

## Scenario chains

These scenario IDs describe the tests expected for the implementation. The T3
browser/runtime scenarios live with T3. The backend contract scenario lives in
Moatless.

```
Chain 1: T3 Moatless browser login
├── T3AUTH-01: Existing Moatless session opens T3
├── T3AUTH-02: Missing Moatless session routes to login
├── T3AUTH-03: Password login returns to the saved destination
├── T3AUTH-04: OAuth login starts from the T3 login route
└── T3AUTH-05: Expired session returns to login

Standalone
└── T3AUTH-10: Moatless T3 surfaces enforce Moatless authentication
```

### T3AUTH-01: Existing Moatless session opens T3

Verifies that T3 can use a valid Moatless browser session without pairing.

* **Given** T3 is running against a Moatless deployment
  * **And** the browser has a valid Moatless session for the shared parent host

* **When** the user opens T3

* **Then** T3 shows the authenticated application
  * **And** T3 does not start environment pairing
  * **And** authenticated primary-environment requests carry browser credentials

### T3AUTH-02: Missing Moatless session routes to login

Verifies that unauthenticated T3 users enter the Moatless-style login flow.

* **Given** T3 is running against a Moatless deployment
  * **And** the browser has no valid Moatless session

* **When** the user opens an authenticated T3 route

* **Then** T3 shows its login route
  * **And** the requested destination is preserved
  * **And** T3 does not start environment pairing

### T3AUTH-03: Password login returns to the saved destination

Verifies the password-auth path when the Moatless deployment enables password
login.

* **Given** T3AUTH-02 has completed
  * **And** the Moatless deployment offers password login

* **When** the user signs in with a valid password

* **Then** T3 establishes a Moatless session
  * **And** cached user data is cleared
  * **And** the user returns to the saved destination

### T3AUTH-04: OAuth login starts from the T3 login route

Verifies the OAuth path when the Moatless deployment enables an OAuth provider.

* **Given** T3AUTH-02 has completed
  * **And** the Moatless deployment offers an OAuth login option

* **When** the user chooses the OAuth login option

* **Then** T3 starts the Moatless OAuth login flow
  * **And** the flow can return to the saved T3 destination

### T3AUTH-05: Expired session returns to login

Verifies that T3 recovers from a session that stops being valid.

* **Given** T3AUTH-01 has completed
  * **And** the Moatless session is no longer valid

* **When** T3 checks the primary environment session

* **Then** T3 treats the primary environment as unauthenticated
  * **And** the user is sent through the Moatless-style login flow again

### T3AUTH-10: Moatless T3 surfaces enforce Moatless authentication

Verifies the backend contract that T3 relies on.

* **Given** the Moatless T3 compatibility surface is enabled

* **When** a client uses the T3 compatibility surface

* **Then** discovery is available before login
  * **And** session bootstrap requires a Moatless identity
  * **And** shell bootstrap requires a Moatless identity
  * **And** socket traffic requires a Moatless identity

## Implementation notes

The implementation should preserve these existing Moatless backend properties:

| Property | Reference |
|---|---|
| T3 compatibility router is opt-in | `backend/src/ui_rpc/mod.rs` |
| Discovery is the only public T3 route | `backend/src/auth/middleware.rs` |
| Browser session cookies are protected and may use a parent domain | `backend/src/auth/cookie.rs` |
| Moatless frontend probes session and redirects unauthenticated users to login | `apps/frontend/src/features/auth/components/auth-gate.tsx` |
| Moatless frontend reads auth mode and performs password/OAuth login | `apps/frontend/src/features/auth/hooks/use-auth-mode.ts`, `apps/frontend/src/features/auth/hooks/use-password-login.ts`, `apps/frontend/src/routes/login.tsx` |

The T3 side should remove or replace these behaviours in Moatless mode:

| Behaviour | Reference in T3 checkout |
|---|---|
| API-key injection for non-loopback Moatless targets | `apps/web/vite.config.ts` |
| Cookie stripping when proxying to Moatless | `apps/web/vite.config.ts` |
| Primary auth fallback to environment pairing | `apps/web/src/environments/primary/auth.ts` |
| Pairing-token exchange as browser recovery | T3 pairing route and components |

## Open questions

Non-blocking: decide whether local loopback keeps the existing cookie-sharing
path as a separate development mode.
