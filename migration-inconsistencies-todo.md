# Migration Inconsistencies TODO

## Overview
This document tracks inconsistencies between the planned feature migration strategy (docs/feature-migration-strategy.md) and the actual implementation. These issues need to be addressed to complete the migration properly.

**IMPORTANT**: The codebase should ALWAYS use camelCase for ALL APIs, types, properties, and data structures. No snake_case should be used anywhere.

## Critical Issues Found

### 1. Inconsistent Property Naming Convention ❌
**Problem**: The shared types correctly use camelCase properties, but some parts of the codebase still use snake_case properties, violating our standard.

**Standard**: ALL properties, APIs, and data structures MUST use camelCase. No exceptions.

**Impact**: Runtime errors due to type mismatches between TypeScript definitions and actual data.

**TODO**:
- [ ] Update server to use camelCase properties EVERYWHERE
- [ ] Ensure ALL API responses use camelCase exclusively
- [ ] Remove ALL snake_case usage from the entire codebase
- [ ] Add linting rules to prevent snake_case usage

### 2. Property Name Mismatches 🔴
**Problem**: Multiple components violate our camelCase standard by accessing snake_case properties.

**Affected Files** (ALL must be fixed to use camelCase):
- [ ] `client/src/features/tasks/hooks/use-task-streaming.ts` - MUST use `event.eventId` NOT `event.event_id`
- [ ] `client/src/features/claude-code/hooks/use-claude-code-parser.ts` - MUST create events with `eventId`, `isError` NOT `event_id`, `is_error`
- [ ] `client/src/features/tasks/components/events/result-event.tsx` - MUST access `payload.isError`, `payload.numTurns`, `payload.totalCostUsd` NOT snake_case
- [ ] `client/src/features/tasks/components/events/workspace-event.tsx` - MUST use `environmentId`, `workingDirectory` NOT snake_case

**TODO**:
- [ ] Update ALL property access to use camelCase exclusively
- [ ] Ensure NO snake_case remains anywhere in the codebase

### 3. Incomplete Type Migration ⚠️
**Problem**: API types are now imported from `@shared/types` but runtime data doesn't match.

**Status**:
- ✅ Type imports migrated from `@/types` to `@shared/types`
- ❌ Runtime property access not updated to match camelCase types
- ✅ Old type directory removed (except git-diff.ts)

### 4. Server Response Format Must Be CamelCase 🟡
**Problem**: Need to verify and ensure server is sending camelCase responses ONLY.

**Standard**: Server MUST send camelCase properties in ALL responses. No exceptions.

**TODO**:
- [ ] Audit ALL server API endpoints for snake_case usage
- [ ] Update server to send camelCase responses EXCLUSIVELY
- [ ] Document and enforce: ALL APIs MUST use camelCase
- [ ] Add server-side validation to ensure camelCase output

## Implementation Plan

### Phase 1: Enforce CamelCase EVERYWHERE (Priority 1)
1. Update server to use camelCase properties in ALL responses - NO EXCEPTIONS
2. Remove ALL snake_case usage from server code
3. Ensure 100% consistency with shared types definitions
4. Test ALL API endpoints to verify camelCase-only output
5. Add automated tests to prevent snake_case regression

### Phase 2: Fix ALL Component Property Access (Priority 2)
1. Update ALL components to use camelCase properties ONLY
2. Remove EVERY instance of snake_case property access
3. Ensure 100% consistent camelCase naming throughout
4. Add ESLint rules to ban snake_case property access

### Phase 3: Validation & Testing (Priority 3)
1. Run `pnpm type:check` to ensure no TypeScript errors
2. Test all affected components:
   - Task list and details
   - Event streaming
   - Workspace status
   - Environment management
3. Verify real-time updates work correctly

## Risk Assessment

### High Risk Areas
1. **Real-time Event Streaming**: SSE connections may break if property names don't match
2. **Task Operations**: Core functionality affected by property mismatches
3. **Type Safety**: Current state provides false sense of security with type definitions not matching runtime

### Mitigation Strategy
1. Update server responses to use camelCase to match shared types
2. Test thoroughly in development before production deployment
3. Consider adding runtime property validation for critical paths

## Success Criteria
- [ ] ZERO runtime errors due to property name mismatches
- [ ] ALL TypeScript types match runtime data structure exactly
- [ ] Server and client use camelCase EXCLUSIVELY - NO snake_case anywhere
- [ ] ALL tests pass with camelCase-only naming
- [ ] Real-time features work with camelCase properties only
- [ ] Linting rules prevent any snake_case usage
- [ ] 100% consistency: camelCase is used EVERYWHERE

## Notes
- The migration to `@shared/types` was completed but runtime data still violates our camelCase standard
- This creates a dangerous situation where TypeScript compilation succeeds but runtime fails
- **CRITICAL**: The entire codebase MUST use camelCase for ALL properties, APIs, and data structures
- **NO EXCEPTIONS**: snake_case is NOT allowed anywhere - client, server, or shared code
- Priority should be given to enforcing camelCase EVERYWHERE to prevent production issues