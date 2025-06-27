import '@testing-library/jest-dom';
import { cleanup } from '@testing-library/react';
// Import React explicitly for React 19 compatibility
import * as React from 'react';
import { afterEach } from 'vitest';

// Make React available in multiple ways for React 19
declare global {
  var React: typeof import('react')
}

// Assign React to global scope in multiple ways
;(globalThis as any).React = React;
;(global as any).React = React;
if (typeof window !== 'undefined') {
  ;(window as any).React = React;
}

// Cleanup after each test
afterEach(() => {
  cleanup()
})
