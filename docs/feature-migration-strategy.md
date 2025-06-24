# Feature-by-Feature Migration Strategy

## Overview

This document outlines the detailed migration strategy for transitioning from `@client/src/types/` to `@shared/types/` across all client features. The strategy is organized by feature module with specific steps, risk assessments, and validation criteria.

## Migration Principles

1. **Risk-Based Approach**: Start with lowest risk, highest impact changes
2. **Feature Isolation**: Migrate one feature at a time to minimize blast radius
3. **Incremental Validation**: Test thoroughly after each feature migration
4. **Backward Compatibility**: Maintain existing functionality during transition
5. **Type Safety First**: Ensure TypeScript compilation success at each step

## Overall Migration Phases

### Phase 1: Foundation Setup
- [ ] Create transformation layer utilities
- [ ] Set up client-side type definitions
- [ ] Update build configuration for shared types
- [ ] Create migration testing framework

### Phase 2: Core Infrastructure (Week 1)
- [ ] Tasks feature (highest complexity, most dependencies)
- [ ] Event system (core to real-time functionality)

### Phase 3: Supporting Features (Week 2)
- [ ] Environment management
- [ ] Settings and configuration

### Phase 4: Specialized Features (Week 3)
- [ ] Agent integration
- [ ] Claude Code specific features

### Phase 5: Cleanup & Optimization (Week 4)
- [ ] Remove legacy type definitions
- [ ] Optimize transformation performance
- [ ] Documentation updates

---

## Feature-Specific Migration Plans

## 1. Tasks Feature Migration

**Priority**: 🔴 Critical (Week 1 - Days 1-3)
**Complexity**: High
**Risk Level**: Medium-High
**Files Affected**: 26 files

### Current State Analysis
- **26 files** using task-related types
- Heavy usage of `TaskMetadata`, `Task`, `TaskIteration`
- Complex event handling and real-time updates
- Manual transformation in `TasksIndex` route
- Mixed camelCase/snake_case property access

### Migration Steps

#### Step 1: Update Type Imports (Day 1)
```bash
# Priority order for file updates:
1. client/src/lib/api/tasks.ts                    # API layer
2. client/src/routes/tasks/index.tsx              # Main tasks route  
3. client/src/features/tasks/hooks/use-tasks.ts   # Core hooks
4. client/src/features/tasks/contexts/task-context.tsx # Context provider
```

**Changes Required**:
```typescript
// Before
import type { TaskMetadata, Task } from '@/types/task'

// After  
import type { TaskMetadata, Task } from '@shared/types/task'
```

#### Step 2: Property Access Updates (Day 2)
Replace snake_case access patterns:
```typescript
// Before
metadata.task_id
metadata.created_at  
metadata.agent_type
metadata.num_turns

// After (using transformation layer)
metadata.taskId
metadata.createdAt
metadata.agentType  
metadata.numTurns
```

#### Step 3: Component Updates (Day 2-3)
**High Priority Components**:
1. `TaskHeader` - Direct metadata access, high visibility
2. `TaskViewer` - Core task display logic
3. `EventList` - Real-time event rendering  
4. `TasksIndex` - Main route with manual transformations

**Medium Priority Components**:
5. `TaskChatInput` - User interaction component
6. `EventViewer` - Event detail display
7. Test files and utilities

#### Step 4: Validation Criteria
- [ ] All task routes load without errors
- [ ] Task metadata displays correctly (dates, status, costs)
- [ ] Real-time events continue working
- [ ] Task creation/updates function properly
- [ ] All tests pass

### Specific File Changes

**`client/src/routes/tasks/index.tsx`**:
- Remove manual transformation code
- Use transformed data from API layer
- Update property access to camelCase

**`client/src/features/tasks/contexts/task-context.tsx`**:
- Update type imports
- Ensure SSE event handling works with new types
- Validate real-time updates

**`client/src/features/tasks/components/task-header.tsx`**:
- Replace all snake_case property access
- Update time formatting logic for Date objects
- Test cost display formatting

### Risk Mitigation
- **Real-time Events**: Test SSE connections thoroughly
- **Date Handling**: Validate all date display and formatting
- **Cost Calculations**: Verify monetary amount displays

---

## 2. Event System Migration

**Priority**: 🔴 Critical (Week 1 - Days 4-5)  
**Complexity**: High
**Risk Level**: High (affects real-time functionality)
**Files Affected**: 21 files

### Current State Analysis
- **21 files** using event types
- Core to real-time task updates
- Complex event merging logic
- Multiple event type handlers
- SSE connection management

### Migration Steps

#### Step 1: Event Type Definitions (Day 4 Morning)
```typescript
// Update imports across all event-related files
import type { Event, EventPayload } from '@shared/types/event'
```

#### Step 2: Event Transformer Integration (Day 4 Afternoon)
- Implement event transformation in API layer
- Update SSE connection handling
- Test event streaming with new types

#### Step 3: Event Component Updates (Day 5)
**Critical Components**:
1. `EventList` - Core event rendering
2. `MessageEvent` - Message display logic
3. `ToolCallEvent` - Tool execution display  
4. `WorkspaceEvent` - Workspace status updates

#### Step 4: Event Utilities (Day 5)
- Update `merge-tool-events.ts` utility
- Fix event filtering and sorting logic
- Update event parsing functions

### Validation Criteria
- [ ] Real-time events stream correctly
- [ ] All event types display properly  
- [ ] Event merging logic works
- [ ] SSE connections remain stable
- [ ] Event history loads correctly

### High-Risk Areas
- **SSE Connections**: Real-time streaming must not break
- **Event Merging**: Complex logic for combining tool events
- **Event Parsing**: Multiple event type handlers

---

## 3. Environment Management Migration

