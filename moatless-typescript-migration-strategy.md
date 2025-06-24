# Moatless TypeScript/Fastify Backend Migration Strategy

## Executive Summary

This document outlines a comprehensive strategy for migrating the Moatless Swarm backend from Python to TypeScript with Fastify as the web framework. The strategy focuses on creating a modern monolithic application that can be easily distributed via NPX, with a clear separation between server, client, and shared code.

## Current Architecture Analysis

### Backend (Python)
The current backend is built with:
- **FastAPI** for REST endpoints and WebSocket support
- **Pydantic v2** for data validation and serialization
- **File-based storage** with optional Redis caching
- **Event streaming** for real-time updates
- **Task-based orchestration** for agent lifecycle management
- **Workspace management** supporting local and Docker environments

### Frontend (TypeScript/React)
The existing frontend in `src/` uses:
- **React 19** with TypeScript
- **TanStack Query** for data fetching and caching
- **TanStack Router** for navigation
- **Tailwind CSS v4** for styling
- **Vite** as the build tool
- **Biome** for code quality
- **pnpm** as package manager

## Target Architecture

### Project Structure
```
moatless-app/
├── package.json          # Main package.json with scripts
├── tsconfig.json         # Root TypeScript config
├── biome.json           # Code quality configuration
├── bin/
│   └── cli.js           # NPX entry point
├── server/
│   ├── index.ts         # Fastify server entry point
│   ├── app.ts           # Fastify app configuration
│   ├── routes/          # API routes
│   ├── services/        # Business logic
│   ├── plugins/         # Fastify plugins
│   └── utils/           # Server utilities
├── client/              # Existing React app from src/
│   ├── src/             # Current src/ contents
│   ├── public/          # Static assets
│   ├── index.html       # HTML entry point
│   ├── vite.config.ts   # Vite configuration
│   └── package.json     # Client dependencies
└── shared/
    ├── types/           # Shared TypeScript types
    ├── constants/       # Shared constants
    └── utils/           # Shared utilities
```

## Migration Strategy Overview

### Key Principles
1. **Reuse existing React code** - Move current `src/` to `client/src/`
2. **Maintain compatibility** - Keep existing API contracts
3. **Incremental migration** - Phase-based approach
4. **Type safety** - Share types between server and client
5. **Developer experience** - Use familiar tools (pnpm, Biome, Vitest)

### High-Level Phases

1. **Foundation & Setup** - Project structure and tooling
2. **Type System** - Shared types and validation
3. **Server Components** - Core Fastify implementation
4. **Service Layer** - Business logic migration
5. **API Routes** - REST and WebSocket endpoints
6. **Integration** - Connect existing React frontend
7. **Testing & Quality** - Comprehensive test coverage
8. **Documentation & Deployment** - NPX distribution

## Related Documents

- [Project Setup Guide](./docs/migration/01-project-setup.md) - Detailed setup instructions
- [Type System Migration](./docs/migration/02-type-system.md) - Type definitions and mappings
- [Server Implementation](./docs/migration/03-server-implementation.md) - Fastify server details
- [Service Architecture](./docs/migration/04-service-architecture.md) - Service layer design
- [Frontend Integration](./docs/migration/05-frontend-integration.md) - React app migration
- [Testing Strategy](./docs/migration/06-testing-strategy.md) - Test approach and tools
- [Deployment Guide](./docs/migration/07-deployment.md) - NPX distribution setup

## Quick Start

```bash
# Clone and setup
git clone <repository>
cd moatless-app
pnpm install

# Development
pnpm run dev

# Testing
pnpm test
pnpm type:check
pnpm biome:check

# Production
pnpm run build
pnpm start
```

## Development Workflow

1. **Use existing tools** - pnpm, Biome, Vitest, TypeScript
2. **Mandatory checks** - Run `pnpm type:check` and `pnpm biome:check`
3. **Incremental commits** - Commit after each completed phase
4. **Quality gates** - All checks must pass before proceeding

## Technical Decisions

- **Package Manager**: pnpm (v10.7.1) - Already in use
- **Node Version**: >=22.16.x - Matching current requirement
- **Code Quality**: Biome - Already configured
- **Testing**: Vitest - Already in use
- **Build Tool**: Vite for client, tsc for server
- **Distribution**: NPX for easy installation

## Migration Checklist

### Phase 1: Foundation & Setup
- [x] Project structure defined
- [x] Tool selection completed
- [x] Configuration templates ready
- [ ] Create directory structure
- [ ] Set up configuration files
- [ ] Install dependencies
- [ ] Verify setup

### Phase 2: Type System
- [ ] Move existing types to shared/
- [ ] Add server-specific types
- [ ] Set up Zod validation

### Phase 3: Server Implementation
- [ ] Fastify server setup
- [ ] Storage service
- [ ] Event system
- [ ] Environment management

### Phase 4: Service Layer
- [ ] Task management
- [ ] Workspace management
- [ ] Agent integration

### Phase 5: API Routes
- [ ] REST endpoints
- [ ] WebSocket support
- [ ] Authentication

### Phase 6: Frontend Integration
- [ ] Move src/ to client/src/
- [ ] Update import paths
- [ ] Connect to new backend

### Phase 7: Testing
- [ ] Unit tests
- [ ] Integration tests
- [ ] E2E tests

### Phase 8: Deployment
- [ ] NPX setup
- [ ] Documentation
- [ ] Release process

## Next Steps

1. Review the [Project Setup Guide](./docs/migration/01-project-setup.md)
2. Begin with Phase 1 implementation
3. Follow the incremental migration approach
4. Maintain backward compatibility throughout

## Success Criteria

- All existing functionality preserved
- Type safety between server and client
- Comprehensive test coverage
- Easy NPX installation
- Performance parity or better
- Developer experience improved