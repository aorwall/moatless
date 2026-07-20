import { describe, expect, it } from 'vitest'
import {
  AT_BOTTOM_THRESHOLD,
  computeAnchorRestore,
  distanceFromBottom,
  FETCH_THRESHOLD,
  isAtBottom,
  shouldFetchPreviousPage,
  shouldRearmFetch,
  shouldShowScrollDownButton,
  shouldStickToBottom,
} from '../message-history-scroll'

describe('distanceFromBottom', () => {
  it('is zero when scrolled fully to the bottom', () => {
    expect(distanceFromBottom({ scrollTop: 900, scrollHeight: 1000, clientHeight: 100 })).toBe(0)
  })

  it('grows as the user scrolls up', () => {
    expect(distanceFromBottom({ scrollTop: 400, scrollHeight: 1000, clientHeight: 100 })).toBe(500)
  })
})

describe('isAtBottom', () => {
  it('is true within the threshold', () => {
    // distance = 40 <= 50
    expect(isAtBottom({ scrollTop: 860, scrollHeight: 1000, clientHeight: 100 })).toBe(true)
  })

  it('is true exactly at the threshold boundary', () => {
    expect(
      isAtBottom({
        scrollTop: 1000 - 100 - AT_BOTTOM_THRESHOLD,
        scrollHeight: 1000,
        clientHeight: 100,
      }),
    ).toBe(true)
  })

  it('is false past the threshold', () => {
    // distance = 51 > 50
    expect(isAtBottom({ scrollTop: 849, scrollHeight: 1000, clientHeight: 100 })).toBe(false)
  })
})

describe('shouldShowScrollDownButton', () => {
  it('shows only when not at the bottom', () => {
    expect(shouldShowScrollDownButton(false)).toBe(true)
    expect(shouldShowScrollDownButton(true)).toBe(false)
  })
})

describe('shouldStickToBottom', () => {
  it('sticks when the previous position was at the bottom', () => {
    expect(shouldStickToBottom({ scrollTop: 870, scrollHeight: 1000, clientHeight: 100 })).toBe(
      true,
    )
  })

  it('does not stick when the user had scrolled up', () => {
    expect(shouldStickToBottom({ scrollTop: 400, scrollHeight: 1000, clientHeight: 100 })).toBe(
      false,
    )
  })
})

describe('shouldRearmFetch', () => {
  it('re-arms once scrolled past the fetch threshold', () => {
    expect(shouldRearmFetch(FETCH_THRESHOLD + 1)).toBe(true)
  })

  it('stays disarmed at or below the threshold', () => {
    expect(shouldRearmFetch(FETCH_THRESHOLD)).toBe(false)
    expect(shouldRearmFetch(0)).toBe(false)
  })
})

describe('shouldFetchPreviousPage', () => {
  const base = {
    scrollTop: 100,
    armed: true,
    hasPreviousPage: true,
    isFetchingPreviousPage: false,
    canFetch: true,
    anchorPending: false,
  }

  it('fetches when near the top, armed, and idle', () => {
    expect(shouldFetchPreviousPage(base)).toBe(true)
  })

  it('does not fetch when scrolled below the threshold', () => {
    expect(shouldFetchPreviousPage({ ...base, scrollTop: FETCH_THRESHOLD + 1 })).toBe(false)
  })

  it('does not fetch when disarmed', () => {
    expect(shouldFetchPreviousPage({ ...base, armed: false })).toBe(false)
  })

  it('does not fetch when there is no previous page', () => {
    expect(shouldFetchPreviousPage({ ...base, hasPreviousPage: false })).toBe(false)
  })

  it('does not fetch while already fetching', () => {
    expect(shouldFetchPreviousPage({ ...base, isFetchingPreviousPage: true })).toBe(false)
  })

  it('does not fetch without a fetch callback', () => {
    expect(shouldFetchPreviousPage({ ...base, canFetch: false })).toBe(false)
  })

  it('does not fetch when an anchor restore is pending', () => {
    expect(shouldFetchPreviousPage({ ...base, anchorPending: true })).toBe(false)
  })
})

describe('computeAnchorRestore', () => {
  it('offsets the anchor by the height the content grew', () => {
    // grew 1500 - 1000 = 500 → new top = 200 + 500
    expect(computeAnchorRestore(1500, { top: 200, height: 1000 })).toBe(700)
  })

  it('returns null when the content did not grow', () => {
    expect(computeAnchorRestore(1000, { top: 200, height: 1000 })).toBeNull()
    expect(computeAnchorRestore(900, { top: 200, height: 1000 })).toBeNull()
  })
})
