import type { FullConfig } from '@playwright/test'

async function simpleGlobalSetup(config: FullConfig) {
  console.log('Setting up E2E test environment (simple mode)...')
  
  // Set basic environment variables for testing
  process.env.NODE_ENV = 'test'
  process.env.VITE_USE_GLOBAL_EVENT_STREAM = 'true'
  process.env.VITE_DEBUG_EVENT_STREAM = 'true'
  
  console.log('✅ Simple E2E setup complete')
}

export default simpleGlobalSetup