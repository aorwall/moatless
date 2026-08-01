import { describe, expect, it } from 'vitest'

import {
  PREVIEW_ANNOTATION_MESSAGE,
  previewAnnotationMessage,
  readPreviewAnnotationMessage,
  START_PICK_CHANNEL,
} from './protocol.js'

describe('readPreviewAnnotationMessage', () => {
  it('reads back what previewAnnotationMessage wrote', () => {
    const sent = previewAnnotationMessage(START_PICK_CHANNEL, [{ colorScheme: 'dark' }])

    expect(readPreviewAnnotationMessage(sent)).toEqual({
      source: PREVIEW_ANNOTATION_MESSAGE,
      channel: START_PICK_CHANNEL,
      args: [{ colorScheme: 'dark' }],
    })
  })

  it("ignores the rest of the page's postMessage traffic", () => {
    // A framed page shares its `message` event with every other frame, library
    // and extension on it — including ones that send strings, nulls, or their
    // own object envelopes.
    expect(readPreviewAnnotationMessage(null)).toBeNull()
    expect(readPreviewAnnotationMessage('webpackHotUpdate')).toBeNull()
    expect(readPreviewAnnotationMessage({ source: 'react-devtools-bridge' })).toBeNull()
    expect(readPreviewAnnotationMessage({ type: 'vite:ping' })).toBeNull()
  })

  it('rejects our own marker carrying a malformed body', () => {
    expect(
      readPreviewAnnotationMessage({ source: PREVIEW_ANNOTATION_MESSAGE, args: [] }),
    ).toBeNull()
    expect(
      readPreviewAnnotationMessage({ source: PREVIEW_ANNOTATION_MESSAGE, channel: 7, args: [] }),
    ).toBeNull()
    expect(
      readPreviewAnnotationMessage({
        source: PREVIEW_ANNOTATION_MESSAGE,
        channel: START_PICK_CHANNEL,
        args: 'start',
      }),
    ).toBeNull()
  })
})
