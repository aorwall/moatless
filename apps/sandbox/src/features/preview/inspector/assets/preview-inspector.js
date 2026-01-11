;(() => {
  var d = {
    HIGHLIGHT_COLOR: '#0da2e7',
    HIGHLIGHT_BG: '#0da2e71a',
    DEBOUNCE_MS: 10,
    TOOLTIP_OFFSET: 25,
    Z: 2147483647,
    MAX_TOOLTIP_WIDTH: 200,
    SCROLL_DEBOUNCE_MS: 420,
    ALLOWED_ORIGINS: ['*'],
    FULL_WIDTH_DELTA: 12,
    FEATURES: { NETWORK_MONITORING: !1, SCROLL_MONITORING: !1 },
  }
  ;(() => {
    const g = {
        log: console.log,
        warn: console.warn,
        error: console.error,
        info: console.info,
        debug: console.debug,
      },
      r = {
        log: (...e) => g.log(...e),
        warn: (...e) => g.warn(...e),
        error: (...e) => g.error(...e),
        info: (...e) => g.info(...e),
        debug: (...e) => g.debug(...e),
      },
      u = (e) => {
        try {
          const o = { ...e, source: 'preview-iframe' }
          e.type !== 'HEARTBEAT' && r.log('[Preview Inspector] Sending message to parent:', o),
            window.parent.postMessage(o, '*')
        } catch (o) {
          r.error('preview-inspector: could not post to parent', o)
        }
      },
      h = (e, o) => {
        let a
        return (...c) => {
          a !== void 0 && clearTimeout(a), (a = setTimeout(() => e(...c), o))
        }
      }
    function b() {
      for (const e of ['log', 'warn', 'error', 'info', 'debug'])
        console[e] = (...o) => {
          g[e].apply(console, o)
          const a =
            e === 'error' || e === 'warn'
              ? new Error().stack
                  ?.split(`
`)
                  .slice(2)
                  .join(`
`)
              : void 0
          u({
            type: 'console',
            level: e,
            message: o
              .map((c) => {
                if (typeof c == 'object')
                  try {
                    return JSON.stringify(c, null, 2)
                  } catch {
                    return String(c)
                  }
                return String(c)
              })
              .join(' '),
            args: o,
            stack: a,
          })
        }
    }
    function _() {
      window.addEventListener('error', (e) => {
        u({
          type: 'console',
          level: 'error',
          message: `${e.message} at ${e.filename}:${e.lineno}:${e.colno}`,
          args: [e.message],
          stack: e.error?.stack,
        })
      }),
        window.addEventListener('unhandledrejection', (e) => {
          const o = e.reason?.stack || (e.reason instanceof Error ? String(e.reason.stack) : void 0)
          u({
            type: 'console',
            level: 'error',
            message: `Unhandled Promise Rejection: ${String(e.reason)}`,
            args: [String(e.reason)],
            stack: o,
          })
        })
    }
    function R() {
      const e = window.fetch
      window.fetch = async (...o) => {
        const a = performance.now()
        try {
          const c = await e(...o)
          return (
            u({
              type: 'NETWORK_REQUEST',
              payload: {
                url: (typeof o[0] == 'string' ? o[0] : o[0]?.url) ?? c.url,
                method: o[1]?.method ?? 'GET',
                status: c.status,
                statusText: c.statusText,
                duration: performance.now() - a,
                timestamp: new Date().toISOString(),
                origin: location.origin,
              },
            }),
            c
          )
        } catch (c) {
          const s = c
          throw (
            (u({
              type: 'NETWORK_REQUEST',
              payload: {
                url: typeof o[0] == 'string' ? o[0] : o[0].url,
                method: o[1]?.method ?? 'GET',
                error: { message: s.message, stack: s.stack },
                duration: performance.now() - a,
                timestamp: new Date().toISOString(),
                origin: location.origin,
              },
            }),
            c)
          )
        }
      }
    }
    function N() {
      const e = h(() => {
        u({
          type: 'SCROLL_POSITION',
          payload: {
            scrollY: window.scrollY,
            scrollHeight: document.documentElement.scrollHeight,
            clientHeight: window.innerHeight,
            timestamp: Date.now(),
          },
        })
      }, d.DEBOUNCE_MS)
      window.addEventListener('scroll', e, { passive: !0 })
    }
    function P() {
      let e = window.location.pathname + window.location.search
      window.location.hash?.startsWith('#/') && (e = window.location.hash),
        u({ type: 'ROUTE_CHANGE', payload: { path: e, timestamp: Date.now() } })
      const o = () => {
        let s = window.location.pathname + window.location.search
        window.location.hash?.startsWith('#/') && (s = window.location.hash),
          s !== e &&
            ((e = s), u({ type: 'ROUTE_CHANGE', payload: { path: e, timestamp: Date.now() } }))
      }
      window.addEventListener('popstate', o)
      const a = history.pushState,
        c = history.replaceState
      ;(history.pushState = (...s) => {
        a.apply(history, s), setTimeout(o, 0)
      }),
        (history.replaceState = (...s) => {
          c.apply(history, s), setTimeout(o, 0)
        }),
        window.addEventListener('hashchange', o)
    }
    function A() {
      const e = document.querySelectorAll('[data-component-id]'),
        o = e.length > 0
      r.log(
        '[Preview Inspector] Inspector availability check:',
        o
          ? `Found ${e.length} elements with data-component-id`
          : 'No data-component-id attributes found',
      ),
        u({
          type: 'INSPECTOR_AVAILABILITY',
          payload: { isAvailable: o, elementCount: e.length, timestamp: Date.now() },
        })
    }
    function M() {
      const e = Object.assign(document.createElement('div'), {
        className: 'inspector-tooltip',
        style: `position:fixed;z-index:${d.Z};pointer-events:none;background:${d.HIGHLIGHT_COLOR};color:#fff;padding:4px 8px;border-radius:4px;font-weight:bold;white-space:nowrap;display:none;max-width:${d.MAX_TOOLTIP_WIDTH}px;font-size:14px;box-shadow:0 2px 4px rgba(0,0,0,.2)`,
      })
      document.body.appendChild(e)
      const o = Object.assign(document.createElement('div'), {
        className: 'inspector-hover-overlay',
        style: `position:fixed;z-index:${d.Z - 1};pointer-events:none;outline:1px dashed ${d.HIGHLIGHT_COLOR};background:${d.HIGHLIGHT_BG};display:none;`,
      })
      document.body.appendChild(o)
      let a = !1,
        c = null,
        s = new Map(),
        m = new Map(),
        x = (t) => {
          const { top: n, left: i, width: l } = t.getBoundingClientRect()
          ;(e.textContent = t.tagName.toLowerCase()),
            (e.style.display = 'block'),
            (e.style.opacity = '1'),
            (e.style.left = `${i}px`),
            (e.style.top = `${Math.max(0, n - d.TOOLTIP_OFFSET)}px`),
            Math.abs(l - window.innerWidth) < 5 &&
              ((e.style.left = `${d.FULL_WIDTH_DELTA}px`),
              (e.style.top = `${d.FULL_WIDTH_DELTA}px`))
        },
        G = () => {
          e.style.display = 'none'
        },
        f = (t, n, i = 0) => {
          const l = n.getBoundingClientRect()
          ;(t.style.display = 'block'),
            (t.style.top = `${l.top - i}px`),
            (t.style.left = `${l.left - i}px`),
            (t.style.width = `${l.width + i * 2}px`),
            (t.style.height = `${l.height + i * 2}px`)
        },
        w = (t) => {
          t.style.display = 'none'
        },
        I = h((t) => {
          if (!a) return
          const n = document.elementFromPoint(t.clientX, t.clientY)
          n instanceof HTMLElement && c !== n && ((c = n), x(n), f(o, n, 0))
        }, d.DEBOUNCE_MS),
        y = h(() => {
          G(), w(o)
        }, d.DEBOUNCE_MS),
        F = (t) => {
          if (!t?.getAttribute) return null
          const n = t.getAttribute('data-component-id')
          if (!n) return null
          const i = t.getAttribute('data-component-name')
          if (!i) return null
          const l = n.lastIndexOf(':'),
            p = n.lastIndexOf(':', l - 1)
          if (l === -1 || p === -1)
            return r.warn('[Preview Inspector] Invalid componentId format:', n), null
          const Y = n.substring(0, p),
            O = Number.parseInt(n.substring(p + 1, l), 10),
            C = Number.parseInt(n.substring(l + 1), 10)
          return Number.isNaN(O) || Number.isNaN(C)
            ? (r.warn('[Preview Inspector] Invalid line/column in componentId:', n), null)
            : { componentId: n, path: Y, line: O, column: C, componentName: i }
        },
        $ = (t) => {
          let n = t,
            i = 0,
            l = 10
          for (; n && i < l; ) {
            const p = F(n)
            if (p) return r.log('[Inspector Debug] Found element with data-component-id'), p
            ;(n = n.parentElement), i++
          }
          return r.log('[Inspector Debug] No data-component-id attribute found'), null
        },
        U = () =>
          Object.assign(document.createElement('div'), {
            className: 'inspector-selected-overlay',
            style: `position:fixed;z-index:${d.Z - 1};pointer-events:none;outline:1px dashed ${d.HIGHLIGHT_COLOR};background:${d.HIGHLIGHT_BG};display:none;`,
          }),
        B = (t) => {
          const n = `[data-component-id="${t.replace(/"/g, '\\"')}"]`
          return Array.from(document.querySelectorAll(n))
        },
        W = (t) => {
          if (s.has(t)) {
            r.log('[Inspector] ComponentId already selected:', t)
            return
          }
          const n = B(t)
          if (n.length === 0) {
            r.warn('[Inspector] Could not find any elements with componentId:', t)
            return
          }
          r.log(`[Inspector] Found ${n.length} element(s) with componentId:`, t)
          const i = []
          for (const l of n) {
            const p = U()
            document.body.appendChild(p), f(p, l, 3), i.push(p)
          }
          s.set(t, n),
            m.set(t, i),
            r.log('[Inspector] Added to selection:', t, `(${n.length} instances)`)
        },
        j = (t) => {
          const n = m.get(t)
          if (n) {
            for (const i of n) i.remove()
            m.delete(t)
          }
          s.delete(t), r.log('[Inspector] Removed from selection:', t)
        },
        V = () => {
          for (const t of m.values()) for (const n of t) n.remove()
          s.clear(), m.clear(), r.log('[Inspector] Cleared all selections')
        },
        E = (t) => (a && (t.preventDefault(), t.stopPropagation()), !1),
        T = (t) => {
          if (!a) return
          const n = t.target
          n &&
            n instanceof HTMLElement &&
            ['input', 'textarea', 'select'].includes(n.tagName?.toLowerCase()) &&
            t.preventDefault()
        },
        L = (t) => {
          if (!a) return
          t.preventDefault(), t.stopPropagation()
          const n = document.elementFromPoint(t.clientX, t.clientY)
          if (!(n instanceof HTMLElement)) return
          const i = $(n),
            l = n.getBoundingClientRect(),
            p = {
              top: l.top,
              left: l.left,
              width: l.width,
              height: l.height,
              bottom: l.bottom,
              right: l.right,
            }
          u({
            type: 'ELEMENT_CLICKED',
            payload: i
              ? {
                  componentId: i.componentId,
                  path: i.path,
                  line: i.line,
                  column: i.column,
                  componentName: i.componentName,
                  elementRect: p,
                  timestamp: Date.now(),
                }
              : {
                  componentId: null,
                  path: null,
                  line: null,
                  column: null,
                  componentName: n.tagName.toLowerCase(),
                  elementRect: p,
                  timestamp: Date.now(),
                },
          })
        },
        v = null,
        K = (t) => {
          if (((a = t), (e.style.display = 'none'), a))
            document.addEventListener('mouseover', I),
              document.addEventListener('mouseout', y),
              document.addEventListener('click', L, !0),
              document.addEventListener('mousedown', T, !0),
              document.addEventListener('submit', E, !0),
              document.addEventListener('touchstart', E, !0),
              document.addEventListener('touchend', E, !0),
              (document.body.style.cursor = 'crosshair'),
              (v = document.createElement('style')),
              (v.textContent = '* { scroll-behavior: auto !important; }'),
              document.head.appendChild(v)
          else {
            document.removeEventListener('mouseover', I),
              document.removeEventListener('mouseout', y),
              document.removeEventListener('click', L, !0),
              document.removeEventListener('mousedown', T, !0),
              document.removeEventListener('submit', E, !0),
              document.removeEventListener('touchstart', E, !0),
              document.removeEventListener('touchend', E, !0),
              (document.body.style.cursor = ''),
              v && (v.remove(), (v = null)),
              w(o)
            for (const n of m.values()) for (const i of n) w(i)
            c = null
          }
        },
        S = h(() => {
          if (a) {
            c && f(o, c, 0)
            for (const [t, n] of s.entries()) {
              const i = m.get(t)
              if (i && n) for (let l = 0; l < n.length && l < i.length; l++) f(i[l], n[l], 3)
            }
          }
        }, d.DEBOUNCE_MS)
      return (
        window.addEventListener('scroll', S, { passive: !0 }),
        window.addEventListener('resize', S, { passive: !0 }),
        { toggle: K, addToSelection: W, removeFromSelection: j, clearAllSelections: V }
      )
    }
    function H() {
      return new Promise((e) => {
        document.readyState === 'loading'
          ? document.addEventListener('DOMContentLoaded', () => e(), { once: !0 })
          : e()
      })
    }
    function k(e) {
      r.log('[Preview Inspector] Navigating to:', e)
      try {
        const o = typeof e == 'object' && e !== null && 'path' in e ? e.path : String(e)
        let a = window.location.pathname + window.location.search
        window.location.hash?.startsWith('#/') && (a = window.location.hash)
        a !== o
          ? (r.log('[Preview Inspector] Current path differs, navigating:', { from: a, to: o }),
            o.startsWith('#') ? (window.location.hash = o) : (window.location.href = o))
          : r.log('[Preview Inspector] Already at target path, skipping navigation')
      } catch (o) {
        r.error('[Preview Inspector] Navigation failed:', o)
      }
    }
    function D(e, o, a) {
      const { type: c, payload: s } = e.data ?? {}
      switch (
        ([
          'TOGGLE_SELECTOR',
          'SELECTOR_STATE_REQUEST',
          'CHECK_AVAILABILITY_REQUEST',
          'CLEAR_SELECTION',
          'ADD_TO_SELECTION',
          'REMOVE_FROM_SELECTION',
          'NAVIGATE',
        ].includes(c ?? '') && r.log('[Preview Inspector] Received message from parent:', e.data),
        c)
      ) {
        case 'TOGGLE_SELECTOR':
          r.log('[Preview Inspector] Toggling selector:', s), (o.current = !!s), a.toggle(o.current)
          break
        case 'SELECTOR_STATE_REQUEST':
          u({ type: 'SELECTOR_STATE_RESPONSE', payload: { isActive: o.current } })
          break
        case 'CHECK_AVAILABILITY_REQUEST':
          r.log('[Preview Inspector] Checking availability on request'), A()
          break
        case 'CLEAR_SELECTION':
          r.log('[Preview Inspector] Clearing all selections'), a.clearAllSelections()
          break
        case 'ADD_TO_SELECTION':
          typeof s == 'object' &&
            s !== null &&
            'componentId' in s &&
            (r.log('[Preview Inspector] Adding to selection:', s.componentId),
            a.addToSelection(String(s.componentId)))
          break
        case 'REMOVE_FROM_SELECTION':
          typeof s == 'object' &&
            s !== null &&
            'componentId' in s &&
            (r.log('[Preview Inspector] Removing from selection:', s.componentId),
            a.removeFromSelection(String(s.componentId)))
          break
        case 'NAVIGATE':
          k(s)
          break
      }
    }
    ;(async () => {
      r.log('[Preview Inspector] Script starting to execute...'),
        await H(),
        r.log('[Preview Inspector] DOM is ready')
      const e = { current: !1 },
        o = M()
      r.log('[Preview Inspector] Overlay built'),
        window.addEventListener('message', (a) => {
          D(a, e, o)
        }),
        r.log('[Preview Inspector] Setting up hooks...'),
        b(),
        r.log('[Preview Inspector] Console hooks installed'),
        _(),
        r.log('[Preview Inspector] Error hooks installed'),
        d.FEATURES.NETWORK_MONITORING && (R(), r.log('[Preview Inspector] Fetch hooks installed')),
        d.FEATURES.SCROLL_MONITORING && (N(), r.log('[Preview Inspector] Scroll hooks installed')),
        P(),
        r.log('[Preview Inspector] Navigation hooks installed'),
        r.log('[Preview Inspector] Sending SCRIPT_LOADED message...'),
        u({
          type: 'INSPECTOR_SCRIPT_LOADED',
          payload: { version: '2.0.0-server', serverId: '__SERVER_ID__' },
        }),
        setInterval(() => {
          u({ type: 'HEARTBEAT', payload: { timestamp: Date.now() } })
        }, 5e3),
        r.log('[Preview Inspector] Bootstrap complete!')
    })().catch((e) => {
      r.error('[Preview Inspector] Bootstrap failed:', e)
    })
  })()
})()
