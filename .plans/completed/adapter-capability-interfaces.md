# Adapter capability interfaces

> **Status:** shipped
> **Updated:** 2026-08-01
> **Vocabulary:** CONTEXT.md

## Current solution

The Adapter layer currently has one broad trait in `backend/src/adapters/mod.rs`.

That trait mixes Adapter identity, user identity lookup, outbound messaging, direct sends, Binding destination construction, reaction support, presentation hooks, status fetching, and thread context fetching.

The default methods make unsupported capabilities easy to express, but they also make the trait shallow.

An Adapter implementer must read the whole surface even when the Adapter only supports a small part of it.

| Area | Current methods | Current callers | Cost |
|---|---|---|---|
| Adapter identity | `adapter_kind`, `provider_key`, `connection_provider_key` | Adapter registry and name lookup. | This is truly universal. |
| User identity | `resolve_user` | Adapter runtimes and direct-message paths. | Every Adapter must implement it, even when it always returns no user. |
| Binding destination construction | `binding_destination_from_existing_thread`, `binding_destination_from_new_thread` | Loop firing, Slack/Telegram handlers, GitHub PR shell reconciliation, outbound sends. | Existing-thread and new-thread construction are different capabilities but live together. |
| Replies | `send_reply`, `send_reply_for_binding`, `reply_to_message` | Binding reply flow and message reply flow. | Thread-shaped reply, Binding-shaped reply, and per-message reply are separate behaviours. |
| Direct sends | `post_to_destination`, `dm_destination`, `send_message`, `on_binding_created` | Adapter handler routes and outbound Loop sends. | Ad-hoc destination sends and Loop-created new threads share the same trait as ordinary replies. |
| Reactions | `add_reaction`, `remove_reaction` | Binding interaction handlers. | Most Adapters inherit rejection defaults. |
| Inbound presentation | `external_id_aliases`, `sender_label_suffix`, `inbound_message_prefix`, `reply_instructions`, `thread_reply_hint`, `fetch_context` | Binding runtime, message send deduplication, Loop-created Task prompts. | Formatting and context enrichment are read from the same interface as outbound transport. |
| Binding status | `binding_status_key`, `binding_label_and_url`, `is_terminal_state_kind`, `fetch_binding_status` | Binding status service and Task connection read paths. | Read-side metadata and live status fetching are coupled to send-side behaviour. |

The current registry stores one map of `AdapterKind` to `Arc<dyn Adapter>`.

It also stores typed handles for GitHub PR, Linear, Slack, and Telegram.

Those typed handles exist because adapter-internal code needs concrete-only methods.

The typed handles should not be removed by this change.

They solve a different problem than capability lookup.

The current default-method pattern has three practical effects.

| Effect | Example | Why it matters |
|---|---|---|
| Unsupported behaviour is discovered at runtime. | A direct destination send can reach an Adapter that only knows Binding replies. | Reviewers must check the default implementation and the caller's error mapping together. |
| Implementers see every feature at once. | Webhook supports callback replies but has no raw direct-send destination, reactions, status, or context fetch, yet those surfaces live on the same broad trait. | Adding a simple Adapter looks larger than it is. |
| Call sites depend on a broad interface. | Binding status reads receive the same trait object that can also send messages. | Tests and future modules learn more interface than they exercise. |

The current code already has useful grouping pressure.

| Call-site module | Capability it actually needs |
|---|---|
| `bindings::reply` | Binding reply and per-message reply. |
| `bindings::outbound` | Loop-created new-thread send, direct reply, and post-send Binding follow-up. |
| `bindings::handlers` | Reactions and message replies. |
| `bindings::status_service` | Binding status metadata and live status fetch. |
| `bindings::runtime` | inbound presentation, external-id aliases, and context fetch. |
| `loops::firing::listen` | destination construction for an existing external thread. |
| Adapter HTTP handlers | direct destination sends and direct-message destination resolution. |
| Task connection read handlers | Binding label and URL. |

The current Adapter implementation files mostly collect all trait methods in one large `impl Adapter for ...` block.

That makes method locality follow the old broad trait rather than the workflow the method belongs to.

| Adapter | Current shape | Cost |
|---|---|---|
| Slack | One broad trait implementation contains reply instructions, destination construction, labels, prefixes, replies, direct sends, reactions, and context fetch. | A maintainer changing reactions scans through unrelated send and presentation code. |
| Telegram | One broad trait implementation contains destination construction, aliases, user lookup, replies, direct messages, new-thread send, and reactions. | Unsupported and supported capabilities are interleaved. |
| GitHub PR | One broad trait implementation contains identity keys, instructions, user lookup, destination construction, aliases, replies, reactions, and status. | Status and reaction behaviour are far from their provider-specific helper modules. |
| Linear | One broad trait implementation contains replies, direct sends, Binding follow-up, instructions, status, labels, and hints. | Linear agent-session behaviour and Binding status behaviour sit behind one trait block. |
| Webhook | The implementation still sits behind the broad trait even though only callback replies and user identity are supported. | The file makes raw direct-send support look closer than it is. |

