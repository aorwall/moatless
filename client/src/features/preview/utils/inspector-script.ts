/**
 * Inspector script that runs inside the preview iframe
 * This is injected as a string and executed in the iframe context
 */

export const INSPECTOR_SCRIPT = `
(function() {
  let isActive = false;
  let highlightedElement = null;
  let overlay = null;
  let tooltip = null;
  let lastHoveredElement = null;

  // Create overlay for highlighting
  function createOverlay() {
    if (overlay) return;
    
    overlay = document.createElement('div');
    overlay.id = 'element-inspector-overlay';
    overlay.style.cssText = \`
      position: fixed;
      pointer-events: none;
      border: 2px solid #3b82f6;
      background: rgba(59, 130, 246, 0.1);
      z-index: 999999;
      transition: all 0.1s ease;
      box-shadow: 0 0 0 1px rgba(59, 130, 246, 0.4);
    \`;
    document.body.appendChild(overlay);

    tooltip = document.createElement('div');
    tooltip.id = 'element-inspector-tooltip';
    tooltip.style.cssText = \`
      position: fixed;
      pointer-events: none;
      background: #1e293b;
      color: white;
      padding: 4px 8px;
      border-radius: 4px;
      font-size: 12px;
      font-family: monospace;
      z-index: 999999;
      white-space: nowrap;
      box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
      max-width: 300px;
      overflow: hidden;
      text-overflow: ellipsis;
    \`;
    document.body.appendChild(tooltip);
  }

  // Remove overlay
  function removeOverlay() {
    if (overlay) {
      overlay.remove();
      overlay = null;
    }
    if (tooltip) {
      tooltip.remove();
      tooltip = null;
    }
  }

  // Update overlay position and size
  function updateOverlay(element) {
    if (!overlay || !tooltip) return;
    
    const rect = element.getBoundingClientRect();
    overlay.style.top = rect.top + 'px';
    overlay.style.left = rect.left + 'px';
    overlay.style.width = rect.width + 'px';
    overlay.style.height = rect.height + 'px';

    // Update tooltip
    const selector = createReadableSelector(element);
    tooltip.textContent = selector;
    
    // Position tooltip above element if possible, otherwise below
    const tooltipTop = rect.top - 28;
    if (tooltipTop > 5) {
      tooltip.style.top = tooltipTop + 'px';
    } else {
      tooltip.style.top = rect.bottom + 5 + 'px';
    }
    
    tooltip.style.left = rect.left + 'px';
    
    // Ensure tooltip stays within viewport
    const tooltipRect = tooltip.getBoundingClientRect();
    if (tooltipRect.right > window.innerWidth) {
      tooltip.style.left = (window.innerWidth - tooltipRect.width - 5) + 'px';
    }
  }

  // Generate readable selector
  function createReadableSelector(element) {
    if (element.id) {
      return '#' + element.id;
    }
    
    let selector = element.tagName.toLowerCase();
    
    if (element.className && typeof element.className === 'string') {
      const classes = element.className
        .split(' ')
        .filter(c => c.trim() && !c.includes(':'))
        .slice(0, 2);
      
      if (classes.length > 0) {
        selector += '.' + classes.join('.');
      }
    }
    
    return selector;
  }

  // Generate full CSS selector path
  function generateSelector(element) {
    const path = [];
    let current = element;

    while (current && current.tagName && current.tagName !== 'HTML') {
      let selector = current.tagName.toLowerCase();
      
      if (current.id) {
        selector = '#' + current.id;
        path.unshift(selector);
        break;
      }
      
      if (current.className && typeof current.className === 'string') {
        const classes = current.className
          .split(' ')
          .filter(c => c.trim() && !c.includes(':'));
        if (classes.length > 0) {
          selector += '.' + classes[0];
        }
      }

      const parent = current.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter(
          child => child.tagName === current.tagName
        );
        if (siblings.length > 1) {
          const index = siblings.indexOf(current) + 1;
          selector += ':nth-child(' + index + ')';
        }
      }

      path.unshift(selector);
      current = current.parentElement;
    }

    return path.join(' > ');
  }

  // Get element path
  function getElementPath(element) {
    const path = [];
    let current = element;

    while (current && current.tagName && current.tagName !== 'HTML') {
      path.unshift(createReadableSelector(current));
      current = current.parentElement;
    }

    return path;
  }

  // Get element attributes
  function getElementAttributes(element) {
    const attributes = {};
    for (let i = 0; i < element.attributes.length; i++) {
      const attr = element.attributes[i];
      attributes[attr.name] = attr.value;
    }
    return attributes;
  }

  // Get computed styles
  function getComputedStyles(element) {
    const computed = window.getComputedStyle(element);
    return {
      display: computed.display,
      position: computed.position,
      width: computed.width,
      height: computed.height,
      color: computed.color,
      backgroundColor: computed.backgroundColor,
      fontSize: computed.fontSize,
      fontFamily: computed.fontFamily,
      padding: computed.padding,
      margin: computed.margin,
      border: computed.border
    };
  }

  // Create element info
  function createElementInfo(element) {
    const rect = element.getBoundingClientRect();
    const displayText = createReadableSelector(element);
    
    return {
      tagName: element.tagName.toLowerCase(),
      className: element.className || '',
      id: element.id || '',
      textContent: (element.textContent || '').substring(0, 100).trim(),
      selector: generateSelector(element),
      displayText: displayText,
      elementPath: getElementPath(element),
      attributes: getElementAttributes(element),
      computedStyles: getComputedStyles(element),
      boundingRect: {
        top: rect.top,
        left: rect.left,
        width: rect.width,
        height: rect.height,
        x: rect.x,
        y: rect.y
      }
    };
  }

  // Handle mouse move
  function handleMouseMove(e) {
    if (!isActive) return;
    
    const element = e.target;
    if (element === lastHoveredElement) return;
    
    lastHoveredElement = element;
    highlightedElement = element;
    updateOverlay(element);
  }

  // Handle click
  function handleClick(e) {
    if (!isActive) return;
    
    e.preventDefault();
    e.stopPropagation();
    
    const element = e.target;
    const elementInfo = createElementInfo(element);
    
    // Send element info to parent
    window.parent.postMessage({
      type: 'ELEMENT_SELECTED',
      elementInfo: elementInfo
    }, '*');
    
    // Keep highlighting after selection
    highlightedElement = element;
    updateOverlay(element);
  }

  // Handle mouse leave
  function handleMouseLeave() {
    if (!isActive || !overlay) return;
    overlay.style.display = 'none';
    if (tooltip) tooltip.style.display = 'none';
  }

  // Handle mouse enter
  function handleMouseEnter() {
    if (!isActive || !overlay || !highlightedElement) return;
    overlay.style.display = 'block';
    if (tooltip) tooltip.style.display = 'block';
    updateOverlay(highlightedElement);
  }

  // Activate inspector
  function activate() {
    if (isActive) return;
    
    isActive = true;
    createOverlay();
    
    // Change cursor
    document.body.style.cursor = 'crosshair';
    
    // Add event listeners
    document.addEventListener('mousemove', handleMouseMove, true);
    document.addEventListener('click', handleClick, true);
    document.addEventListener('mouseleave', handleMouseLeave, true);
    document.addEventListener('mouseenter', handleMouseEnter, true);
    
    // Prevent all clicks from doing their default action
    document.addEventListener('click', function(e) {
      if (isActive) {
        e.preventDefault();
        e.stopPropagation();
      }
    }, true);
  }

  // Deactivate inspector
  function deactivate() {
    if (!isActive) return;
    
    isActive = false;
    highlightedElement = null;
    lastHoveredElement = null;
    
    // Reset cursor
    document.body.style.cursor = '';
    
    // Remove overlay
    removeOverlay();
    
    // Remove event listeners
    document.removeEventListener('mousemove', handleMouseMove, true);
    document.removeEventListener('click', handleClick, true);
    document.removeEventListener('mouseleave', handleMouseLeave, true);
    document.removeEventListener('mouseenter', handleMouseEnter, true);
  }

  // Listen for messages from parent
  window.addEventListener('message', function(e) {
    if (e.data.type === 'INSPECTOR_ACTIVATE') {
      if (e.data.active) {
        activate();
      } else {
        deactivate();
      }
    } else if (e.data.type === 'DISABLE_INSPECTOR') {
      deactivate();
    }
  });

  // Signal that inspector is ready
  window.parent.postMessage({ type: 'INSPECTOR_READY' }, '*');
})();
`;