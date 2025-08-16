/**
 * Generate the full preview inspector script that gets injected into preview iframe
 * This script provides console logging, network monitoring, element inspection, and more
 */
export function getPreviewInspectorScript(taskId: string): string {
  return `
    /* Preview Inspector Script - Injected by Preview Environment Service */
    (function() {
      const CONFIG = {
        HIGHLIGHT_COLOR: '#0da2e7',
        HIGHLIGHT_BG: '#0da2e71a',
        DEBOUNCE_MS: 10,
        TOOLTIP_OFFSET: 25,
        Z: 10000,
        MAX_TOOLTIP_WIDTH: 200,
        SCROLL_DEBOUNCE_MS: 420,
        ALLOWED_ORIGINS: ['*'], // Allow all origins for iframe communication
        FULL_WIDTH_DELTA: 12,
      };

      const ATTR_SELECTED = 'data-inspector-selected';
      const ATTR_HOVERED = 'data-inspector-hovered';

      // Store original console methods for internal logging
      const originalConsole = {
        log: console.log,
        warn: console.warn,
        error: console.error,
        info: console.info,
        debug: console.debug
      };
      
      // Helper for internal logging that doesn't trigger hooks
      const internalLog = {
        log: (...args) => originalConsole.log(...args),
        warn: (...args) => originalConsole.warn(...args),
        error: (...args) => originalConsole.error(...args),
        info: (...args) => originalConsole.info(...args),
        debug: (...args) => originalConsole.debug(...args)
      };
      
      const sendToParent = (payload) => {
        try {
          // Always ensure source is set for all messages
          const messageWithSource = { ...payload, source: 'preview-iframe' };
          
          // Use internal logging to prevent infinite recursion
          internalLog.log('[Preview Inspector] Sending message to parent:', messageWithSource);
          
          window.parent.postMessage(messageWithSource, '*');
        } catch (err) {
          internalLog.error('preview-inspector: could not post to parent', err);
        }
      };

      const debounce = (fn, wait) => {
        let handle;
        return (...args) => {
          clearTimeout(handle);
          handle = setTimeout(() => fn(...args), wait);
        };
      };

      function hookConsole() {
        ['log', 'warn', 'error', 'info', 'debug'].forEach((level) => {
          console[level] = (...args) => {
            // Always call original first to show in browser console
            originalConsole[level].apply(console, args);
            
            const stack = level === 'error' || level === 'warn'
              ? new Error().stack?.split('\\n').slice(2).join('\\n')
              : undefined;
            sendToParent({
              type: 'console',
              level,
              message: args.map((a) => {
                if (typeof a === 'object') {
                  try { return JSON.stringify(a, null, 2); }
                  catch (e) { return String(a); }
                }
                return String(a);
              }).join(' '),
              args,
              stack,
              // Note: removed payload wrapper - ConsoleLogs expects flat structure
            });
          };
        });
      }

      function hookRuntimeErrors() {
        window.addEventListener('error', (ev) => {
          sendToParent({
            type: 'console',
            level: 'error',
            message: ev.message + ' at ' + ev.filename + ':' + ev.lineno + ':' + ev.colno,
            args: [ev.message],
            stack: ev.error?.stack,
          });
        });
        window.addEventListener('unhandledrejection', (ev) => {
          const stack = ev.reason?.stack || (ev.reason instanceof Error ? String(ev.reason.stack) : undefined);
          sendToParent({
            type: 'console',
            level: 'error',
            message: 'Unhandled Promise Rejection: ' + String(ev.reason),
            args: [String(ev.reason)],
            stack,
          });
        });
      }

      function hookFetch() {
        const original = window.fetch;
        window.fetch = async (...args) => {
          const started = performance.now();
          try {
            const resp = await original(...args);
            sendToParent({
              type: 'NETWORK_REQUEST',
              payload: {
                url: args[0] ?? resp.url,
                method: args[1]?.method ?? 'GET',
                status: resp.status,
                statusText: resp.statusText,
                duration: performance.now() - started,
                timestamp: new Date().toISOString(),
                origin: location.origin,
              },
            });
            return resp;
          } catch (err) {
            sendToParent({
              type: 'NETWORK_REQUEST',
              payload: {
                url: args[0],
                method: args[1]?.method ?? 'GET',
                error: { message: err.message, stack: err.stack },
                duration: performance.now() - started,
                timestamp: new Date().toISOString(),
                origin: location.origin,
              },
            });
            throw err;
          }
        };
      }

      function hookScroll() {
        const throttled = debounce(() => {
          sendToParent({
            type: 'SCROLL_POSITION',
            payload: {
              scrollY: window.scrollY,
              scrollHeight: document.documentElement.scrollHeight,
              clientHeight: window.innerHeight,
              timestamp: Date.now(),
            },
          });
        }, CONFIG.DEBOUNCE_MS);
        window.addEventListener('scroll', throttled, { passive: true });
      }

      function hookNavigation() {
        let currentPath = window.location.pathname + window.location.search;
        
        // For hash-based routing, use the hash as the path
        if (window.location.hash && window.location.hash.startsWith('#/')) {
          currentPath = window.location.hash.substring(1); // Remove the # character
        }
        
        // Send initial route
        sendToParent({
          type: 'ROUTE_CHANGE',
          payload: {
            path: currentPath,
            timestamp: Date.now(),
          },
        });
        
        // Hook into browser navigation events
        const checkRouteChange = () => {
          let newPath = window.location.pathname + window.location.search;
          
          // For hash-based routing, use the hash as the path
          if (window.location.hash && window.location.hash.startsWith('#/')) {
            newPath = window.location.hash.substring(1); // Remove the # character
          }
          
          if (newPath !== currentPath) {
            currentPath = newPath;
            sendToParent({
              type: 'ROUTE_CHANGE',
              payload: {
                path: currentPath,
                timestamp: Date.now(),
              },
            });
          }
        };
        
        // Listen for browser navigation events
        window.addEventListener('popstate', checkRouteChange);
        
        // Hook into pushState and replaceState for client-side navigation
        const originalPushState = history.pushState;
        const originalReplaceState = history.replaceState;
        
        history.pushState = function(...args) {
          originalPushState.apply(history, args);
          setTimeout(checkRouteChange, 0); // Use setTimeout to ensure the URL has changed
        };
        
        history.replaceState = function(...args) {
          originalReplaceState.apply(history, args);
          setTimeout(checkRouteChange, 0);
        };
        
        // Also listen for hash changes (for hash-based routing)
        window.addEventListener('hashchange', checkRouteChange);
      }

      function buildOverlay() {
        const tooltip = Object.assign(document.createElement('div'), {
          className: 'inspector-tooltip',
          style: \`position:fixed;z-index:\${CONFIG.Z};pointer-events:none;background:\${CONFIG.HIGHLIGHT_COLOR};color:#fff;padding:4px 8px;border-radius:4px;font-weight:bold;white-space:nowrap;display:none;max-width:\${CONFIG.MAX_TOOLTIP_WIDTH}px;font-size:14px;box-shadow:0 2px 4px rgba(0,0,0,.2)\`,
        });
        document.body.appendChild(tooltip);

        const style = document.createElement('style');
        style.textContent = \`[\${ATTR_HOVERED}]::before,[\${ATTR_SELECTED}]::before{content:'';position:absolute;inset:0;outline:1px dashed \${CONFIG.HIGHLIGHT_COLOR};background:\${CONFIG.HIGHLIGHT_BG};pointer-events:none;z-index:\${CONFIG.Z}}[\${ATTR_SELECTED}]::before{outline-offset:3px}\`;
        document.head.appendChild(style);

        let enabled = false;
        let hovered;

        const showTooltip = (el) => {
          const { top, left, width } = el.getBoundingClientRect();
          tooltip.textContent = el.tagName.toLowerCase();
          tooltip.style.display = 'block';
          tooltip.style.opacity = '1';
          tooltip.style.left = \`\${left}px\`;
          tooltip.style.top = \`\${Math.max(0, top - CONFIG.TOOLTIP_OFFSET)}px\`;
          if (Math.abs(width - window.innerWidth) < 5) {
            tooltip.style.left = \`\${CONFIG.FULL_WIDTH_DELTA}px\`;
            tooltip.style.top = \`\${CONFIG.FULL_WIDTH_DELTA}px\`;
          }
        };

        const hideTooltip = () => (tooltip.style.display = 'none');

        const over = debounce((ev) => {
          if (!enabled) return;
          const target = ev.target;
          if (!(target instanceof HTMLElement)) return;
          hovered?.removeAttribute(ATTR_HOVERED);
          hovered = target;
          showTooltip(target);
          target.setAttribute(ATTR_HOVERED, 'true');
        }, CONFIG.DEBOUNCE_MS);

        const out = debounce(() => {
          hovered?.removeAttribute(ATTR_HOVERED);
          hideTooltip();
        }, CONFIG.DEBOUNCE_MS);

        // Extract React component information from DOM element
        const extractReactInfo = (element) => {
          try {
            // Find React fiber key - React 16+ uses __reactFiber$, older versions use __reactInternalInstance$
            const fiberKey = Object.keys(element).find(key => 
              key.startsWith('__reactFiber') || 
              key.startsWith('__reactInternalInstance')
            );
            
            if (!fiberKey) {
              internalLog.debug('[Inspector] No React fiber found on element');
              return null;
            }
            
            const fiber = element[fiberKey];
            if (!fiber) return null;
            
            // Navigate up the fiber tree to find the nearest component (not DOM element)
            let componentFiber = fiber;
            while (componentFiber) {
              // Check if this is a component (function or class) rather than DOM element
              if (componentFiber.type && typeof componentFiber.type === 'function') {
                const componentInfo = {
                  componentName: componentFiber.type.displayName || componentFiber.type.name || 'Anonymous',
                  props: {},
                  state: null,
                  hooks: null,
                  source: null,
                  owner: null
                };
                
                // Extract props (safely clone to avoid circular references)
                try {
                  if (componentFiber.memoizedProps) {
                    componentInfo.props = JSON.parse(JSON.stringify(componentFiber.memoizedProps, (key, value) => {
                      // Handle special cases
                      if (typeof value === 'function') return '[Function: ' + (value.name || 'anonymous') + ']';
                      if (value instanceof HTMLElement) return '[HTMLElement: ' + value.tagName + ']';
                      if (value === window) return '[Window]';
                      if (value === document) return '[Document]';
                      if (key === '_owner' || key === '_store') return undefined; // Skip React internals
                      return value;
                    }));
                  }
                } catch (e) {
                  componentInfo.props = { _error: 'Failed to serialize props' };
                }
                
                // Extract state for class components or hooks for function components
                try {
                  if (componentFiber.memoizedState) {
                    // Check if it's a function component with hooks
                    if (componentFiber.type && typeof componentFiber.type === 'function' && componentFiber.type.prototype === undefined) {
                      // Function component - extract hooks info
                      const hooks = [];
                      let hookState = componentFiber.memoizedState;
                      let hookIndex = 0;
                      
                      while (hookState) {
                        hooks.push({
                          index: hookIndex++,
                          value: hookState.memoizedState,
                          // Don't expose internal hook structure
                        });
                        hookState = hookState.next;
                      }
                      
                      componentInfo.hooks = hooks.slice(0, 5); // Limit to first 5 hooks
                    } else {
                      // Class component - state is directly available
                      componentInfo.state = JSON.parse(JSON.stringify(componentFiber.memoizedState || null));
                    }
                  }
                } catch (e) {
                  internalLog.debug('[Inspector] Failed to extract state/hooks:', e);
                }
                
                // Extract source location (only available in development builds)
                if (componentFiber._debugSource) {
                  componentInfo.source = {
                    fileName: componentFiber._debugSource.fileName,
                    lineNumber: componentFiber._debugSource.lineNumber,
                    columnNumber: componentFiber._debugSource.columnNumber
                  };
                }
                
                // Extract owner component (who rendered this component)
                if (componentFiber._debugOwner && componentFiber._debugOwner.type) {
                  componentInfo.owner = {
                    componentName: componentFiber._debugOwner.type.displayName || 
                                   componentFiber._debugOwner.type.name || 
                                   'Anonymous'
                  };
                }
                
                return componentInfo;
              }
              
              // Move up to parent in the fiber tree
              componentFiber = componentFiber.return;
            }
            
            return null;
          } catch (error) {
            internalLog.error('[Inspector] Error extracting React info:', error);
            return null;
          }
        };
        
        const click = (ev) => {
          if (!enabled) return;
          ev.preventDefault();
          ev.stopPropagation();
          const target = ev.target;
          if (!(target instanceof HTMLElement)) return;
          target.setAttribute(ATTR_SELECTED, 'true');
          
          // Extract basic DOM information
          const attrs = [...target.attributes].reduce((acc, a) => ({ ...acc, [a.name]: a.value }), {});
          const domInfo = {
            tagName: target.tagName.toLowerCase(),
            attrs,
            text: target.innerText?.substring(0, 100), // Limit text length
            rect: target.getBoundingClientRect(),
            computedStyle: {
              display: window.getComputedStyle(target).display,
              position: window.getComputedStyle(target).position,
              zIndex: window.getComputedStyle(target).zIndex,
              margin: window.getComputedStyle(target).margin,
              padding: window.getComputedStyle(target).padding,
            }
          };
          
          // Extract React component information
          const reactInfo = extractReactInfo(target);
          
          // Send combined information to parent (matching shared interface format)
          sendToParent({
            type: 'ELEMENT_CLICKED',
            payload: {
              dom: domInfo,
              react: reactInfo,
              timestamp: Date.now(),
            },
            isMultiSelect: ev.metaKey || ev.ctrlKey,
          });
        };

        const toggle = (state) => {
          enabled = state;
          tooltip.style.display = 'none';
          if (enabled) {
            document.addEventListener('mouseover', over);
            document.addEventListener('mouseout', out);
            document.addEventListener('click', click, true);
            document.body.style.cursor = 'crosshair';
          } else {
            document.removeEventListener('mouseover', over);
            document.removeEventListener('mouseout', out);
            document.removeEventListener('click', click, true);
            document.body.style.cursor = '';
            document.querySelectorAll(\`[\${ATTR_HOVERED}], [\${ATTR_SELECTED}]\`).forEach((el) =>
              [ATTR_HOVERED, ATTR_SELECTED].forEach((a) => el.removeAttribute(a))
            );
          }
        };

        return { toggle };
      }

      // Wait for DOM to be ready before bootstrapping
      function waitForDOM() {
        return new Promise((resolve) => {
          if (document.readyState === 'loading') {
            document.addEventListener('DOMContentLoaded', resolve, { once: true });
          } else {
            resolve();
          }
        });
      }

      // Bootstrap everything
      (async function() {
        internalLog.log('[Preview Inspector] Script starting to execute...');
        
        // Wait for DOM to be ready
        await waitForDOM();
        internalLog.log('[Preview Inspector] DOM is ready');
        
        let selectorEnabled = false; // Track selector state globally
        const overlay = buildOverlay();
        internalLog.log('[Preview Inspector] Overlay built');

        window.addEventListener('message', (ev) => {
          internalLog.log('[Preview Inspector] Received message from parent:', ev.data);
          
          const { type, payload } = ev.data ?? {};
          switch (type) {
            case 'TOGGLE_SELECTOR':
              internalLog.log('[Preview Inspector] Toggling selector:', payload);
              selectorEnabled = Boolean(payload);
              overlay.toggle(selectorEnabled);
              break;
            case 'SELECTOR_STATE_REQUEST':
              sendToParent({ type: 'SELECTOR_STATE_RESPONSE', payload: { isActive: selectorEnabled } });
              break;
          }
        });

        internalLog.log('[Preview Inspector] Setting up hooks...');
        
        hookConsole();
        internalLog.log('[Preview Inspector] Console hooks installed');
        
        hookRuntimeErrors();
        internalLog.log('[Preview Inspector] Error hooks installed');
        
        hookFetch();
        internalLog.log('[Preview Inspector] Fetch hooks installed');
        
        hookScroll();
        internalLog.log('[Preview Inspector] Scroll hooks installed');
        
        hookNavigation();
        internalLog.log('[Preview Inspector] Navigation hooks installed');
        
        internalLog.log('[Preview Inspector] Sending SCRIPT_LOADED message...');
        sendToParent({ type: 'INSPECTOR_SCRIPT_LOADED', payload: { version: '2.0.0-server', taskId: '${taskId}' } });
        
        // Start sending heartbeat messages every 5 seconds
        setInterval(() => {
          sendToParent({ type: 'HEARTBEAT', payload: { timestamp: Date.now() } });
        }, 5000);
        
        internalLog.log('[Preview Inspector] Bootstrap complete!');
      })().catch(err => {
        internalLog.error('[Preview Inspector] Bootstrap failed:', err);
      });
    })();
  `
}
