# Design Token Architecture Documentation

This directory contains all documentation related to the Three-Tier Design Token Architecture refactoring proposal for the Moatless Swarm UI application.

## 📚 Documentation Index

### 1. [Architecture Proposal](./design-token-architecture-proposal.md)
**The main document** - Complete analysis and refactoring strategy including:
- Executive Summary
- Current State Analysis
- Three-Tier Token Architecture Design
- 5-Week Migration Strategy
- Testing Guidelines
- Team Onboarding

### 2. [Migration Examples](./token-migration-examples.md)
**Implementation guide** - Concrete before/after code examples:
- Component migration examples (Button, Card, Input, etc.)
- Utility class migrations
- TypeScript integration
- Creating new components with tokens

### 3. [Deliverables Summary](./design-token-refactoring-deliverables.md)
**Quick reference** - Overview of all deliverables and next steps:
- Key findings summary
- Implementation checklist
- Priority matrix
- Quick start guide

## 🚀 Getting Started

1. **Start with:** [Architecture Proposal](./design-token-architecture-proposal.md) for the complete overview
2. **Then review:** [Migration Examples](./token-migration-examples.md) for implementation details
3. **Reference:** [Deliverables Summary](./design-token-refactoring-deliverables.md) for quick navigation

## 📁 Related Files

The actual token CSS files are located in:
```
apps/frontend/src/styles/tokens/
├── 01-core.css      # Primitive tokens
├── 02-semantic.css  # Semantic/alias tokens
└── 03-components.css # Component tokens
```

## 🎯 Implementation Status

- [x] Investigation and analysis complete
- [x] Token architecture designed
- [x] Migration strategy documented
- [x] Code examples provided
- [ ] Implementation started
- [ ] Testing setup
- [ ] Team training
- [ ] Gradual rollout

---

For questions about this refactoring proposal, refer to the detailed documentation in this directory.