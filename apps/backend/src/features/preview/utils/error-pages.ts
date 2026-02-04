import type { ResolutionFailureReason } from '../types/interfaces.js'

interface ErrorPageConfig {
  statusIcon: string
  title: string
  explanation: string
  autoRefresh: boolean
  showTroubleshoot: boolean
}

const reasonConfig: Record<ResolutionFailureReason, ErrorPageConfig> = {
  no_workspace: {
    statusIcon: '&#x2753;', // question mark
    title: 'No Workspace Found',
    explanation:
      'No workspace or environment exists for this preview. The task may not have been started yet.',
    autoRefresh: false,
    showTroubleshoot: false,
  },
  workspace_stopped: {
    statusIcon: '&#x23F8;', // pause
    title: 'Workspace Stopped',
    explanation:
      'The workspace was suspended. It is being restarted automatically&mdash;this page will refresh shortly.',
    autoRefresh: true,
    showTroubleshoot: false,
  },
  workspace_initializing: {
    statusIcon: '&#x23F3;', // hourglass
    title: 'Workspace Starting',
    explanation:
      'The workspace environment is being prepared. This page will refresh automatically.',
    autoRefresh: true,
    showTroubleshoot: false,
  },
  preview_not_started: {
    statusIcon: '&#x26A0;', // warning
    title: 'Preview Server Not Running',
    explanation:
      'The workspace is ready but the preview server has not been started or is not responding.',
    autoRefresh: false,
    showTroubleshoot: true,
  },
  preview_starting: {
    statusIcon: '&#x23F3;', // hourglass
    title: 'Preview Server Starting',
    explanation:
      'The preview server is currently starting up. This page will refresh automatically. If it has been starting for a while, try troubleshooting with the agent.',
    autoRefresh: true,
    showTroubleshoot: true,
  },
  pod_not_running: {
    statusIcon: '&#x23F3;', // hourglass
    title: 'Environment Not Ready',
    explanation:
      'The container is provisioned but not yet in a running state. This page will refresh automatically.',
    autoRefresh: true,
    showTroubleshoot: false,
  },
  connection_refused: {
    statusIcon: '&#x26A0;', // warning
    title: 'Connection Refused',
    explanation:
      'The server was reported as started but is not reachable on the expected port. It may have crashed or be listening on a different port. Try troubleshooting with the agent.',
    autoRefresh: false,
    showTroubleshoot: true,
  },
}

function renderPage(
  title: string,
  icon: string,
  explanation: string,
  options: {
    autoRefresh: boolean
    showTroubleshoot: boolean
    reason: string
    subdomain: string
  },
): string {
  const troubleshootButton = options.showTroubleshoot
    ? `<button id="troubleshoot-btn" onclick="troubleshoot()">Troubleshoot Preview Server</button>`
    : ''

  const autoRefreshScript = options.autoRefresh
    ? `
      let refreshTimer;
      function startAutoRefresh() {
        const statusEl = document.getElementById('auto-refresh-status');
        if (statusEl) statusEl.style.display = 'block';
        refreshTimer = setTimeout(function() { location.reload(); }, 5000);
      }
      startAutoRefresh();
    `
    : ''

  const autoRefreshStatus = options.autoRefresh
    ? `<p id="auto-refresh-status" class="auto-refresh" style="display:none;">Auto-refreshing every 5 seconds&hellip;</p>`
    : ''

  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${title}</title>
  <style>
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    :root { color-scheme: light dark; }
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
      display: flex; align-items: center; justify-content: center;
      min-height: 100vh; padding: 2rem;
      background: #fafafa; color: #333;
    }
    @media (prefers-color-scheme: dark) {
      body { background: #1a1a1a; color: #e0e0e0; }
      .card { background: #242424; border-color: #333; }
      button { background: #2563eb; }
      button:hover { background: #1d4ed8; }
      .auto-refresh { color: #888; }
    }
    .card {
      text-align: center; max-width: 480px; width: 100%;
      padding: 2.5rem 2rem; border-radius: 12px;
      background: #fff; border: 1px solid #e5e5e5;
      box-shadow: 0 1px 3px rgba(0,0,0,0.08);
    }
    .icon { font-size: 3rem; margin-bottom: 1rem; }
    h1 { font-size: 1.25rem; font-weight: 600; margin-bottom: 0.75rem; }
    .explanation { font-size: 0.9rem; line-height: 1.6; opacity: 0.8; margin-bottom: 1.5rem; }
    button {
      display: inline-block; padding: 0.6rem 1.25rem;
      background: #3b82f6; color: #fff; border: none; border-radius: 8px;
      font-size: 0.875rem; font-weight: 500; cursor: pointer;
      transition: background 0.15s;
    }
    button:hover { background: #2563eb; }
    .auto-refresh { font-size: 0.8rem; color: #999; margin-top: 1rem; }
    .subdomain { font-size: 0.75rem; color: #999; margin-top: 1.25rem; font-family: monospace; }
  </style>
</head>
<body>
  <div class="card">
    <div class="icon">${icon}</div>
    <h1>${title}</h1>
    <p class="explanation">${explanation}</p>
    ${troubleshootButton}
    ${autoRefreshStatus}
    <p class="subdomain">${options.subdomain}</p>
  </div>
  <script>
    function troubleshoot() {
      try {
        window.parent.postMessage({
          source: 'preview-iframe',
          type: 'TROUBLESHOOT_REQUEST',
          payload: { reason: '${options.reason}' }
        }, '*');
      } catch(e) {
        // If not in an iframe, just reload
        location.reload();
      }
    }
    ${autoRefreshScript}
  </script>
</body>
</html>`
}

export function renderProxyErrorPage(subdomain: string, reason: ResolutionFailureReason): string {
  const config = reasonConfig[reason]
  return renderPage(config.title, config.statusIcon, config.explanation, {
    autoRefresh: config.autoRefresh,
    showTroubleshoot: config.showTroubleshoot,
    reason,
    subdomain,
  })
}

export function renderBadGatewayPage(subdomain: string): string {
  const config = reasonConfig.connection_refused
  return renderPage(config.title, config.statusIcon, config.explanation, {
    autoRefresh: false,
    showTroubleshoot: true,
    reason: 'connection_refused',
    subdomain,
  })
}