**Priority**: 🟡 Medium (Week 2 - Days 1-2)
**Complexity**: Medium  
**Risk Level**: Low-Medium
**Files Affected**: 12 files

### Current State Analysis
- **12 files** using environment types
- CRUD operations for environment management
- Form validation and submission
- API integration for environment settings

### Migration Steps

#### Step 1: Environment Type Updates (Day 1)
**Files to Update**:
```
client/src/features/environments/api/environments.ts
client/src/features/environments/hooks/use-environments.ts
client/src/features/environments/components/environment-form.tsx
client/src/features/environments/components/environment-list.tsx
```

#### Step 2: Form Component Updates (Day 1-2)
- Update form field mappings
- Fix validation logic for new types
- Test environment creation/editing

#### Step 3: Settings Integration (Day 2)
- Update settings page environment components
- Ensure environment selection works
- Test environment configuration flow

### Validation Criteria
- [ ] Environment list loads correctly
- [ ] Environment creation/editing works
- [ ] Form validation functions properly
- [ ] Environment selection in tasks works
- [ ] Settings page displays environments

### Risk Mitigation
- **Form Validation**: Test all validation rules
- **Environment Settings**: Verify configuration persistence

---

## 4. Settings & Configuration Migration

**Priority**: 🟡 Medium (Week 2 - Day 3)
**Complexity**: Low-Medium
**Risk Level**: Low  
**Files Affected**: 6 files

### Current State Analysis
- **6 files** using settings-related types
- Environment management within settings
- Configuration persistence
- User preference handling

### Migration Steps

#### Step 1: Settings Component Updates
**Files to Update**:
```
client/src/features/settings/components/environment-form.tsx
client/src/features/settings/components/environment-list.tsx  
client/src/features/settings/hooks/use-settings.ts
```

#### Step 2: Configuration Logic
- Update settings storage/retrieval
- Fix environment configuration UI
- Test settings persistence

### Validation Criteria
- [ ] Settings page loads without errors
- [ ] Environment configuration works
- [ ] Settings persist correctly
- [ ] All preference changes save

---

## 5. Agent Integration Migration

**Priority**: 🟢 Low (Week 3 - Day 1)
**Complexity**: Low
**Risk Level**: Low
**Files Affected**: 2 files

### Current State Analysis
- **2 files** using agent types
- Agent configuration and triggers
- API integration for agent operations

### Migration Steps

#### Step 1: Agent Type Updates
**Files to Update**:
```
client/src/features/agents/api/agents.ts
client/src/features/agents/hooks/use-agent-trigger.ts
```

#### Step 2: Agent Configuration
- Update agent option handling
- Test agent trigger functionality

### Validation Criteria
- [ ] Agent operations work correctly
- [ ] Agent configuration persists
- [ ] Trigger functionality operates

---

## 6. Claude Code Features Migration

**Priority**: 🟢 Low (Week 3 - Day 2)
**Complexity**: Low
**Risk Level**: Low  
**Files Affected**: 2 files

### Current State Analysis
- **2 files** using Claude Code specific types
- These types may need to remain client-specific
- Custom Claude Code integration logic

### Migration Steps

#### Step 1: Evaluate Shared Type Availability
- Determine if Claude Code types exist in shared
- Keep client-specific types if needed

#### Step 2: Update Integration Logic
- Update API calls and hooks
- Test Claude Code functionality

### Validation Criteria
- [ ] Claude Code integration works
- [ ] All Claude Code features function
- [ ] Configuration and triggers operate

---

## Testing Strategy

### Automated Testing
```bash
# Run after each feature migration
pnpm test                    # Unit tests
pnpm test:coverage          # Coverage validation  
pnpm type:check             # TypeScript validation
pnpm biome:check            # Linting and formatting
```

### Manual Testing Checklist

#### After Tasks Migration:
- [ ] Load tasks list page
- [ ] Create new task
- [ ] View task details
- [ ] Check real-time event updates
- [ ] Verify task status changes
- [ ] Test task metadata display

#### After Events Migration:
- [ ] Monitor event streaming
- [ ] Check all event type displays
- [ ] Verify event merging
- [ ] Test event history loading

#### After Environment Migration:
- [ ] Create/edit environments
- [ ] Test environment selection
- [ ] Verify environment settings

#### Integration Testing:
- [ ] Full task workflow (create → run → view results)
- [ ] Real-time updates across all features  
- [ ] Cross-feature data consistency

### Rollback Plan

For each feature migration:
1. **Git Branch Strategy**: Create feature branch for each migration
2. **Backup Points**: Tag before each major change
3. **Quick Rollback**: Keep old type imports commented for quick revert
4. **Validation Gates**: Don't proceed to next feature until current passes all tests

### Success Metrics

- [ ] **Zero TypeScript Errors**: All code compiles successfully
- [ ] **Test Coverage Maintained**: No decrease in test coverage  
- [ ] **Performance Maintained**: No significant performance degradation
- [ ] **Functionality Preserved**: All existing features work identically
- [ ] **Real-time Features**: SSE connections and live updates function properly

### Post-Migration Cleanup

#### Week 4: Optimization & Documentation
1. **Remove Legacy Code**:
   - Delete old type definitions from `client/src/types/`
   - Remove manual transformation code
   - Clean up unused imports

2. **Performance Optimization**:
   - Optimize transformation layer performance
   - Add caching for frequently transformed data
   - Monitor bundle size impact

3. **Documentation Updates**:
   - Update API documentation
   - Create migration completion report
   - Document new type system usage

4. **Team Training**:
   - Share type system guidelines
   - Document transformation layer usage
   - Create troubleshooting guide

This migration strategy provides a systematic approach to transitioning the entire client codebase to use shared types while maintaining stability and functionality throughout the process.