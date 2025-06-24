# Complete Type Migration Plan: Client to Shared Types

## Executive Summary

This document presents a comprehensive plan for migrating from client-specific types (`@client/src/types/`) to shared types (`@shared/types/`) while transitioning from snake_case API responses to camelCase client conventions.

## Key Findings from Research

### 1. **Current State Analysis**
- **40 files** across client features require migration
- **Mixed naming conventions**: snake_case (API) ↔ camelCase (client)
- **Inconsistent transformations**: Manual, scattered throughout components
- **Type mismatches**: TypeScript types don't match actual API responses

### 2. **Impact Assessment**
- **Tasks Feature**: 26 files (highest complexity, most critical)
- **Events System**: 21 files (real-time functionality at risk)
- **Environment Management**: 12 files (medium complexity)
- **Settings & Configuration**: 6 files (low complexity)
- **Agent Integration**: 2 files (low complexity)
- **Claude Code Features**: 2 files (may remain client-specific)

### 3. **Technical Challenges Identified**
- **Real-time Event Streaming**: SSE connections must not break
- **GitDiff Rendering**: Structural differences between client/shared types
- **Date Handling**: String timestamps → Date objects throughout
- **Property Transformations**: 15+ field mappings (task_id → taskId, etc.)

## Migration Strategy Overview

### **4-Week Phased Approach**

#### **Week 1: Core Infrastructure** 🔴 Critical
- **Days 1-3**: Tasks feature migration (26 files)
- **Days 4-5**: Event system migration (21 files)
- **Risk**: High (affects core functionality)
- **Validation**: Extensive testing of real-time features

#### **Week 2: Supporting Features** 🟡 Medium
- **Days 1-2**: Environment management (12 files)
- **Day 3**: Settings & configuration (6 files)
- **Risk**: Medium (affects user workflows)
- **Validation**: Feature-specific testing

#### **Week 3: Specialized Features** 🟢 Low
- **Day 1**: Agent integration (2 files)
- **Day 2**: Claude Code features (2 files)
- **Risk**: Low (limited scope)
- **Validation**: Integration testing

#### **Week 4: Optimization & Cleanup** ✨ Enhancement
- Remove legacy type definitions
- Performance optimization
- Documentation updates
- Team training

## Technical Architecture

### **Transformation Layer Design**

```typescript
// Centralized API transformation at client boundary
client/src/lib/api/
├── transformers/
│   ├── task-transformer.ts      # TaskMetadata snake_case → camelCase
│   ├── event-transformer.ts     # Event transformation + date handling  
│   ├── environment-transformer.ts
│   └── base-transformer.ts      # Common utilities
├── client.ts                    # HTTP client with auto-transformation
└── types.ts                     # Client-side camelCase types
```

### **Key Transformations**

| API Response (snake_case) | Client Usage (camelCase) |
|---------------------------|--------------------------|
| `task_id` | `taskId` |
| `created_at` | `createdAt` |
| `last_message_at` | `lastMessageAt` |
| `agent_type` | `agentType` |
| `environment_id` | `environmentId` |
| `num_turns` | `numTurns` |
| `total_cost_usd` | `totalCostUsd` |
| `event_id` | `eventId` |
| `workspace_status` | `workspaceStatus` |

## Risk Assessment & Mitigation

### **High-Risk Areas**
1. **Real-time Event Streaming** 
   - Risk: SSE connections break
   - Mitigation: Extensive testing, gradual rollout
   
2. **GitDiff Rendering**
   - Risk: Structural differences cause display issues
   - Mitigation: Custom transformation layer, UI testing

3. **Date Handling**
   - Risk: Date parsing/formatting breaks
   - Mitigation: Centralized date utilities, validation

### **Medium-Risk Areas**
1. **Form Validation** (Environment/Settings)
   - Risk: Validation rules fail with new types
   - Mitigation: Test all form scenarios

2. **API Integration**
   - Risk: Request/response type mismatches
   - Mitigation: TypeScript strict mode, integration tests

### **Low-Risk Areas**
1. **Agent Operations** (limited scope)
2. **Claude Code Features** (may remain client-specific)

## Implementation Steps

### **Phase 1: Foundation (Week 1, Day 1)**

1. **Create Transformation Layer**
```bash
mkdir -p client/src/lib/api/transformers
# Implement base utilities and task transformer
```

2. **Update Build Configuration**
```bash
# Add shared types to TypeScript paths
# Update import resolution
```

3. **Set Up Testing Framework**
```bash
# Create transformer tests
# Set up integration test helpers
```

