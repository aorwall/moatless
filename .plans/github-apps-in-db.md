# Administering GitHub Apps from the database

> **Status:** approved
> **Updated:** 2026-07-30
> **Vocabulary:** CONTEXT.md

## Current solution

A GitHub App's registration — its identifier, its numeric id at GitHub, and the signing key this
installation holds — lives in the process environment and nowhere else. Everything built on top of
that registration already lives in the database and is already administered from the UI.

### Where each piece of App state lives

| State | Lives in | Changed by | What it costs |
|---|---|---|---|
| App key, numeric App id, signing key, optional login client id and secret, optional installation id | Process environment: `GITHUB_APPS_JSON` for bot Apps, the `GITHUB_APP_*` variables for the login App | Whoever can edit the Helm release or the local env file | A rollout. No record of who changed it. Silent on malformed input. |
| Which installation the shared binding uses | Database, on the GitHub Adapter connection row | An admin, in the UI | Takes effect immediately |
| The Bot user tied to an App | Database, a user row plus its stored credential row | An admin, through the CLI | Takes effect immediately. No UI exists. |
| Minted installation tokens, per-owner installation ids, resolved bot identity | Memory, per registry instance | Nobody — expiry only | Discarded whenever the registry instance is replaced |

The first row is the whole problem. It is the only row that needs a deploy, and it is the row every
other row depends on.

### The trace, in six hops

1. **Read.** The environment is parsed into config once, at process start — `backend/src/config.rs:1137`
   for the bot-App list, `backend/src/config.rs:573-587` for the login App. A malformed list is
   logged and discarded (`backend/src/config.rs:207`); the process then starts normally with no bot
   Apps at all.
2. **Build.** Two registries are built from that config, in the same process — `backend/src/main.rs:366`
   for the HTTP state and the Adapter layer, `backend/src/startup.rs:314` for the sandbox service.
   They share nothing, including their caches.
3. **Register.** The login App is inserted under the key `default`, then each listed App under its own
   key — `backend/src/auth/github_app_registry.rs:37`. A colliding key is skipped with a warning. A
   signing key that will not parse is skipped with a warning (`backend/src/auth/github_app_registry.rs:60`).
   Both outcomes leave the App simply absent.
4. **Hold.** The registry is held behind a shared pointer with no interior mutability —
   `backend/src/state.rs:90`. No code path adds, removes or replaces an App after this point.
5. **Use.** Four readers look Apps up by key: the admin listing and the installation binding
   (`backend/src/provider_settings/handlers.rs:278` and `:155`), Bot user creation
   (`backend/src/auth/routes.rs:1794`), and token resolution for Bot users
   (`backend/src/auth/credentials.rs:37`). An unknown key is a clear, immediate error at each one,
   and the error names the keys that do exist.
6. **Cache.** Each App instance accumulates its own caches: installation tokens, per-repository
   tokens, installation id per owner, and the resolved bot identity —
   `backend/src/auth/github_app_service.rs:47-56`. All of it is discarded if the instance is replaced.

### What the App is used for once it exists

- **Minting a short-lived token for a repository owner.** The canonical credential resolver branches
  on how a stored credential is obtained: a Bot user's credential says "mint from this App", and the
  App key is what says which App (`backend/src/auth/credentials.rs:37-46`). A missing App or a
  missing repository owner is a hard error there, deliberately, so a misconfigured bot cannot look
  like a bot with no credential.
- **Resolving the bot identity.** The App's own name at GitHub is fetched, the bot login is derived
  from it, and the bot's numeric id yields the address that commits are attributed to
  (`backend/src/auth/github_app_service.rs:589-655`).
- **Backing the shared installation binding.** An admin picks an App and one of its installations;
  the choice is mirrored onto the GitHub Adapter connection row for that account, which is what makes
  inbound GitHub activity resolve (`backend/src/provider_settings/handlers.rs:201-265`).