## Planned solution

Split the broad Adapter trait into a small universal identity interface and a set of capability interfaces.

The registry becomes the seam that answers which capabilities a configured Adapter has.

Callers ask for the capability they need.

Adapters implement only the capability traits they support.

Adapter implementation files should then be broken up by those capability traits.

Each capability implementation should sit near the provider helper code it uses.

A reviewer should be able to inspect one capability without reading unrelated capability methods.

The first implementation keeps behaviour unchanged.

Unsupported operations still become the same user-visible rejections.

The difference is that rejection is produced by the registry when a capability is absent, not by a broad trait default that every Adapter inherits.

### Problem

The current Adapter trait makes the easy path for new Adapters too large.

The affected people are:

| Actor | Problem |
|---|---|
| Adapter implementer | Must understand unrelated send, status, reaction, and context hooks before adding one Adapter. |
| Reviewer | Must distinguish real Adapter behaviour from inherited default behaviour in every implementation. |
| Product engineer | Cannot tell from the type surface whether an Adapter supports a workflow. |
| Test author | Must construct broad stand-in Adapters for tests that need one narrow behaviour. |

The goal is a deeper Adapter registry interface.

The goal is not to make a trait per method.

The goal is not to remove all default behaviour in one commit.

### Behaviour

#### Integration developer adds a simple inbound Adapter

| Given | When | Then |
|---|---|---|
| The Adapter only admits inbound messages and never sends replies. | The developer registers it. | The Adapter implements identity plus inbound delivery registration only. |
| A caller asks for outbound send support. | The Adapter has not registered that capability. | The registry returns the existing unsupported-operation error. |
| A reviewer opens the Adapter implementation. | They inspect supported capabilities. | Unsupported capabilities are absent, not hidden behind inherited defaults. |

#### Integration developer adds a chat Adapter

| Given | When | Then |
|---|---|---|
| The Adapter supports Binding replies and direct-message destinations. | The developer implements the outbound and destination capabilities. | Reply call sites can send through the capability without learning status or reaction hooks. |
| The Adapter supports reactions. | It implements the reaction capability. | Reaction handlers depend only on the reaction interface. |
| The Adapter supports thread context enrichment. | It implements the inbound presentation capability. | Binding runtime can fetch context without depending on direct-send behaviour. |
| The Adapter implementation grows. | The developer adds another capability. | The new implementation lands in the capability-specific block or module, not in one broad Adapter implementation. |

#### Product engineer reads capabilities

| Given | When | Then |
|---|---|---|
| A UI path exposes a direct-send action. | It checks the registry for direct-send capability. | The action is based on declared support. |
| A status read path renders a Binding. | It checks the status capability. | The path cannot accidentally call send behaviour. |
| A task reply targets a specific external message. | It checks message-reply capability. | Providers without per-message reply fall back through the chosen reply capability path. |

#### Existing Adapters keep behaviour

