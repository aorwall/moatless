/**
 * No-op metric implementations for when METRICS_ENABLED is not set.
 * These provide the same interface as prom-client metrics but do nothing.
 */

interface NoOpLabelValues {
  inc: (value?: number) => void
  dec: (value?: number) => void
  set: (value: number) => void
  observe: (value: number) => void
  startTimer: () => () => number
}

const noOpLabelValues: NoOpLabelValues = {
  inc: () => {
    /* noop */
  },
  dec: () => {
    /* noop */
  },
  set: () => {
    /* noop */
  },
  observe: () => {
    /* noop */
  },
  startTimer: () => () => 0,
}

export interface NoOpCounter {
  inc: (value?: number) => void
  labels: (labels: Record<string, string>) => { inc: (value?: number) => void }
}

export interface NoOpHistogram {
  observe: (value: number) => void
  startTimer: () => () => number
  labels: (labels: Record<string, string>) => {
    observe: (value: number) => void
    startTimer: () => () => number
  }
}

export interface NoOpGauge {
  inc: (value?: number) => void
  dec: (value?: number) => void
  set: (value: number) => void
  labels: (labels: Record<string, string>) => {
    inc: (value?: number) => void
    dec: (value?: number) => void
    set: (value: number) => void
  }
}

export function createNoOpCounter(): NoOpCounter {
  return {
    inc: () => {
      /* noop */
    },
    labels: () => ({
      inc: () => {
        /* noop */
      },
    }),
  }
}

export function createNoOpHistogram(): NoOpHistogram {
  return {
    observe: () => {
      /* noop */
    },
    startTimer: () => () => 0,
    labels: () => ({
      observe: () => {
        /* noop */
      },
      startTimer: () => () => 0,
    }),
  }
}

export function createNoOpGauge(): NoOpGauge {
  return {
    inc: () => {
      /* noop */
    },
    dec: () => {
      /* noop */
    },
    set: () => {
      /* noop */
    },
    labels: () => noOpLabelValues,
  }
}
