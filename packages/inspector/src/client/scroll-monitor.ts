import { debounce, sendMessage } from './communication'
import { CONFIG } from './constants'

/** Install scroll position monitor */
export function installScrollMonitor(): void {
  const reportScroll = debounce(() => {
    sendMessage({
      type: 'SCROLL_POSITION',
      payload: {
        scrollY: window.scrollY,
        scrollHeight: document.documentElement.scrollHeight,
        clientHeight: window.innerHeight,
        timestamp: Date.now(),
      },
    })
  }, CONFIG.DEBOUNCE_MS)

  window.addEventListener('scroll', reportScroll, { passive: true })
}
