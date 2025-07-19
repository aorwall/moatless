# NPM Publishing Guide

This guide explains how the Moatless Vibe application is packaged and published to npm.

## Overview

Moatless Vibe uses **esbuild** to bundle the backend into a single JavaScript file, while the frontend remains as static files built by Vite. This approach significantly reduces the package size and installation time for users.

## Build Architecture

### Frontend (Vite)
- Built with Vite into static files
- All dependencies are bundled into the output
- No runtime dependencies needed
- Output: `dist/client/` directory

### Backend (esbuild)
- Bundled with esbuild into a single file
- Most dependencies are included in the bundle
- Only native/binary dependencies remain external
- Output: `dist/server.js` (with source map)

## Package Structure

```
moatless-vibe-0.1.0.tgz
├── bin/cli.js              # CLI entry point
├── dist/
│   ├── server.js          # Bundled backend (1MB)
│   ├── server.js.map      # Source map (2MB)
│   └── client/            # Frontend static files
│       ├── index.html
│       └── assets/        # JS, CSS, and other assets
├── package.json
├── LICENSE
└── README.md
```

## Build Process

### 1. Build All Packages
```bash
bun build
```
This runs Turborepo to build:
- `@moatless/types` - Shared types package
- `@moatless/frontend` - React frontend
- `@moatless/backend` - Node.js backend

### 2. Bundle Backend
The `scripts/build-backend.js` script uses esbuild to:
- Bundle all backend code and dependencies
- Keep native modules external (`dockerode`, `@anthropic-ai/claude-code`)
- Add necessary polyfills for ES modules
- Generate source maps for debugging

### 3. Prepare Package
The `scripts/prepare-package.js` script:
- Cleans the `dist` directory
- Runs the backend bundler
- Copies frontend build to `dist/client`

## Configuration Files

### Backend Bundler (`scripts/build-backend.js`)
```javascript
await esbuild.build({
  entryPoints: ['apps/backend/index.ts'],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: 'dist/server.js',
  external: [
    'dockerode',                    // Native Docker bindings
    '@anthropic-ai/claude-code',    // Keep external for licensing
  ],
  sourcemap: true,
  minify: process.env.NODE_ENV === 'production',
})
```

### Package Configuration
The root `package.json` only includes runtime dependencies that can't be bundled:
```json
{
  "dependencies": {
    "@anthropic-ai/claude-code": "^1.0.33",
    "dockerode": "^4.0.7"
  }
}
```

## Publishing Workflow

### 1. Build the Package
```bash
bun build:package
```

### 2. Test Locally
```bash
# Create tarball
npm pack

# Test installation
cd /tmp/test-dir
npm install /path/to/moatless-vibe-0.1.0.tgz
npx moatless-vibe
```

### 3. Publish to NPM
```bash
# Login to npm
npm login

# Publish
npm publish
```

## Runtime Behavior

The CLI (`bin/cli.js`) detects whether it's running from:
- **Installed Package**: Uses `dist/server.js` and `dist/client`
- **Development**: Uses source files in `apps/`

```javascript
const isInstalledPackage = !existsSync(join(rootDir, 'apps'))
const serverPath = isInstalledPackage
  ? join(rootDir, 'dist', 'server.js')
  : join(rootDir, 'apps', 'backend', 'dist', 'index.js')
```

## Troubleshooting

### Common Issues

1. **Module not found errors**: Ensure external dependencies are listed in package.json
2. **Build failures**: Check that all TypeScript files compile without errors
3. **Large bundle size**: Review external dependencies list in esbuild config

### Debugging

- Source maps are included for debugging production issues
- Use `NODE_ENV=development` for verbose logging
- Check `dist/server.js` directly to verify bundling

## Docker Usage

### Using the Packaged Application in Docker

The main Dockerfile now uses the packaged application instead of building from source, which is faster and more reliable:

#### Build Process & Caching Strategy

The Docker build uses a multi-stage approach for optimal caching:

**Stage 1: Base System** (`base-system`)
- Ubuntu packages, Node.js, Python, Rust, Go, Java
- Development tools and environment setup
- This layer is cached and rarely changes

**Stage 2: Application** (`final`)
- Copies the packaged application files
- Installs moatless-vibe globally
- Only rebuilds when the application changes

The `docker/build-worker.sh` script:
1. Runs `bun build:package` to create the bundled application
2. Builds the Docker image using cached base layers
3. Only the application layer rebuilds on code changes

#### Building the Docker Image

```bash
# Fast build (uses cache for base system)
./docker/build-worker.sh --tag latest

# Build base system separately (for CI/caching)
./docker/build-base.sh --tag latest

# Force full rebuild without cache
./docker/build-worker.sh --tag latest --no-cache

# Build for specific platform
./docker/build-worker.sh --tag latest --platform linux/amd64,linux/arm64
```

#### Caching Benefits

- **First build**: ~10-15 minutes (installs all system dependencies)
- **Subsequent builds**: ~1-2 minutes (only rebuilds application layer)
- **Base system changes**: Can rebuild base separately with `build-base.sh`
- **Application changes**: Fast incremental builds

#### Simple Dockerfile for NPM Package
For a minimal container using the published npm package:
```dockerfile
FROM node:22-slim
RUN npm install -g moatless-vibe
EXPOSE 8001
CMD ["moatless-vibe", "start"]
```

### Benefits of Using NPM Package in Docker
- **Faster builds**: No compilation required, just npm install
- **Smaller layers**: Only runtime dependencies needed
- **Consistent versioning**: Pin to specific npm versions
- **Easier updates**: Just update the npm package version

## Security Best Practices

### 1. Essential Security Setup

```bash
# Enable 2FA for your npm account
npm profile enable-2fa auth-and-writes

# Review what will be published before releasing
npm pack --dry-run
npm pack && tar -tf moatless-vibe-*.tgz

# Check for vulnerabilities
npm audit --production
```

### 2. Secure Package Configuration

#### Avoid Install Scripts
```json
{
  "scripts": {
    "test": "vitest",
    "build": "bun build:package"
    // Never use: postinstall, preinstall, install
  }
}
```

#### Use Exact Versions
```json
{
  "dependencies": {
    "@anthropic-ai/claude-code": "1.0.33",  // Not "^1.0.33"
    "dockerode": "4.0.7"                     // Not "~4.0.7"
  }
}
```

### 3. Protect Sensitive Files

Create `.npmignore`:
```
# Keep these out of the published package
.env*
*.key
*.pem
test/
docs/
scripts/
.git/
.github/
```

### 4. Pre-publish Checklist

```bash
# Quick security check before publishing
npm audit --production          # No vulnerabilities
npm pack --dry-run              # Review files
grep -E "postinstall" package.json  # No install scripts
```

## Future Improvements

- Consider using `pkg` to create standalone executables
- Implement tree-shaking for smaller frontend bundles
- Add compression for distributed files
- Create GitHub Actions workflow for automated publishing
- Publish official Docker images to Docker Hub
- Implement package signing with sigstore
- Add Software Bill of Materials (SBOM)
- Set up automated security scanning in CI/CD