- **Providing the git identity inside a Sandbox.** A Bot user's login and address are delivered as
  the commit author and committer for tasks that run as it (`backend/src/sandbox/secrets.rs:440`).

### Dead code in the affected area

The GitHub Adapter connection row carries a credential reference field. Every site that creates one
writes it empty, and nothing resolves it: the persistence layer round-trips it
(`backend/src/adapters/dao.rs:49`) and the installation binding preserves whatever was already there
(`backend/src/provider_settings/handlers.rs:253`). The seam for "this connection points at a stored
credential" exists and has never carried anything. This change does not use it, so it should be
deleted separately.

### What already exists and is used

- **Secrets.** Scoped, envelope-encrypted, audited, with an admin UI and CLI. A kind exists for
  values that must never reach a Sandbox. Secret names are constrained to upper case and underscores,
  and are unique per scope.
- **Plugins.** The closest prior art for the shape this change wants: an admin registers one once, it
  is stored in the database, and it is thereafter available to everyone, with an admin page and a CLI.
- **Bot user creation.** One admin call resolves the App's bot identity from GitHub and writes the
  user row and its credential row together, in one transaction, repeatably
  (`backend/src/auth/routes.rs:1782-1910`).

### What it costs today

| Task | Today |
|---|---|
| Add a second bot App | Hand-assemble a JSON list containing a signing key, place it in the deployment's extra-secret escape hatch because the chart has no wiring of its own for it, and roll the backend out. |
| Find out you got it wrong | The App is silently absent. The next bot task fails when a token cannot be minted. |
| Rotate a signing key | Edit the deployment, roll out, discard every cached token for every App. |
| See which Apps exist | Works. The admin page lists them and their installations, read-only. |
| Add or remove an App from the UI | Not possible. |
| Know who changed an App, and when | Not recorded anywhere. |

## Planned solution

### Problem

An admin who wants a second bot cannot get one without being a deployment operator. The knowledge
needed is not "which App should this be" but "where the Helm values live, which escape hatch passes
an unlisted variable through to the backend, and how to embed a multi-line signing key inside a JSON
string". The person who knows the first thing is often not the person who can do the second.

When they get it wrong, nothing says so. A malformed list is discarded at start-up and the
installation runs with no bot Apps. The failure surfaces later, somewhere else, as a task that could
not obtain a token.

Rotating a signing key — the one operation that has a deadline attached, because it is what you do
when a key has leaked — requires a deploy, and throws away the working tokens of every other App on
the way through.

And nothing records who did any of it.

### Behaviour

#### As an admin

**Registering an App.**

* **Given** an App has been created at GitHub, and its numeric id and signing key are in hand
* **When** the admin registers it, choosing the short name it will be known by
* **Then** the signing key is used against GitHub immediately, and the App it identifies is confirmed
  to be the App the admin named
  * **And** the App's display name and the bot login it would have are stored alongside it
  * **And** the App is usable at once, by this backend and by every other backend serving the same
    installation
  * **And** the signing key cannot be read back out of the installation by anybody, through any
    surface

**Registering an App that cannot be proved.**

* **Given** the admin has supplied a signing key that GitHub does not accept for the App id given
* **When** they try to register it
* **Then** nothing is stored, and they are told which part failed

| What went wrong | What the admin is told |
|---|---|
| The signing key cannot be read as a key at all | The key could not be read. Nothing was stored. |
| GitHub rejects a request signed with it | GitHub did not accept this key for this App. Nothing was stored. |
| The key belongs to a different App than the id given | This key belongs to a different App, named. Nothing was stored. |
| The chosen short name is already taken | That name is already in use by another App. |
| GitHub cannot be reached | GitHub could not be reached, so the App was not stored. Try again. |

**Rotating a signing key.**

* **Given** a registered App whose key must be replaced
* **When** the admin supplies a new key
* **Then** the new key is proved against GitHub before anything is replaced
  * **And** on success the stored key is replaced, and only that App's obtained tokens are discarded
  * **And** the next call anywhere in the installation uses the new key, with no restart
  * **And** on failure the App is left exactly as it was

