# Project Setup Guide

This guide provides detailed instructions for setting up the Moatless TypeScript/Fastify project structure.

## Prerequisites

- Node.js >=22.16.x
- pnpm 10.7.1
- Git

## Initial Project Structure

### 1. Create Base Directory Structure

```bash
mkdir -p moatless-app/{bin,server/{routes,services,plugins,utils},client,shared/{types,constants,utils}}
cd moatless-app
```

### 2. Initialize Package Configuration

#### Root package.json
```json
{
  "name": "moatless-app",
  "version": "1.0.0",
  "description": "Moatless - AI-powered code analysis and task management",
  "type": "module",
  "engines": {
    "node": ">=22.16.x"
  },
  "packageManager": "pnpm@10.7.1",
  "pnpm": {
    "onlyBuiltDependencies": [
      "@tailwindcss/oxide",
      "sharp"
    ]
  },
  "bin": {
    "moatless": "./bin/cli.js"
  },
  "scripts": {
    "dev": "concurrently \"pnpm:dev:*\"",
    "dev:server": "tsx watch server/index.ts",
    "dev:client": "cd client && pnpm dev",
    "build": "pnpm build:shared && pnpm build:server && pnpm build:client",
    "build:shared": "tsc -p shared/tsconfig.json",
    "build:server": "tsc -p server/tsconfig.json",
    "build:client": "cd client && pnpm build",
    "start": "node dist/server/index.js",
    "test": "vitest run",
    "test:ui": "vitest --ui --coverage",
    "test:coverage": "vitest --coverage run",
    "type:check": "tsc --noEmit",
    "biome:check": "biome check",
    "biome:fix": "biome check --write",
    "check:turbo": "pnpm turbo biome:check type:check test",
    "prepare": "husky"
  },
  "dependencies": {
    "@fastify/cors": "^10.0.0",
    "@fastify/helmet": "^12.0.0",
    "@fastify/static": "^8.0.0",
    "@fastify/websocket": "^11.0.0",
    "fastify": "^5.0.0",
    "fastify-plugin": "^5.0.0",
    "zod": "^3.25.61"
  },
  "devDependencies": {
    "@biomejs/biome": "2.0.0-beta.6",
    "@types/node": "^24.0.3",
    "@vitest/ui": "^3.2.4",
    "concurrently": "^9.0.0",
    "tsx": "^4.0.0",
    "typescript": "^5.7.2",
    "vitest": "^3.2.3"
  }
}
```

### 3. TypeScript Configuration

#### Root tsconfig.json
```json
{
  "compilerOptions": {
    "target": "ES2023",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2023"],
    "outDir": "./dist",
    "rootDir": "./",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "paths": {
      "@shared/*": ["./shared/*"],
      "@server/*": ["./server/*"]
    }
  },
  "include": ["server/**/*", "shared/**/*", "bin/**/*"],
  "exclude": ["node_modules", "dist", "client"]
}
```

#### server/tsconfig.json
```json
{
  "extends": "../tsconfig.json",
  "compilerOptions": {
    "outDir": "../dist/server",
    "rootDir": "./"
  },
  "include": ["./**/*"],
  "references": [
    { "path": "../shared" }
  ]
}
```

#### shared/tsconfig.json
```json
{
  "extends": "../tsconfig.json",
  "compilerOptions": {
    "outDir": "../dist/shared",
    "rootDir": "./",
    "composite": true
  },
  "include": ["./**/*"]
}
```

### 4. Biome Configuration