### **Phase 2: Tasks Migration (Week 1, Days 1-3)**

**Priority Order**:
1. `client/src/lib/api/tasks.ts` (API layer)
2. `client/src/routes/tasks/index.tsx` (main route)  
3. `client/src/features/tasks/hooks/use-tasks.ts` (core hooks)
4. `client/src/features/tasks/contexts/task-context.tsx` (context)
5. Components: TaskHeader → TaskViewer → EventList → others

**Key Changes**:
```typescript
// Before
import type { TaskMetadata } from '@/types/task'
const transformedTasks = tasks?.map(metadata => ({
  id: metadata.task_id,
  createdAt: metadata.created_at,
  // ... manual transformation
}))

// After  
import type { TaskMetadata } from '@shared/types/task'
// Automatic transformation in API layer
const tasks = await apiClient.getTasks() // Already transformed
```

### **Phase 3: Events Migration (Week 1, Days 4-5)**

**Critical Focus**: Maintain real-time functionality
- Update SSE connection handling
- Test event streaming thoroughly
- Validate event merging logic

### **Phase 4: Remaining Features (Week 2-3)**

Lower complexity migrations following the same pattern:
- Update imports
- Replace property access
- Test feature functionality

## Validation Strategy

### **Automated Testing**
```bash
# After each feature migration:
pnpm test                 # Unit tests
pnpm test:coverage       # Coverage validation
pnpm type:check          # TypeScript compilation
pnpm biome:check         # Code quality
```

### **Manual Testing Priority**

**Critical Path Testing** (Week 1):
- [ ] Task creation and viewing
- [ ] Real-time event updates  
- [ ] Task status changes
- [ ] Cost and metadata display

**Feature Testing** (Week 2):
- [ ] Environment CRUD operations
- [ ] Settings persistence
- [ ] Form validation

**Integration Testing** (Week 3):
- [ ] Cross-feature workflows
- [ ] End-to-end task execution
- [ ] Real-time updates across features

## Success Metrics

### **Technical Metrics**
- ✅ Zero TypeScript compilation errors
- ✅ 100% test suite pass rate
- ✅ No performance regression (< 5% impact)
- ✅ Code coverage maintained (≥ current levels)

### **Functional Metrics**  
- ✅ All existing features work identically
- ✅ Real-time updates function properly
- ✅ Data consistency across features
- ✅ User workflows uninterrupted

### **Code Quality Metrics**
- ✅ Consistent camelCase naming throughout
- ✅ Single source of truth for transformations  
- ✅ Eliminated manual transformation code
- ✅ Type safety improvements

## Rollback Strategy

### **Safety Measures**
1. **Git Branch Strategy**: Feature branch per migration phase
2. **Backup Tags**: Before each major change
3. **Quick Revert**: Commented fallback imports ready
4. **Validation Gates**: No progression without passing tests

### **Emergency Rollback Procedure**
```bash
# If critical issues discovered:
git checkout previous-working-tag
# Deploy previous version
# Investigate issues in parallel branch
```

## Post-Migration Benefits

### **Developer Experience**
- **Consistent Types**: Single source of truth across client/server
- **Type Safety**: Better TypeScript integration and error catching
- **Maintainability**: Centralized transformation logic
- **Performance**: Optimized transformation at API boundary

### **Code Quality**
- **Reduced Complexity**: Elimination of scattered transformation code
- **Better Testing**: Isolated, testable transformation layer
- **Standardization**: Consistent naming conventions
- **Documentation**: Clear type system guidelines

## Timeline Summary

| Week | Focus | Files | Risk | Validation |
|------|-------|-------|------|------------|
| 1 | Core (Tasks + Events) | 47 files | High | Extensive |
| 2 | Supporting Features | 18 files | Medium | Feature-specific |
| 3 | Specialized Features | 4 files | Low | Integration |
| 4 | Cleanup & Optimization | - | Low | Documentation |

## Conclusion

This migration plan provides a systematic, risk-managed approach to transitioning the client codebase from snake_case API types to camelCase shared types. The phased approach prioritizes critical functionality while maintaining system stability throughout the migration process.

**Key Success Factors**:
1. **Comprehensive Analysis**: 40 files analyzed, all dependencies mapped
2. **Risk Management**: High-risk areas identified with specific mitigation strategies  
3. **Systematic Approach**: Clear phases with validation at each step
4. **Technical Foundation**: Transformation layer design ensures consistency
5. **Validation Strategy**: Automated and manual testing throughout

The migration will result in a more maintainable, type-safe, and consistent codebase while preserving all existing functionality.