**Giving an App a bot identity.**

* **Given** an App has just been registered
* **When** the admin is shown the bot identity that App would have, offered as the next thing to do
* **Then** accepting it creates the Bot user, and work can be attributed to it from that point
  * **And** running it again refreshes the identity from GitHub without producing a second user
  * **And** declining it leaves a perfectly usable App that simply never runs as anyone

**Seeing what exists.**

* **Given** an installation with several Apps
* **When** the admin opens the GitHub admin page
* **Then** every App is listed, with its accounts, whether it has a Bot user, and enough of a
  fingerprint of its signing key to tell two apart
  * **And** the App used for signing in is listed too, marked as configured by the deployment and not
    editable

**Removing an App.**

* **Given** a registered App that is no longer wanted
* **When** the admin removes it
* **Then** it stops being available across the whole installation at once

**What an admin cannot do.**

* Change the App used for signing in. It is configured by the deployment, and no screen offers to
  change it.
* Read any stored signing key back.
* Register an App on an installation that has no encryption configured — see Open questions.

#### As an operator upgrading an existing installation

* **Given** an installation that configures bot Apps at deploy time today
* **When** the upgraded backend starts for the first time
* **Then** each configured App is copied into durable state, and everything that referenced those
  Apps keeps working
  * **And** the number carried over is stated, and any entry that could not be carried over is named
  * **And** the deployment configuration is ignored from then on and can be deleted
  * **And** a later restart does not undo an admin's subsequent changes, including a deletion

The carry-over must hold four properties:

1. **It happens once, and it knows that it has.** Whether it has run is recorded in its own right,
   never inferred from which Apps currently exist. Inferring it would resurrect a carried-over App
   that an admin has since deleted, at the next restart.
2. **Several backends starting together produce one outcome.** Two processes carrying over the same
   App must not create two records, and must not fail each other's start.
3. **A bad entry does not stop the installation from starting.** An entry that cannot be carried over
   is reported and skipped. The App is then absent, exactly as it is absent today, and an admin can
   register it properly.
4. **Nothing is dropped in silence.** The count carried over and the names skipped are both stated,
   because silent absence is one of the failures this change exists to remove.

#### As someone doing this from the command line

Every action available to an admin in the UI is available from the CLI, over the same interface: list,
register, rotate, remove, and create the Bot user. Registering reads the signing key from a file or
from standard input, never from a command-line argument, so it does not land in a shell history. No
command prints a stored signing key.

#### As everyone else

Nothing changes. Loops and triggers that name a Bot user to run as are untouched. Commits, pull
requests and comments carry the same identities they carried before. The shared installation binding
keeps pointing at the App it points at today.

#### As the running system

* **Given** two backends serving one installation
* **When** an App is registered, rotated or removed on either of them
* **Then** both behave identically from the next use onwards, with no coordination between them and
  no window in which they disagree
* **And** editing one App does not disturb any other App's obtained tokens

## Decisions