| Given | When | Then |
|---|---|---|
| Slack sends replies, direct messages, reactions, and context. | The split lands. | Slack keeps every supported workflow. |
| Telegram sends replies and direct messages but has no live status. | The split lands. | Telegram registers no status capability. |
| GitHub PR supports Binding replies, per-message replies, reactions, status, labels, and aliases. | The split lands. | GitHub PR keeps those behaviours under narrower traits. |
| Linear supports issue comments, status, labels, and instructions. | The split lands. | Linear keeps those behaviours under narrower traits. |
| Webhook supports callback replies but no raw direct-send destination. | The split lands. | Webhook registers outbound reply capability while direct-send behavior still rejects as unsupported. |

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-capability-groups) | Capability groups | Split by call-site behaviour: identity, destination, outbound, reactions, inbound presentation, status, and user identity. | One trait per method is shallow; one outbound mega-trait still makes simple Adapters read too much. |
| [2](#2-registry-shape) | Registry shape | Registry stores optional capability trait objects per Adapter kind. | Keeping one `dyn Adapter` map leaves call sites broad; downcasting concrete adapters makes call sites know provider types. |
| [3](#3-migration-order) | Migration order | Add capabilities beside the current trait, migrate call sites, then delete broad defaults. | A direct trait replacement creates a wide mechanical diff with no behaviour checkpoint. |
| [4](#4-unsupported-behaviour) | Unsupported behaviour | Capability absence maps to the same AdapterError rejections as today's defaults. | Returning `Option` to every caller spreads unsupported-case policy across call sites. |
| [5](#5-typed-provider-handles) | Typed provider handles | Keep the concrete handles for provider-internal code. | Removing them would force downcasts or push concrete-only workflows into shared traits. |
| [6](#6-implementation-layout) | Implementation layout | Break Adapter implementation code into capability-specific impl blocks or modules after the trait split. | Keeping one broad implementation block preserves the old navigation cost even after call sites get narrower traits. |

### 1. Capability groups

The split follows existing call-site clusters.

The first capability interfaces are:

| Interface | Methods | Primary callers |
|---|---|---|
| `AdapterIdentity` | Adapter kind and provider keys. | Registry and name lookup. |
| `AdapterUserIdentity` | Resolve an external identity to a user. | Adapter runtimes and direct-message lookup paths. |
| `AdapterDestinationBuilder` | Build a Binding destination from an existing external subject or from a newly created external subject. | Loop firing, Adapter handlers, outbound sends. |
| `AdapterOutbound` | Send a Binding reply, reply to a specific external message, post to a raw destination, resolve a direct-message destination, start a new external thread, and run post-Binding follow-up. | Binding reply, message reply, direct send, and Loop outbound paths. |
| `AdapterReactions` | Add and remove reactions. | Binding interaction handlers. |
| `AdapterInboundPresentation` | External-id aliases, sender label suffix, inbound prefix, reply instructions, thread reply hint, and context fetch. | Binding runtime and Task creation prompts. |
| `AdapterBindingStatus` | Binding status key, label and URL, terminal state, and live status fetch. | Binding status service and Task connection read paths. |

This is not final naming.

The implementation may choose shorter names if they preserve the same grouping.

The grouping is the stable decision.

The `AdapterOutbound` interface is intentionally larger than a single method.

Its methods participate in the same send workflows and share message, destination, and Binding vocabulary.

Splitting it further at the start would create shallow interfaces before the call sites prove they need separate seams.

### 2. Registry shape

The registry becomes the capability directory.

For each configured Adapter kind it stores:

| Slot | Required |
|---|---|
| Identity | Yes. |
| User identity | Optional. |
| Destination builder | Optional. |
| Outbound | Optional. |
| Reactions | Optional. |
| Inbound presentation | Optional. |
| Binding status | Optional. |

The registry exposes methods that name the capability, not the storage.

Example shape:

```rust
impl Adapters {
    pub fn outbound(&self, kind: AdapterKind) -> Result<&dyn AdapterOutbound, AdapterError>;
    pub fn reactions(&self, kind: AdapterKind) -> Result<&dyn AdapterReactions, AdapterError>;
    pub fn binding_status(&self, kind: AdapterKind) -> Option<&dyn AdapterBindingStatus>;
}
```

Callers do not inspect the registry's maps.

Callers do not downcast.

Callers get either a capability object or a stable unsupported-operation result.

### 3. Migration order

The migration should happen in phases.

| Phase | Change | Verification |
|---|---|---|
| 1 | Add capability traits and registry slots while preserving the current broad trait. | Existing tests still pass. |
| 2 | Register existing Adapters into capability slots. | Registry unit tests prove expected capability presence and absence. |
| 3 | Migrate read-only call sites first: labels, status, aliases, prompt instructions, thread hints, context fetch. | Binding status and inbound routing tests pass. |
| 4 | Migrate outbound call sites: replies, direct sends, new-thread sends, post-Binding follow-up. | Binding reply, outbound, direct-send, and Adapter handler tests pass. |
| 5 | Migrate reaction call sites. | Reaction handler tests pass. |
| 6 | Split adapter implementation code by capability trait. | Each Adapter has capability-specific impl blocks or modules. |
| 7 | Delete broad trait defaults that no call site uses. | Compile errors identify any remaining broad calls. |
| 8 | Remove or shrink the old broad trait once it only aliases identity. | Adapter implementation files get smaller. |

This order gives behaviour checkpoints.

It avoids one large diff that changes registration, implementation, and every caller at once.

### 4. Unsupported behaviour

Unsupported behaviour remains explicit and stable.

The registry should own the conversion from missing capability to the current user-facing rejection.

| Missing capability | Existing behaviour to preserve |
|---|---|
| Outbound destination send | Reject as unsupported for that Adapter. |
| Direct-message destination | Report no direct-message surface. |
| Binding reply | Reject when a Binding cannot be replied to. |
| Per-message reply | Use Binding reply fallback where that is currently the Adapter's behaviour. |
| Reactions | Reject as unsupported for that Adapter. |
| Status | Return no live status and fall back to persisted status. |
| Context fetch | Return no extra context. |

Call sites should not each invent a missing-capability response.

This keeps locality in the registry.

### 5. Typed provider handles

Typed handles remain.

They are not an Adapter capability.

They are a locality tool for provider modules that need concrete-only behaviour.

Examples:

| Handle | Why it stays |
|---|---|
| GitHub PR | The provider module has webhook parsing, GitHub app token resolution, and PR-specific runtime helpers. |
| Linear | The provider module has Linear agent-session and OAuth behaviours. |
| Slack | The Slack handler needs Slack-specific event parsing and destination construction helpers. |
| Telegram | The poller and webhook path need Telegram-specific update handling. |

Capability traits are for cross-adapter call sites.

Typed handles are for adapter-internal call sites.

Those are different seams.

### 6. Implementation layout

The trait split should be visible in each Adapter implementation.

Do not leave one large compatibility-shaped implementation after call sites move to capabilities.

Use one of two layouts per Adapter.

| Layout | Use when | Shape |
|---|---|---|
| Capability-specific impl blocks in the existing adapter file | The Adapter file stays readable after the split. | `impl AdapterOutbound for SlackAdapter`, `impl AdapterReactions for SlackAdapter`, and similar blocks are grouped by capability. |
| Capability-specific submodules | The Adapter already has enough helper code that one file remains too large. | Provider module owns files such as `outbound.rs`, `reactions.rs`, `status.rs`, or `presentation.rs`, each implementing the matching trait. |

The layout should follow locality.

| Capability | Preferred location |
|---|---|
| Outbound sending | Near provider API calls and outbound destination helpers. |
| Destination construction | Near subject-key and destination-config builders. |
| Reactions | Near reaction API helpers and external-id parsing. |
| Binding status | Near status parsing and terminal-state helpers. |
| Inbound presentation | Near message formatting, aliases, prompt instructions, and context fetch code. |
| User identity | Near provider identity lookup helpers. |

The implementation split is part of the change.

It is not optional cleanup.

The old broad implementation block may remain only while the migration still needs it as a compatibility bridge.

Once a capability's call sites have moved, that capability's implementation should live under its new trait.

## Seams and testing

The main module is the Adapter registry.

Its interface is the new capability lookup surface.

The implementation owns capability storage and unsupported-operation mapping.

| Module | Interface | Depth gained | Test surface |
|---|---|---|---|
| Adapter registry | Capability lookup by Adapter kind or provider name. | Callers learn one narrow capability instead of the whole Adapter trait. | Unit tests over a registry containing minimal stand-in Adapters. |
| Adapter capability traits | Behaviour grouped by call-site workflow. | Adapter implementations expose only supported workflows. | Trait-object tests through registry accessors. |
| Adapter implementation modules | Capability-specific impl blocks or files. | Maintainers change one capability without scanning unrelated ones. | Existing adapter tests continue to exercise the same behaviour through narrower traits. |
| Binding reply and outbound modules | Depend on outbound capability. | Reply tests no longer need status or presentation stubs. | Existing binding reply and outbound tests. |
| Binding status module | Depends on status capability. | Status reads cannot call send methods. | Existing binding status tests plus missing-status case. |
| Binding runtime | Depends on inbound presentation capability. | Message formatting and context enrichment are local. | Existing inbound routing tests. |
| Adapter interaction handlers | Depend on reaction and direct-send capabilities. | Unsupported behaviour policy is exercised through registry lookup. | Existing handler tests plus missing-capability cases. |

Add focused tests before deleting old defaults.

Suggested coverage:

| Scenario | Expected proof |
|---|---|
| Webhook registers outbound capability for callback replies. | Direct-send behavior still returns the existing unsupported rejection. |
| Slack registers outbound, destination, reactions, inbound presentation, and context capabilities. | Existing Slack reply and reaction tests pass through the capability accessors. |
| Telegram registers outbound and destination capabilities but no status capability. | Status lookup returns absent and read paths fall back. |
| GitHub PR registers status and reactions. | Existing status and reaction tests pass. |
| Linear registers status and outbound Binding replies but no reaction capability. | Reaction lookup rejects without calling Linear code. |
| A test stand-in implements only one capability. | A caller test can construct that stand-in without filling unrelated methods. |

The highest useful seam is the registry interface.

Tests should not call capability storage internals.

Tests should not assert map shape.

## Out of scope

| Out of scope | Reason |
|---|---|
| Changing Adapter delivery, typed delivery, ingress, or subject-event routing. | Those runtime seams were handled in the previous change. |
| Removing concrete provider handles. | They serve provider-internal locality. |
| Renaming AdapterKind values or provider keys. | This is a capability split, not a data migration. |
| Changing user-visible unsupported-operation messages. | Behaviour should stay stable during the interface split. |
| Rewriting Adapter HTTP handlers. | They should only change where they fetch capabilities. |
| Adding a new Adapter. | The split prepares for this but does not validate one new Adapter in the same change. |

## Open questions

None.

The exact Rust trait names may change during implementation.

The capability grouping and migration order are settled.

## Ticket breakdown

None.