#### biome.json
```json
{
  "$schema": "https://biomejs.dev/schemas/1.9.4/schema.json",
  "vcs": {
    "enabled": true,
    "clientKind": "git",
    "useIgnoreFile": true
  },
  "files": {
    "ignoreUnknown": false,
    "ignore": ["dist/", "node_modules/", "client/dist/"]
  },
  "formatter": {
    "enabled": true,
    "formatWithErrors": false,
    "indentStyle": "space",
    "indentWidth": 2,
    "lineEnding": "lf",
    "lineWidth": 100,
    "attributePosition": "auto"
  },
  "organizeImports": {
    "enabled": true
  },
  "linter": {
    "enabled": true,
    "rules": {
      "recommended": true,
      "style": {
        "noNonNullAssertion": "off",
        "useImportType": "error"
      },
      "correctness": {
        "noUnusedImports": "error"
      },
      "complexity": {
        "noBannedTypes": "error"
      }
    }
  },
  "javascript": {
    "formatter": {
      "jsxQuoteStyle": "double",
      "quoteProperties": "asNeeded",
      "trailingCommas": "all",
      "semicolons": "asNeeded",
      "arrowParentheses": "always",
      "bracketSpacing": true,
      "bracketSameLine": false,
      "quoteStyle": "single",
      "attributePosition": "auto"
    }
  }
}
```

### 5. NPX Entry Point

#### bin/cli.js
```javascript
#!/usr/bin/env node

import { spawn } from 'child_process'
import { fileURLToPath } from 'url'
import { dirname, join } from 'path'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)
const rootDir = join(__dirname, '..')

// Parse command line arguments
const args = process.argv.slice(2)
const command = args[0] || 'start'

// Environment setup
process.env.NODE_ENV = process.env.NODE_ENV || 'production'

switch (command) {
  case 'start':
    startServer()
    break
  case 'dev':
    startDevelopment()
    break
  default:
    console.log(`Unknown command: ${command}`)
    console.log('Usage: npx moatless [start|dev]')
    process.exit(1)
}

function startServer() {
  const serverPath = join(rootDir, 'dist', 'server', 'index.js')
  const child = spawn('node', [serverPath], {
    cwd: rootDir,
    stdio: 'inherit',
    env: { ...process.env }
  })

  child.on('error', (err) => {
    console.error('Failed to start server:', err)
    process.exit(1)
  })

  child.on('exit', (code) => {
    process.exit(code || 0)
  })
}

function startDevelopment() {
  const child = spawn('pnpm', ['dev'], {
    cwd: rootDir,
    stdio: 'inherit',
    shell: true
  })

  child.on('error', (err) => {
    console.error('Failed to start development server:', err)
    process.exit(1)
  })

  child.on('exit', (code) => {
    process.exit(code || 0)
  })
}
```

### 6. Move Existing React Application

```bash
# Move existing React code to client directory
mv src/* client/src/
mv public/* client/public/
mv index.html client/
mv vite.config.ts client/
mv tsconfig.json client/tsconfig.json.bak  # Backup existing config

# Create client-specific package.json
```

#### client/package.json
```json
{
  "name": "moatless-client",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite --port 3000",
    "build": "vite build",
    "preview": "vite preview",
    "type:check": "tsc --noEmit"
  },
  "dependencies": {
    "@git-diff-view/react": "^0.0.28",
    "@radix-ui/react-alert-dialog": "^1.1.14",
    "@radix-ui/react-checkbox": "^1.3.2",
    "@radix-ui/react-collapsible": "^1.1.11",
    "@radix-ui/react-dialog": "^1.1.14",
    "@radix-ui/react-label": "^2.1.7",
    "@radix-ui/react-progress": "^1.1.7",
    "@radix-ui/react-scroll-area": "^1.2.9",
    "@radix-ui/react-select": "^2.2.5",
    "@radix-ui/react-separator": "^1.1.7",
    "@radix-ui/react-slot": "^1.2.3",
    "@radix-ui/react-switch": "^1.2.5",
    "@radix-ui/react-tabs": "^1.1.12",
    "@radix-ui/react-tooltip": "^1.2.7",
    "@tailwindcss/vite": "^4.0.6",
    "@tanstack/react-query": "^5.80.7",
    "@tanstack/react-query-devtools": "^5.80.7",
    "@tanstack/react-router": "^1.121.0",
    "@tanstack/react-router-devtools": "^1.121.0",
    "@tanstack/router-plugin": "^1.121.0",
    "@theme-toggles/react": "^4.1.0",
    "class-variance-authority": "^0.7.1",
    "clsx": "^2.1.1",
    "lucide-react": "^0.515.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "tailwind-merge": "^3.3.1",
    "tailwindcss": "^4.0.6",
    "zod": "^3.25.61"
  },
  "devDependencies": {
    "@types/react": "^19.1.7",
    "@types/react-dom": "^19.1.6",
    "@vitejs/plugin-react": "^4.5.2",
    "typescript": "^5.7.2",
    "vite": "^6.1.0",
    "vite-tsconfig-paths": "^5.1.4"
  }
}
```