| # | Decision | Chosen | Rejected because |
|---|---|---|---|
| [1](#1-a-change-applies-without-a-restart) | Restart needed to apply | No — every backend picks a change up on its own | A restart-to-apply rule makes the UI a form that does not do what it says, and leaves the operator holding the deploy step this change exists to remove |
| [2](#2-stored-state-is-authoritative-at-the-moment-of-use) | How a change reaches each backend | Stored state is consulted at each use; obtained tokens survive an unchanged record | A staleness window lets an admin register an App and watch it not work yet; announcing changes puts an optional message service on a correctness path, and its absence is silent |
| [3](#3-the-app-is-its-own-administered-record) | Where the record lives | Its own table | An Adapter connection row would make the App key masquerade as an external account id, in a table keyed one row per account, when several accounts already point at one App |
| [3b](#3b-the-table-is-github-shaped) | Table shape | GitHub-shaped, every column required | A host-generic table would carry columns mandatory for exactly one host and meaningless for the rest, and would duplicate the stored-credential path that already covers every host without a signing key |
| [4](#4-the-signing-key-belongs-to-the-app-record) | Where the signing key lives | Encrypted on the App record, with the same cipher the secret store uses | A separate Secret needs a derived upper-case name, so two App keys differing only in punctuation collide silently; it is also independently deletable by an admin who cannot see what depends on it |
| [5](#5-existing-deployments-carry-over-by-themselves) | Existing deployments | Carried over automatically on first start, then stored state is the only source | Reading both sources leaves half the Apps permanently uneditable and puts precedence on every read; dropping the environment outright stops a working bot at upgrade until someone runs a command |
| [6](#6-the-login-app-stays-where-it-is) | The login App | Stays deployment-configured; bot Apps only | Administering it from the UI means one bad edit can lock every admin out of the only surface that could fix it |
| [7](#7-a-stored-app-is-one-that-has-already-worked) | Registration | Proved against GitHub before it is stored; a failed proof refuses the registration | Storing blind reproduces the silent failure this change exists to remove; storing an unproved App keeps registration possible during an outage at the price of a state every screen must show and every reader must decide whether to honour |
| [8](#8-apps-are-installation-wide-and-admin-owned) | Who may register | Admins only, one shared set for the installation | Personal registrations would have to carry through to Bot users, which are installation-wide identities that Loops and triggers name by id, and the access plumbing would serve a capability nobody asked for |
| [9](#9-two-new-glossary-entries) | Vocabulary | Two entries: GitHub App, Bot user | A third entry for the App's identifier would introduce a word for something the code already names consistently, sitting next to a signing key in the same record |
| [10](#10-registering-and-staffing-are-separate-acts) | Bot user creation | A separate second act, offered immediately after registering | Always creating one welds together two operations that are useful apart, and leaves an unused identity behind for an App that only authenticates repository access |

### 1 · A change applies without a restart

An App registered through either surface is usable immediately, on the backend that took the request
and on every other backend serving the same installation. No screen says a change is pending, and no
operator is asked to roll anything out.

**Rejected: a restart applies the change.** It halves the work, and it makes the new UI a form that
does not do what it appears to do. The deploy step is the thing being removed; keeping it while
adding a form that looks like it replaced it is worse than not adding the form.

**What this forces:**

- The App list stops being start-up state. Both construction sites read it once at process start and
  hold it immutably today; neither survives as it is.
- The obtained-token caches need a life of their own. If picking up a change means replacing a
  registry instance, an unrelated edit discards live tokens for every App.
- Absence must stay loud. An unknown App key is a clear error at every reader today, and re-reading
  records must not turn that into an intermittent error while a reload is in flight.

### 2 · Stored state is authoritative at the moment of use

Nothing holds a private copy of which Apps exist. Every reader establishes what an App is from
durable state at the moment it needs it. There is no window in which two backends disagree, and no
timer to tune.

Two things are cached, and they are not the same thing:

| Cached | Policy |
|---|---|
| What an App **is** | Never cached. Read when needed. |
| What an App **has obtained from GitHub** — minted tokens, resolved installation ids, the bot identity | Cached as today, held against a specific revision of the App record. Editing one App discards only that App's. |

The cost is one small indexed read on paths that were already about to call GitHub. The benefit is
that rotating a key takes effect on the next call, everywhere, with no coordination between backends.

**Rejected: a short staleness window**, each backend re-reading on a timer, in the shape the auth
cache already uses. It is the least code. It also means an admin can register an App and watch it not
work yet, which is a support question that recurs forever, and the window has to be explained on the
screen.

**Rejected: announcing a change to the other backends.** Also immediate. It puts an optional message
service on a correctness path — the installation runs without it today, and its absence produces no
error, so the failure mode is an installation where registration silently only affects one backend.

### 3 · The App is its own administered record

An App registration is a durable record in its own right, in its own table. Adapter connections and
Bot users point at it; it belongs to neither.

The relation is already many-to-one and the code already assumes it: the GitHub Adapter connection
row is keyed one per account, of kind `org_webhook`, and names its App by key inside its transport
configuration (`backend/src/provider_settings/handlers.rs:239`). Several accounts can name the same
App. The glossary also already defines an Adapter connection as a credentialed installation of an
Adapter — an App is the credential issuer behind several of those, not one of them.

**Rejected: an Adapter connection row per App, under a new Adapter kind.** It reuses the existing
persistence, scope handling and access checks. It also requires the App key to masquerade as an
external account id in a table whose uniqueness is keyed on exactly that, and puts a credential
issuer in the same table as the accounts that consume it.

### 3b · The table is GitHub-shaped

Every column is required. When another git host arrives, its bot gets a stored credential and a new
resolution branch, and this table is not touched.

GitLab is a declared direction rather than a hypothesis: a git-host variant exists
(`crates/api-schemas/src/enums.rs:495`), a feature flag exists and is off
(`backend/src/feature_flags.rs:47`), the repository config reader already fetches GitLab files with a
token (`backend/src/repository/config_reader.rs:172`), and GitLab is phase 2 of
`docs/architecture/repository-source-model.md`. So the question of whether this table should serve it
is real. The answer turns on which parts of a GitHub App have a counterpart elsewhere.

| A GitHub App is four things | GitLab counterpart | Gitness counterpart | Generalises? |
|---|---|---|---|
| A machine identity registered at the host, with a stable id and display name | A group or project access token, or a service account | A user holding a personal token | Yes |
| A bot identity derived from it, usable as a commit author | Yes — a bot user is created alongside the token | Whatever user issued the token | Yes |
| An asymmetric key we hold and sign with | None. The credential is the token itself | None | No |
| Per-account installations, each yielding a freshly minted short-lived token | None. One long-lived token, scoped where it was created | None | No |

The asymmetry in one line: a GitHub App is a **credential issuer**, and every other host here hands
out a **credential**. Storing a credential is a solved problem in this codebase — encrypted, scoped,
audited, already administered from the UI and the CLI. Nothing about a GitLab bot needs a new table.

The extension point already exists, and it is not this table. A Bot user does not reference an App
directly: it holds a stored credential whose method says how to obtain a usable token, and the
resolver branches on that method (`backend/src/auth/credentials.rs:37-46`). A GitLab bot is a new
branch there.

**Rejected: a host-generic table with a discriminator and nullable per-host columns.** The nullable
columns would be "required for exactly one host", which a column cannot express, and the table's
meaning would depend on its discriminator.

**Rejected: a host-generic table that every host's machine credential moves into.** One place for all
machine identities, at the cost of duplicating the stored-credential path that already does this for
every host, including GitHub's own personal tokens.

**Caveat to verify before implementing.** There is no GitLab machine-credential code in this
repository to check against, so the GitLab column above comes from general knowledge of the platform
rather than from the codebase. It should be confirmed against GitLab's current documentation. The
conclusion holds as long as GitLab still has no signing-key-and-installation model.

### 4 · The signing key belongs to the App record

The signing key is part of the App record, encrypted at rest with the same protection every other
stored credential gets. It cannot be orphaned, renamed, or removed by somebody editing a list of
credentials who cannot see what depends on them. An App without a usable key is not a state the
storage can represent.

**Rejected: a Secret that the App record points at.** It reuses encryption, versioning, audit and
rotation exactly as they stand, and it would have revived the dead credential-reference field. It
fails on two counts. Secret names are constrained to upper case and underscores, so the name has to
be derived from the App key — and two App keys differing only in punctuation derive the same name and
collide silently, which is a data-loss bug between two unrelated Apps. And a Secret is independently
visible and deletable in the admin secrets list, by an admin who has no way to see that an App
depends on it.

**Consequences to design for:**

- **Writing is one-way.** The key goes in and is never returned. Screens and command output show only
  that a key is present, plus enough of a fingerprint to tell two apart.
- **Rotation replaces it in place** and discards that App's obtained tokens, and only that App's.
- **An audit record is not free here.** Credentials in the shared store get one automatically; this
  does not. Registering, editing, rotating and removing an App must each record who did it.
- **Encryption stops being optional.** See Open questions.

### 5 · Existing deployments carry over by themselves

An installation that configures bot Apps at deploy time keeps working across the upgrade with nobody
doing anything. The first start after upgrading copies each configured App into durable state. From
then on the deployment configuration is ignored and can be deleted.

The four properties the carry-over must hold are in Behaviour, above. The one that is easy to get
wrong: whether the carry-over has run is recorded in its own right, and never inferred from which
Apps exist, or deleting a carried-over App brings it back at the next restart.

**Rejected: reading both sources, with a stored App winning a key collision.** No migration step at
all, and the deployment path keeps working indefinitely. It also leaves the model permanently split —
some Apps editable, some not, for reasons an admin cannot see — and puts precedence evaluation on
every read, which decision 2 made a hot path.

**Rejected: dropping environment support outright, with a documented one-time registration.** The
cleanest end state and the least machinery. It also stops a working bot at the moment of upgrade,
until a human runs a command, which is an outage caused by a housekeeping change.

### 6 · The login App stays where it is

The App users sign in through stays configured by the deployment: present from the moment the
installation starts, editable nowhere. Every other App is registered, edited and removed by an admin.
Both kinds answer the same lookup, because everything that consumes an App consumes it by key.

| | The login App | A bot App |
|---|---|---|
| Configured by | The deployment, as today | An admin, in the UI or from the command line |
| Present when | The installation starts | It is registered |
| Key rotation | Needs a deploy | Immediate, everywhere |
| Shown in the admin list | Yes, marked deployment-configured and not editable | Yes, editable |
| Can be removed | No | Yes |
| Can back a Bot user | Yes, unchanged | Yes |
| Can back the shared installation binding | Yes, unchanged, and this is the common case today | Yes |

Two facts made this the safer choice. The login App is start-up state by design: its client fields are
read from config at boot and the process refuses to start without them when that sign-in method is
selected (`backend/src/config.rs:1247`, `backend/src/auth/routes.rs:78`). And it is how an admin
reaches the UI at all, so a bad edit locks out the only person who could undo it.

**Rejected: every App administered, with the environment kept as a fallback and the App currently used
for signing in protected from deletion.** It gives one consistent list and a way back in. It also
means the sign-in path depends on a database read, and the guard rails are the kind of protection that
has to be right the first time, because being wrong means nobody can log in to fix it.

**Rejected: every App administered, with no special case.** The simplest model to describe, and one
bad edit locks the installation out with no recovery short of editing the database by hand.

**The cost of the choice, stated plainly:** the admin page has one row it cannot edit, and rotating
the login App's signing key still needs a deploy. In exchange, nothing done in the UI can make the UI
unreachable.

### 7 · A stored App is one that has already worked

Registering an App proves it before keeping it. The signing key is used against GitHub immediately,
the App it identifies is confirmed to be the App the admin named, and the display name and bot login
that come back are stored with it. An App that could not be proved is not stored, and the admin is
told which part failed. The failure table is in Behaviour, above.

Rotating a key runs the same proof, and replaces the stored key only once the new one has worked.

The code for this already exists and is already relied on: creating a Bot user resolves the App's name
and bot identity from GitHub using a freshly parsed key
(`backend/src/auth/github_app_service.rs:589-655`), and the admin listing already calls GitHub per App
to list its installations (`backend/src/provider_settings/handlers.rs:290`). The admin screen already
depends on being able to reach GitHub.

**Rejected: storing blind and discovering on first use.** No dependency on GitHub at registration
time. It also reproduces exactly the failure this change exists to remove — a registration that looks
successful and produces a bot that cannot obtain a token.

**Rejected: proving, but storing either way and marking the App unproved.** It keeps registration
possible during a GitHub outage. It costs a state that every screen has to display and every reader
has to decide whether to honour, in exchange for the ability to register an App during a window in
which the App would not work anyway.

### 8 · Apps are installation-wide and admin-owned

There is one set of Apps for the whole installation. Admins register, edit, rotate and remove them.
Everybody else sees the consequences — work attributed to a bot, repositories that can be reached —
and never the Apps themselves. This is the same shape as a Plugin: registered once, then simply
available.

Every operation that touches an App is admin-only today: the listing, the installation binding, and
Bot user creation. The access model has two values, shared and personal (ADR 0006), and this uses the
shared one.

**Rejected: anyone may register, with personal registrations shadowed by shared ones.** A personal App
would have to carry through to its Bot user, which is an installation-wide identity that Loops and
triggers name by id, so work running as somebody's private bot has no meaning today. The access
plumbing would serve a capability nobody has asked for.

### 9 · Two new glossary entries

The glossary has no entry for either concept today. Both go into `CONTEXT.md`, under Integrations.

**GitHub App**:
A machine identity registered at GitHub, holding a signing key this installation stores, from which
short-lived tokens are minted per account it is installed on. One is configured by the deployment and
used for signing in; the rest are registered by admins so work can run as a bot. Referenced by its App
key — the short stable name chosen at registration, which an Adapter connection and a Bot user both
point at.
_Avoid_: bare "app", "integration", "GitHub App connection". Never a bare "key" for the identifier —
that already means a Secret's name and an API key.

**Bot user**:
A user representing a GitHub App's bot identity, which work is attributed to. It never signs in and
holds no personal credential; its usable token is minted from its App on demand, and its name and
address are what commits, pull requests and comments carry.
_Avoid_: service account, machine user, bare "bot".

**Rejected: a third entry for the App's identifier.** It was proposed as "App key", to carry the rule
that the identifier is never a bare "key" — the word already means a Secret's name and an API key, and
here it would sit in the same record as a signing key. The entry was dropped rather than renamed: the
code already names the identifier consistently, and inventing a different domain word for it would put
the glossary and the code in disagreement to solve a problem a sentence solves. The rule survives
inside the GitHub App entry.

### 10 · Registering and staffing are separate acts

Registering an App makes it available. Giving it a bot identity is a second, separate act, because not
every App needs one — an App that exists only so repositories can be reached never runs as anyone.
After a successful registration the admin is shown the bot identity that App would have, and offered
it as the next thing to do.

| The act | What it buys |
|---|---|
| Register the App | Repositories can be reached through it, and it can back the shared installation binding |
| Create its Bot user | Work can be attributed to it — commits, pull requests and comments carry the bot's name, and a Loop or trigger can name it as who to run as |

Creating the Bot user stays repeatable: running it again refreshes the identity from GitHub and does
not produce a second user.

**Rejected: one step, where registering always creates the Bot user.** The shortest path for the common
case. It also leaves an unused identity behind for an App that only backs an installation binding, and
welds together two operations that are useful apart.

**Rejected: two fully separate steps, as the CLI does it today.** Nothing to design, and the admin has
to already know the second step exists — which is the current situation, and there is no UI for it at
all.

## Seams and testing

### The module and its interface

The App registry is the module. Its interface today is a lookup by App key, plus enumeration for the
admin listing, and the fact that an unknown key is a hard error rather than a silent absence.

**That interface does not change.** All four readers keep asking the same question. What moves inside
is where the answer comes from, and what is cached on the way. This is a deepening: callers learn
nothing new, and behaviour that would otherwise spread across them — read the record, decide whether a
cached instance is still valid, decrypt, parse — concentrates in one place.

The interface gains one fact callers must know: a lookup can now fail because durable state could not
be read. That failure is distinct from "no such App", and readers must not collapse the two, because
one means the admin should register something and the other means the installation is unhealthy.

### The seams

| Seam | Adapters | Why it is real |
|---|---|---|
| App record storage | The Postgres store, and an in-memory one for tests | Two adapters, matching the existing repository-trait pattern used for Secrets and stored credentials |
| The proof call to GitHub | Live GitHub, and a stub | Already exists: the API base is overridable by environment variable (`backend/src/adapters/providers/github_pr/api.rs:106`), which is how existing tests exercise GitHub-calling code |
| Encryption | The existing cipher, already a trait with a local implementation | Reused unchanged; no new adapter |

The registry's own lookup interface is the external seam, and it is the test surface. The
record-storage seam is internal to the registry's implementation, used by the registry's own tests.

### Where each behaviour is tested

Highest available seam first, because the alternatives cost more and prove less.

| Tier | Location | What it covers |
|---|---|---|
| Unit | Alongside the registry implementation | An unchanged record keeps its obtained tokens; a changed record discards its own and no other's; an unreadable store is a distinct failure from an absent App; the proof classifies each failure into the message the admin is shown |
| Unit | Alongside the carry-over | Runs once and records it; a deleted App is not resurrected; a bad entry is skipped and named; nothing is dropped silently |
| Integration, against Postgres | `backend/tests/`, behind the `db-tests` feature | Registering, rotating, removing and listing through the real API with the GitHub base pointed at a stub; the carry-over converging to one record when two processes run it together; a Bot user created against a registered App resolving to a mintable credential; removing an App and observing what happens to a Bot user that depended on it |
| Integration, against Postgres | `backend/tests/`, behind the `db-tests` feature | An App registered on one state instance being visible to another instance sharing the database, which is the two-replica case without two processes |

Anything added under `backend/tests/` needs `task test:integration` to compile it; the plain test line
builds almost none of that directory.

**Not covered by the E2E suite, deliberately.** The suite drives the product through the CLI against a
real installation, and every path here needs either a real GitHub App's signing key or a stubbed
GitHub, neither of which that suite has. Registering an App is therefore proved at the integration
tier, and the E2E suite is left alone. This is a stated gap, not an oversight: the surface a person
touches — the admin page — is covered by frontend tests for its own behaviour, and by nothing
end-to-end.

### Locality gained

Today the knowledge "a GitHub App is app id plus signing key plus optional installation, and here is
how you turn that into a usable token" is split between config parsing, two construction sites, the
registry, and the App service. After this change the first three collapse into the registry's
implementation. The two construction sites stop differing, which removes the current situation where
one process holds two registries with two independent caches.

## Out of scope

- Creating the App at GitHub. That stays a task a person does in GitHub's own interface, including
  choosing its permissions and generating its signing key.
- The organisation webhook and its shared secret. Separate lifecycle, separate runbook
  (`docs/runbooks/github-channel-org-webhook.md`).
- How installation tokens are minted, cached, or scoped to a repository owner. Unchanged.
- Which work runs as which bot. Loop and trigger configuration is untouched.
- The App used for signing in. Decision 6.
- Other git hosts. Decision 3b explains why they need nothing here.
- Deleting the dead credential-reference field on Adapter connection rows. Named in Current solution
  so it is not mistaken for a seam this change uses; removing it is its own change.
- Any UI for browsing or editing Bot users beyond creating one from its App.

## Open questions

1. **Encryption is now a hard requirement, and it is optional today.** Decision 4 puts the signing key
   in the App record, encrypted, and encryption is unavailable when an installation has no master key
   configured. Such an installation can then neither register an App nor carry one over from its
   deployment configuration. The choice is between refusing to start when Apps are configured but
   encryption is not, and failing at registration time with a clear reason. This is the one open
   question that changes what an operator sees.
2. **Removing an App that a Bot user depends on.** The Bot user is left holding a credential that can
   no longer mint anything. Either refuse the removal, or define a broken state the admin can see.
   Not blocking: both answers fit the design as settled.
3. **Whether removing an App should also remove its Bot user.** The same question from the other side.
   Leaving the user in place keeps history readable, because past work is attributed to it.

## Ticket breakdown

None.
