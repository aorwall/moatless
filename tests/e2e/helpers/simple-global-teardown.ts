import type { FullConfig } from '@playwright/test'

async function simpleGlobalTeardown(config: FullConfig) {
  console.log('Cleaning up E2E test setup (simple mode)...')
  
  // Clean up setup variables
  delete process.env.VITE_USE_GLOBAL_EVENT_STREAM
  delete process.env.VITE_DEBUG_EVENT_STREAM
  
  console.log('✅ Simple E2E teardown complete')
}

export default simpleGlobalTeardown