// Framework configuration constants
export const FRAMEWORK_OPTIONS = [
  { value: 'vite', label: 'Vite' },
  { value: 'nextjs', label: 'Next.js' },
  { value: 'cra', label: 'Create React App' },
  { value: 'webpack', label: 'Webpack' },
  { value: 'unknown', label: 'Unknown/Other' },
] as const

// Script injection method constants with detailed descriptions
export const INJECTION_METHOD_OPTIONS = [
  {
    value: 'html-injection',
    label: 'HTML Injection',
    description: 'Injects script tags directly into HTML files (index.html)',
    details: 'Best for static sites and SPAs. Modifies the HTML file to include preview scripts.',
    requirements: 'Requires access to the main HTML file (usually index.html)',
    pros: ['Simple to implement', 'Works with most static sites', 'No server changes needed'],
    cons: ['Requires file write access', 'May conflict with build tools'],
  },
  {
    value: 'middleware',
    label: 'Middleware',
    description: 'Injects scripts via server middleware during request handling',
    details: 'Intercepts HTTP responses and modifies HTML content on-the-fly.',
    requirements: 'Server must support middleware or proxy configuration',
    pros: ['No file modifications', 'Works with any HTML response', 'Dynamic injection'],
    cons: ['Requires server support', 'May affect performance'],
  },
  {
    value: 'build-plugin',
    label: 'Build Plugin',
    description: 'Integrates with build tools (Webpack, Vite, etc.) to inject scripts',
    details: 'Uses build tool plugins to automatically include preview scripts during compilation.',
    requirements: 'Compatible build tool (Webpack, Vite, Rollup, etc.)',
    pros: ['Integrated with build process', 'Optimal performance', 'Automatic updates'],
    cons: ['Requires build tool support', 'Complex setup'],
  },
  {
    value: 'proxy',
    label: 'Proxy',
    description: 'Uses a proxy server to inject scripts into responses',
    details:
      'Routes traffic through a proxy that modifies HTML responses to include preview scripts.',
    requirements: 'Proxy server configuration',
    pros: ['No application changes', 'Works with any server', 'Easy to enable/disable'],
    cons: ['Additional network layer', 'Potential latency', 'Complex routing'],
  },
  {
    value: 'none',
    label: 'None',
    description: 'No automatic script injection - manual integration required',
    details: 'Scripts must be manually added to the application code.',
    requirements: 'Manual code changes',
    pros: ['Full control', 'No tooling dependencies', 'Predictable behavior'],
    cons: ['Manual setup required', 'Easy to forget', 'Maintenance overhead'],
  },
] as const

// Note: The system always uses random available ports for actual server binding.
// The serverPort in preview config is only used if the serverStartCommand
// contains a port placeholder (like {PORT}) that gets replaced.

// Default server commands for different frameworks (with PORT placeholder)
export const FRAMEWORK_DEFAULT_COMMANDS = {
  vite: 'npm run dev -- --port {PORT}',
  nextjs: 'npm run dev -- --port {PORT}',
  cra: 'npm start', // CRA doesn't support port via CLI args easily
  webpack: 'npm run serve -- --port {PORT}',
  unknown: 'npm run dev -- --port {PORT}',
} as const

// Package manager detection order (from most specific to least)
export const PACKAGE_MANAGER_FILES = [
  { file: 'bun.lockb', manager: 'bun' },
  { file: 'pnpm-lock.yaml', manager: 'pnpm' },
  { file: 'yarn.lock', manager: 'yarn' },
  { file: 'package-lock.json', manager: 'npm' },
] as const

// Common script injection configurations
export const INJECTION_CONFIG_TEMPLATES = {
  'html-injection': {
    indexPath: 'index.html',
    insertPosition: 'head',
  },
  middleware: {
    pattern: '**/*.html',
    insertPosition: 'head',
  },
  'build-plugin': {
    entry: 'main',
    insertPosition: 'head',
  },
  proxy: {
    targetPattern: '**/*.html',
    insertPosition: 'head',
  },
  none: {},
} as const
