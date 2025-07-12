import type { ElementInfo } from '../../types'

export function createMockElementInfo(overrides?: Partial<ElementInfo>): ElementInfo {
  return {
    selector: '.test-element',
    displayText: 'Test Element',
    tagName: 'div',
    className: 'test-element',
    id: 'test',
    elementPath: ['body', 'div'],
    attributes: {},
    textContent: 'Test content',
    computedStyles: {
      display: 'block',
      position: 'static',
      width: '100px',
      height: '50px',
      color: 'black',
      backgroundColor: 'white',
      fontSize: '16px',
      fontFamily: 'Arial',
      padding: '0',
      margin: '0',
      border: 'none',
    },
    boundingRect: {
      top: 0,
      left: 0,
      width: 100,
      height: 50,
      x: 0,
      y: 0,
    },
    ...overrides,
  }
}