#### client/tsconfig.json
```json
{
  "compilerOptions": {
    "target": "ES2023",
    "useDefineForClassFields": true,
    "lib": ["ES2023", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true,
    "paths": {
      "@/*": ["./src/*"],
      "@shared/*": ["../shared/*"]
    }
  },
  "include": ["src", "../shared"]
}
```

#### client/vite.config.ts
```typescript
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tsconfigPaths from 'vite-tsconfig-paths'
import tailwindcss from '@tailwindcss/vite'
import { TanStackRouterVite } from '@tanstack/router-plugin/vite'

export default defineConfig({
  plugins: [
    react(),
    tsconfigPaths(),
    tailwindcss(),
    TanStackRouterVite()
  ],
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8000',
        changeOrigin: true
      },
      '/ws': {
        target: 'ws://localhost:8000',
        ws: true
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: true
  }
})
```

### 7. Environment Configuration

#### .env.example
```bash
# Server Configuration
NODE_ENV=development
PORT=8000
HOST=0.0.0.0

# Storage Configuration
STORAGE_PATH=./data
CACHE_ENABLED=false

# CORS Configuration
CORS_ORIGIN=http://localhost:3000

# Logging
LOG_LEVEL=info
```

### 8. Git Configuration

#### .gitignore
```gitignore
# Dependencies
node_modules/
.pnpm-store/

# Build outputs
dist/
*.tsbuildinfo

# Environment files
.env
.env.local
.env.*.local

# Logs
logs/
*.log

# OS files
.DS_Store
Thumbs.db

# IDE
.vscode/
.idea/
*.swp
*.swo

# Test coverage
coverage/
.nyc_output/

# Data files
data/
*.db
*.sqlite

# Temporary files
tmp/
temp/
```

## Installation Steps

### 1. Install Dependencies

```bash
# Install root dependencies
pnpm install

# Install client dependencies
cd client && pnpm install && cd ..
```

### 2. Verify Setup

```bash
# Check TypeScript compilation
pnpm type:check

# Run Biome checks
pnpm biome:check

# Run tests
pnpm test
```

### 3. Update Import Paths

After moving the React code, update all import paths in the client code:

```typescript
// Before
import { TaskList } from '@/features/tasks'

// After (no change needed - @/ still maps to client/src/)
import { TaskList } from '@/features/tasks'

// New shared imports
import type { Task } from '@shared/types/task'
```

## Development Workflow

### 1. Start Development Servers

```bash
# Start both server and client in development mode
pnpm dev
```

This will run:
- Fastify server on http://localhost:8000
- React client on http://localhost:3000

### 2. Build for Production

```bash
# Build all components
pnpm build

# Start production server
pnpm start
```

### 3. Testing

```bash
# Run all tests
pnpm test

# Run with UI
pnpm test:ui

# Run with coverage
pnpm test:coverage
```

## Troubleshooting

### Common Issues

1. **Port conflicts**: Change ports in `.env` and `client/vite.config.ts`
2. **TypeScript errors**: Run `pnpm type:check` to identify issues
3. **Import path errors**: Ensure tsconfig paths are correctly configured
4. **Build failures**: Clear dist folders and rebuild

### Verification Checklist

- [ ] All dependencies installed without errors
- [ ] TypeScript compilation passes
- [ ] Biome checks pass
- [ ] Development servers start correctly
- [ ] Client can proxy to server API
- [ ] Build process completes successfully

## Next Steps

1. Review [Type System Migration](./02-type-system.md) for shared type setup
2. Continue with [Server Implementation](./03-server-implementation.md)
3. Follow the migration checklist in the main strategy document