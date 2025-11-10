# LikeC4 Architecture Documentation Guide

## Introduction

LikeC4 is an architecture-as-code tool that enables you to design, visualize, and maintain software architecture using a domain-specific language (DSL). Unlike traditional diagramming tools, LikeC4 treats architecture diagrams as source code that can be version-controlled and validated.

### Why LikeC4 for This Project?

- **Version Control**: Architecture diagrams live in Git alongside code
- **Validation**: Syntax checking catches errors before they become diagrams
- **Automatic Updates**: Changes to the model automatically update all views
- **Living Documentation**: Architecture evolves with the codebase
- **Multiple Views**: Create different perspectives from a single model

## Quick Start

### Option 1: Interactive Playground
Visit [playground.likec4.dev](https://playground.likec4.dev) for instant experimentation.

### Option 2: VSCode Extension
Install the LikeC4 extension from the VSCode marketplace for live validation and preview.

### Option 3: CLI Usage (This Project)
```bash
# Already installed in this project
bunx likec4 validate docs/architecture
bunx likec4 preview docs/architecture
```

## The 2-File Approach

**Start simple with just 2 files:**

```
docs/architecture/
├── specs.c4    # Element kinds and relationships
└── model.c4    # Models, relationships, and views
```

This structure is:
- ✅ Easy to validate incrementally
- ✅ Simple to navigate
- ✅ Sufficient for most projects
- ✅ Can be split later if needed

## Your First Diagram: Step-by-Step Tutorial

### Step 1: Create specs.c4 (Minimal)

Start with just 5 element types:

```likec4
specification {
  element actor
  element web_app
  element api_server
  element database
  element external_system

  relationship uses
  relationship calls
}
```

**Validate:** `bunx likec4 validate docs/architecture` ✅

### Step 2: Add Basic Styling

Use only **predefined colors** (hex codes don't work!):

```likec4
specification {
  element actor {
    style {
      shape person
      color blue
    }
  }

  element web_app {
    style {
      shape browser
      color amber
    }
  }

  element api_server {
    style {
      shape rectangle
      color green
    }
  }

  element database {
    style {
      shape cylinder
      color indigo
    }
  }

  element external_system {
    style {
      shape rectangle
      color gray
    }
  }

  relationship uses {
    line dotted
  }

  relationship calls {
    line solid
  }
}
```

**Validate:** ✅

### Step 3: Create model.c4 (Top-Level)

```likec4
model {
  developer = actor 'Developer'

  frontend = web_app 'React Frontend'
  backend = api_server 'Hono Backend'
  db = database 'SQLite Database'

  claude = external_system 'Claude API'

  developer -> frontend
  frontend -> backend
  backend -> db
  backend -> claude
}
```

**Validate:** ✅

### Step 4: Add a Simple View

```likec4
views {
  view index {
    title 'System Context'
    description 'High-level overview'

    include *
  }
}
```

**Validate:** ✅

### Step 5: Add Components with `extend`

```likec4
model {
  developer = actor 'Developer'
  frontend = web_app 'React Frontend'
  backend = api_server 'Hono Backend'
  db = database 'SQLite Database'

  // Add components using extend
  extend backend {
    taskService = service 'Task Service' {
      description 'Manages tasks'
    }

    authService = service 'Auth Service' {
      description 'Handles authentication'
    }
  }

  developer -> frontend
  frontend -> backend
  backend -> db

  // Component relationships
  backend.taskService -> backend.authService
}
```

**Validate:** ✅

### Step 6: Add a Detailed View

```likec4
views {
  view index {
    title 'System Context'
    include *
  }

  view containers {
    title 'Container Architecture'
    include developer
    include frontend, backend, db
    include -> backend ->
  }
}
```

**Validate:** ✅

**Congratulations!** You now have a working, validated architecture.

## Element Types Reference

### Supported Element Kinds

Based on our working implementation:

```likec4
element actor           // People/users
element external_system // External services
element web_app         // Web applications
element api_server      // API servers
element database        // Databases
element file_storage    // File systems
element service         // Services/modules
element routes          // API routes
element coordinator     // Coordinators
element repository      // Repositories
element feature         // Feature modules
element component       // Components
element proxy           // Proxy services
element middleware      // Middleware
```

### Predefined Colors (ONLY These Work!)

LikeC4 **does NOT support hex colors** (`#RRGGBB`). Use these predefined names only:

- `blue` - Primary blue
- `green` - Success green
- `amber` - Warning amber/orange
- `indigo` - Deep purple/indigo
- `gray` - Neutral gray
- `secondary` - Secondary purple
- `muted` - Muted gray
- `primary` - Primary theme color
- `red` - Error red

### Shape Options

```likec4
shape person      // Person icon
shape browser     // Browser window
shape rectangle   // Standard box
shape cylinder    // Database cylinder
shape storage     // Storage icon
shape queue       // Queue icon
shape mobile      // Mobile device
```

### Element Definition Syntax

```likec4
model {
  // Simple element
  user = actor 'End User'

  // Element with properties
  api = api_server 'Backend API' {
    description 'Core REST API'
    technology 'Hono + TypeScript'
  }

  // Element with nested components
  backend = api_server 'Backend' {
    taskService = service 'Task Service' {
      description 'Task management'
    }
  }
}
```

## Relationships Reference

### Basic Relationships

```likec4
model {
  frontend = web_app 'Frontend'
  backend = api_server 'Backend'
  db = database 'Database'

  // Simple relationship
  frontend -> backend

  // With label
  frontend -> backend 'API calls'

  // With type
  frontend -[calls]-> backend

  // With type and label
  frontend -[calls]-> backend 'REST API'

  // Nested element relationships
  backend.taskService -> db 'Persists data'
}
```

### Relationship Types

```likec4
relationship uses {
  line dotted
}

relationship calls {
  line solid
}
```

Keep it simple! Two types are enough for most projects.

## Views Reference

### Basic View Patterns That Work

```likec4
views {
  // Include all top-level elements
  view index {
    title 'System Context'
    include *
  }

  // Include specific elements
  view containers {
    title 'Containers'
    include frontend, backend, db
  }

  // Include with relationships
  view backend_deps {
    title 'Backend Dependencies'
    include backend
    include -> backend ->
  }

  // Scoped view (show all children)
  view of backend {
    title 'Backend Detail'
    include *
    include -> *
  }

  // Auto-layout
  view organized {
    title 'Organized View'
    include *
    autoLayout TopBottom
  }
}
```

### View Include Patterns

```likec4
include *                    // All direct children
include element1, element2   // Specific elements
include -> element           // Elements with relationships TO element
include element ->           // Elements with relationships FROM element
include -> element ->        // All connected elements
```

### ⚠️ Avoid Complex Predicates

These often fail validation:

```likec4
// ❌ Complex filters often don't work
include ** where tag is #backend
include ** where kind is service

// ✅ Keep views simple instead
view backend_services {
  include backend.taskService
  include backend.authService
}
```

## Validation-Driven Development

### The Golden Rule: Validate After Every Change

```bash
# After adding element types
bunx likec4 validate docs/architecture

# After adding models
bunx likec4 validate docs/architecture

# After adding views
bunx likec4 validate docs/architecture
```

### Validation Workflow

1. **Make a small change** (add 1-3 elements)
2. **Validate immediately**
3. **If it fails:** Fix before adding more
4. **If it passes:** Continue with next change

This incremental approach saves hours of debugging!

### Common Validation Errors

```
ERROR: Could not resolve reference to CustomColor named 'emerald'
→ Fix: Use predefined colors only (blue, green, amber, etc.)

ERROR: Expecting token 'EOF' but found '{'
→ Fix: Check for missing closing braces

ERROR: Could not resolve reference to Element named 'xyz'
→ Fix: Ensure element is defined before referencing it

ERROR: FqnRef "element.child" is empty
→ Fix: Simplify view or check element paths
```

## Common Pitfalls

### ❌ Pitfall 1: Custom Hex Colors

**Don't do this:**
```likec4
element service {
  style {
    color #10B981  // ❌ DOESN'T WORK!
  }
}
```

**Do this instead:**
```likec4
element service {
  style {
    color green  // ✅ Works!
  }
}
```

### ❌ Pitfall 2: Deep Nesting

**Avoid:**
```likec4
model {
  app = system 'App' {
    backend = component 'Backend' {
      domain = component 'Domain' {
        service = component 'Service' {
          // Too deep! Hard to reference, often fails
        }
      }
    }
  }
}
```

**Better:**
```likec4
model {
  backend = api_server 'Backend'

  extend backend {
    taskService = service 'Task Service'
    authService = service 'Auth Service'
  }
}
```

### ❌ Pitfall 3: Complex View Predicates

**Avoid:**
```likec4
view complex {
  include ** where tag is #backend and kind is service
  // Often fails validation
}
```

**Better:**
```likec4
view backend_services {
  include backend.taskService
  include backend.authService
  include backend.previewService
}
```

### ❌ Pitfall 4: Tags Inside Element Blocks

**Don't do this:**
```likec4
model {
  api = service 'API' {
    description 'API service'
    #backend  // ❌ Tags not supported here
  }
}
```

**Keep element definitions simple - tags are optional.**

### ❌ Pitfall 5: Colored Elements in Dynamic Views

When elements have colors defined in `specs.c4`, those colors carry over to dynamic sequence diagrams, creating visual clutter.

**The Problem:**
```likec4
// specs.c4
element component {
  style {
    shape rectangle
    color green        // ❌ This color shows in ALL views
  }
}

element feature {
  style {
    shape rectangle
    color amber        // ❌ Brown/orange in sequence diagrams
  }
}
```

Result in dynamic views: Components appear green, features appear brown, creating a distracting rainbow effect in sequence diagrams.

**The Solution:**

Remove colors from component/feature elements in `specs.c4`:

```likec4
// specs.c4 - Use neutral elements
element component {
  style {
    shape rectangle
    // No color - elements appear neutral
  }
}

element feature {
  style {
    shape rectangle
    // No color - elements appear neutral
  }
}
```

Apply colors at the **view level** instead:

```likec4
// model.c4 - Apply colors only where needed
dynamic view myFlow {
  user -> frontend -> backend

  include user, frontend, backend

  // Apply hierarchical styling
  style frontend._, backend._ {
    color secondary
    opacity 20%
  }
  style frontend, backend {
    color muted
    opacity 15%
  }
}
```

**When to use spec-level vs view-level colors:**

**Spec-Level Colors** (in `specs.c4`):
- Use for: actors, external systems, databases (always have consistent meaning)
- Example: `actor` is always blue, `database` is always indigo

**View-Level Colors** (in views):
- Use for: components, features, services (meaning varies by context)
- Allows: neutral appearance in most views, emphasis where needed

**Best Practice**: Keep `component` and `feature` elements neutral in specs, apply colors in specific views for hierarchy visualization.

## Progressive Enhancement

### Phase 1: Minimal (Start Here)

**Files:** 2 (specs.c4 + model.c4)
**Elements:** 3-5 containers
**Views:** 1 system context view
**Time:** 30 minutes

```likec4
// specs.c4
specification {
  element actor
  element web_app
  element api_server
  element database

  relationship uses
}

// model.c4
model {
  user = actor 'User'
  frontend = web_app 'Frontend'
  backend = api_server 'Backend'
  db = database 'Database'

  user -> frontend
  frontend -> backend
  backend -> db
}

views {
  view index {
    include *
  }
}
```

### Phase 2: Detailed (Add Components)

**Elements:** 10-15 with nested structure
**Views:** 2-3 views
**Time:** +1 hour

```likec4
// Add to model.c4
extend backend {
  taskService = service 'Task Service'
  authService = service 'Auth Service'
}

extend frontend {
  tasksFeature = feature 'Tasks Feature'
  authFeature = feature 'Auth Feature'
}

// Add relationships
frontend.tasksFeature -> backend.taskService
frontend.authFeature -> backend.authService

views {
  view index { include * }

  view containers {
    include frontend, backend, db
    include -> backend ->
  }

  view of backend {
    include *
    include -> *
  }
}
```

### Phase 3: Comprehensive (Add External Systems)

**Elements:** 20+ including external systems
**Views:** 4-5 views with specific focuses
**Time:** +2 hours

See our working implementation in `docs/architecture/` for a complete example.

## Advanced Topics

### When to Use Multi-File Organization

**Consider splitting into multiple files when:**
- You have >200 lines in a single file
- You have >10 containers with components
- Multiple teams need to update architecture independently

**Split pattern:**
```
docs/architecture/
├── specs.c4           # Shared definitions
├── landscape.c4       # System context
├── frontend.c4        # Frontend model
├── backend.c4         # Backend model
└── views.c4           # All views
```

**Keep it simple:** Most projects don't need this complexity.

### View of Pattern

For detailed component views:

```likec4
view of backend.previewFeature {
  title 'Preview Feature Architecture'
  description 'Internal components and data flow'

  include *          // All children of previewFeature
  include -> *       // All elements calling children
  include frontend.previewFeature  // Related frontend

  autoLayout TopBottom
}
```

### Multi-Level Hierarchical Views

To show **3+ level hierarchy** (System > Feature > Component) with visible grouping boxes, you must explicitly include intermediate containers.

#### The Problem

When you only include the top-level container, LikeC4 shows all nested children flat:

```likec4
view flat {
  include backend  // Shows: Backend > all components (flat)
}
```

Result: `Backend > Preview Routes` (skips the "Preview Feature" level)

#### The Solution

Explicitly include **intermediate containers** to create visible grouping boxes:

```likec4
view hierarchical {
  include backend                    // Level 1: Top container
  include backend.previewFeature     // Level 2: Feature container (EXPLICIT!)
  // Level 3: Components (included automatically as children)
}
```

Result: `Backend > Preview Feature > Preview Routes` ✅

#### Complete Example from Our Project

```likec4
view of backend.previewFeature {
  title 'Preview Feature with Full Hierarchy'

  include backend                    // Show backend container
  include backend.previewFeature     // Show preview feature container
  // Components shown automatically: previewRoutes, previewOrchestrator, etc.

  include frontend.previewFeature    // Related frontend

  autoLayout TopBottom
}
```

This creates **nested grouping boxes**:
```
┌─ Hono Backend ──────────────────────────┐
│  ┌─ Preview Feature ──────────────────┐ │
│  │  • Preview Routes                  │ │
│  │  • Preview Orchestrator            │ │
│  │  • Preview Runtime Service         │ │
│  │  • ...other components             │ │
│  └────────────────────────────────────┘ │
│  • Task Service                         │
│  • Other direct children                │
└─────────────────────────────────────────┘
```

#### Key Rules

1. **Explicit is better than implicit** - Include intermediate containers explicitly
2. **Order matters** - Include parent before children for proper nesting
3. **Children are automatic** - You don't need to list individual components if the parent is included
4. **Works in all view types** - Applies to normal views, `view of`, and dynamic views

### Auto-Layout Options

```likec4
autoLayout TopBottom      // Vertical flow
autoLayout LeftRight      // Horizontal flow
autoLayout TopBottom 120 110  // With custom spacing
```

### View-Level Styling Patterns

LikeC4 allows you to style elements **within views** to create visual hierarchy and emphasize specific components without changing the global element definitions.

#### When to Use View-Level Styling

- **Emphasize grouping boxes** in hierarchical views
- **Differentiate hierarchy levels** with opacity
- **Override default colors** for specific views
- **Create visual distinction** without modifying specs.c4

#### Basic View-Level Styling

Style specific elements within a view:

```likec4
view myView {
  include backend
  include frontend

  style frontend {
    color amber
  }
  style backend {
    color green
  }
}
```

#### Styling All Children with `._`

Use the `._` notation to style all children of a container:

```likec4
view hierarchical {
  include backend
  include backend.previewFeature

  // Style all direct children of backend
  style backend._ {
    color secondary
    opacity 20%
  }

  // Style all children of previewFeature
  style backend.previewFeature._ {
    color secondary
    opacity 20%
  }
}
```

#### Opacity Patterns for Visual Hierarchy

Create subtle visual hierarchy using opacity:

```likec4
view hierarchical {
  include frontend
  include backend
  include backend.previewFeature

  // Children boxes (more visible)
  style frontend._, backend._ {
    color secondary
    opacity 20%
  }

  style backend.previewFeature._ {
    color secondary
    opacity 20%
  }

  // Container boxes (subtle background)
  style frontend, backend {
    color muted
    opacity 15%
  }

  style backend.previewFeature {
    color muted
    opacity 15%
  }
}
```

This creates layered grouping boxes:
- **Container boxes**: 15% opacity (subtle background)
- **Children boxes**: 20% opacity (slightly more visible)
- **Actual components**: Full opacity (most prominent)

#### Complete Example with Hierarchy

```likec4
view systemArchitecture {
  title 'System Architecture with Visual Hierarchy'

  include developer
  include frontend
  include backend
  include backend.previewFeature
  include github, storage

  // Style actor
  style developer {
    color green
  }

  // Style hierarchy levels
  style frontend._, backend._ {
    color secondary
    opacity 20%
  }

  style backend.previewFeature._ {
    color secondary
    opacity 20%
  }

  style frontend, backend {
    color muted
    opacity 15%
  }

  style backend.previewFeature {
    color muted
    opacity 15%
  }

  autoLayout TopBottom
}
```

#### Spec-Level vs View-Level Styling

**Spec-Level** (in `specs.c4`):
- Applies **globally** to all views
- Good for: consistent colors across all diagrams
- Use for: element types that always have the same role

**View-Level** (in `model.c4`):
- Applies **only to that view**
- Good for: emphasizing specific aspects in different views
- Use for: context-specific styling, hierarchy visualization

**Best Practice**: Use minimal spec-level colors (or none) for components, apply view-level styling for visual hierarchy.

### Dynamic Views and Sequence Diagrams

Dynamic views show **interaction flows** and **sequences** between components over time, unlike static views which show structural relationships.

#### What Are Dynamic Views?

Dynamic views create sequence diagrams that show:
- **Interactions** between components (messages, calls, data flow)
- **Temporal ordering** (what happens first, second, third)
- **Notes and annotations** explaining each step
- **Parallel operations** that happen simultaneously

#### When to Use Dynamic Views

Use dynamic views to document:
- User journeys and workflows
- API call sequences
- System startup/shutdown flows
- Request/response cycles
- Complex multi-step processes

#### Basic Syntax

```likec4
dynamic view flowName {
  title 'Flow Title'
  description 'What this flow represents'

  // Simple interaction
  user -> frontend 'opens app'

  // Interaction with notes
  frontend -> backend 'GET /api/data' {
    notes 'Fetches user data with authentication'
  }

  // Response
  frontend <- backend 'returns JSON'
}
```

#### Element Ordering (Critical!)

**The order of `include` statements controls left-to-right positioning:**

```likec4
dynamic view ordered {
  user -> frontend -> backend -> database

  // LEFT to RIGHT positioning:
  include user          // Far left
  include frontend      // Second from left
  include backend       // Third from left
  include database      // Far right
}
```

**Best Practice**: Include elements in the order they appear in the flow (left to right).

#### Actor Positioning

Always place actors (users, developers) **first** in the include list:

```likec4
dynamic view withActor {
  developer -> frontend -> backend

  // Correct: Actor first
  include developer     // ✅ Far left (actor position)
  include frontend
  include backend
}
```

#### Interaction Notes

Add context to each step with `notes`:

```likec4
dynamic view documented {
  user -> auth 'login' {
    notes '''
      - User enters credentials
      - Client validates format
      - Sends encrypted password
    '''
  }

  auth -> database 'verify' {
    notes 'Checks credentials against stored hash'
  }
}
```

Use single quotes for inline notes, triple quotes for multi-line.

#### Parallel Operations

Show operations that happen simultaneously:

```likec4
dynamic view parallel {
  api -> database 'get data'

  parallel {
    api -> cache 'check cache' {
      notes 'Fast path for cached data'
    }
    api -> search 'index data' {
      notes 'Update search index asynchronously'
    }
  }

  api -> api 'process results'
}
```

#### Complete Example: Preview Server Startup

```likec4
dynamic view previewServerStartup {
  title 'Preview Server Startup Sequence'
  description 'Shows the complete flow of starting a preview server'

  // User initiates
  developer -> frontend 'clicks Start Preview'

  // API call
  frontend -> backend 'POST /api/v1/tasks/:taskId/server/start' {
    notes 'Returns 202 Accepted immediately'
  }

  // Orchestration
  backend -> workspaceCoordinator 'ensureWorkspace()' {
    notes 'Provisions workspace if needed'
  }

  workspaceCoordinator -> github 'git clone' {
    notes 'Clones repository to local workspace'
  }

  workspaceCoordinator -> storage 'saves workspace' {
    notes 'Persists workspace snapshot'
  }

  // Start server
  backend -> previewServer 'spawn process' {
    notes 'Executes configured start command'
  }

  previewServer -> previewServer 'listens on port' {
    notes 'Server starts and emits ready signal'
  }

  // Event notification
  backend -> eventService 'emit(started)' {
    notes 'Notifies subscribers via SSE'
  }

  eventService -> frontend 'SSE: server started' {
    notes 'Real-time update to UI'
  }

  // Include elements (order = left-to-right positioning)
  include developer
  include frontend
  include backend
  include workspaceCoordinator
  include eventService
  include github, storage

  // Apply hierarchical styling
  style frontend._, backend._ {
    color secondary
    opacity 20%
  }
  style frontend, backend {
    color muted
    opacity 15%
  }
}
```

#### Self-Interactions

Components can interact with themselves to show internal processing:

```likec4
dynamic view processing {
  api -> api 'validates input' {
    notes 'Internal validation logic'
  }

  api -> database 'stores data'

  api -> api 'transforms result' {
    notes 'Maps database model to API response'
  }
}
```

#### Navigation Between Views

Link to other views from notes:

```likec4
dynamic view main {
  user -> auth 'authenticate' {
    navigateTo authFlow
    notes 'See detailed auth flow'
  }
}

dynamic view authFlow {
  title 'Detailed Authentication Flow'
  // ... detailed steps
}
```

#### Multi-Level Hierarchy in Dynamic Views

Just like static views, you can show hierarchical grouping boxes:

```likec4
dynamic view hierarchical {
  frontend -> backend.previewFeature.previewRoutes 'API call'

  backend.previewFeature.previewRoutes -> backend.previewFeature.previewOrchestrator

  // Show 3-level hierarchy
  include frontend
  include backend
  include backend.previewFeature  // Explicit intermediate container!

  // Style hierarchy
  style backend.previewFeature._ {
    color secondary
    opacity 20%
  }
  style backend.previewFeature {
    color muted
    opacity 15%
  }
}
```

#### Best Practices for Dynamic Views

1. **Keep flows focused** - One user story or process per view
2. **Order includes logically** - Left-to-right matches the flow
3. **Use notes liberally** - Explain why, not just what
4. **Group related interactions** - Use parallel blocks for simultaneous operations
5. **Show hierarchy explicitly** - Include intermediate containers for grouping boxes
6. **Apply consistent styling** - Use opacity patterns for visual hierarchy

#### Dynamic vs Static Views

| Feature | Static Views | Dynamic Views |
|---------|--------------|---------------|
| **Purpose** | Show structure | Show behavior/flow |
| **Elements** | Containers, components | Same + interactions |
| **Layout** | Spatial arrangement | Temporal sequence |
| **Best for** | Architecture overview | User journeys, workflows |
| **Syntax** | `view { include ... }` | `dynamic view { A -> B }` |

## Working Examples from This Project

### Example 1: Minimal System (What We Started With)

```likec4
specification {
  element actor
  element external_system
  element web_app
  element api_server
  element database
  element file_storage

  relationship uses
  relationship calls
}

model {
  developer = actor 'Developer'

  frontend = web_app 'React Frontend'
  backend = api_server 'Hono Backend'
  db = database 'SQLite Database'
  storage = file_storage 'File Storage'

  developer -> frontend
  frontend -> backend
  backend -> db
  backend -> storage
}

views {
  view index {
    title 'System Context'
    include *
  }
}
```

**Result:** Validates ✅ - Clean foundation

### Example 2: With External Systems

```likec4
model {
  developer = actor 'Developer'

  frontend = web_app 'React Frontend'
  backend = api_server 'Hono Backend'
  db = database 'SQLite Database'
  storage = file_storage 'File Storage'

  // External Systems
  claude = external_system 'Anthropic Claude API' {
    description 'AI model for code generation'
  }

  github = external_system 'GitHub' {
    description 'Repository hosting'
  }

  auth0 = external_system 'Auth0' {
    description 'Authentication service'
  }

  // Relationships
  developer -> frontend
  frontend -> backend
  backend -> db
  backend -> storage
  backend -> claude
  backend -> github
  backend -> auth0
}

views {
  view index {
    title 'System Context'
    include *
    include -> backend ->
  }

  view containers {
    title 'Container Architecture'
    include developer
    include frontend, backend, db, storage
    include claude, github, auth0
    include -> backend ->
  }
}
```

**Result:** Validates ✅ - Shows external dependencies

### Example 3: Detailed Preview Feature (Current Implementation)

```likec4
model {
  backend = api_server 'Hono Backend'

  // Detailed feature structure
  extend backend {
    previewFeature = feature 'Preview Feature' {
      description 'Live preview of running applications'

      previewRoutes = routes 'Preview Routes' {
        description 'API endpoints'
      }

      previewOrchestrator = coordinator 'Preview Orchestrator' {
        description 'Orchestrates lifecycle'
      }

      previewRuntimeService = service 'Preview Runtime Service' {
        description 'Manages runtime'
      }

      previewProxyService = proxy 'Preview Proxy Service' {
        description 'Proxies requests'
      }

      hostnameProxyService = proxy 'Hostname Proxy Service' {
        description 'Hostname-based routing'
      }

      previewServer = component 'Preview Server' {
        description 'Running application instance'
      }

      htmlInjector = middleware 'HTML Injector' {
        description 'Injects inspector script'
      }
    }
  }

  // Internal relationships
  backend.previewFeature.previewRoutes -> backend.previewFeature.previewOrchestrator
  backend.previewFeature.previewOrchestrator -> backend.previewFeature.previewRuntimeService
  backend.previewFeature.previewOrchestrator -> backend.previewFeature.previewProxyService
  backend.previewFeature.previewRuntimeService -> backend.previewFeature.previewServer
  backend.previewFeature.previewProxyService -> backend.previewFeature.hostnameProxyService
  backend.previewFeature.previewProxyService -> backend.previewFeature.htmlInjector
  backend.previewFeature.htmlInjector -> backend.previewFeature.previewServer
}

views {
  view of backend.previewFeature {
    title 'Preview Feature Architecture'
    description 'Detailed view of components and data flow'

    include *
    include -> *
    include frontend.previewFeature

    autoLayout TopBottom
  }
}
```

**Result:** Validates ✅ - Complete feature with 7 components and relationships

## CLI Reference

### Validation
```bash
# Validate all .c4 files
bunx likec4 validate docs/architecture

# Output:
# INFO  likec4.lang version 1.43.0
# INFO  likec4.lang workspace: found 2 source files
# ✅ Success = No errors shown
```

### Preview
```bash
# Start interactive preview server
bunx likec4 preview docs/architecture

# Opens browser with interactive diagrams
# - Navigate between views
# - Zoom and pan
# - Click elements for details
```

### Build
```bash
# Generate static site
bunx likec4 build docs/architecture -o ./dist

# Creates HTML/JS for hosting
```

### Export
```bash
# Export to PNG
bunx likec4 export docs/architecture -o architecture.png

# Export specific view
bunx likec4 export docs/architecture --view containers -o containers.png
```

## VSCode Extension

**Features:**
- Live validation as you type
- Syntax highlighting
- Auto-completion
- Inline preview
- Quick navigation

**Install:** Search "LikeC4" in VSCode Extensions

## Resources

### Official Documentation
- **Website**: [likec4.dev](https://likec4.dev)
- **Playground**: [playground.likec4.dev](https://playground.likec4.dev)
- **GitHub**: [github.com/likec4/likec4](https://github.com/likec4/likec4)
- **Examples**: [likec4.dev/examples](https://likec4.dev/examples)

### This Project
- **Specs**: `docs/architecture/specs.c4`
- **Model**: `docs/architecture/model.c4`
- **README**: `docs/architecture/README.md`

## Quick Reference Card

### Predefined Colors
`blue`, `green`, `amber`, `indigo`, `gray`, `secondary`, `muted`, `primary`, `red`

### Common Shapes
`person`, `browser`, `rectangle`, `cylinder`, `storage`, `queue`, `mobile`

### Basic View Patterns
```likec4
include *                // All children
include element1, element2  // Specific
include -> element ->    // Connected
view of element { ... }  // Scoped
autoLayout TopBottom     // Layout
```

### Validation Command
```bash
bunx likec4 validate docs/architecture
```

### File Structure
```
docs/architecture/
├── specs.c4    # Element kinds & relationships
└── model.c4    # Models & views
```

## Summary

**Start simple:**
1. Create `specs.c4` with 5 element types
2. Create `model.c4` with 3-4 containers
3. Add one simple view
4. **Validate** ✅
5. Add components incrementally
6. **Validate after each change** ✅

**Remember:**
- ✅ Use predefined colors only
- ✅ Keep hierarchies flat (2-3 levels)
- ✅ Validate frequently
- ✅ Start with 2 files
- ✅ Simple views win
- ❌ No hex colors
- ❌ No deep nesting
- ❌ No complex predicates

**Master the basics before going advanced.**

The architecture in `docs/architecture/` validates successfully and serves as a complete, working reference.
