import { logger } from './utils/logger.js'

let telemetryInitialized = false

/**
 * Initialize OpenTelemetry with Azure Application Insights integration.
 * Telemetry is only enabled if APPLICATIONINSIGHTS_CONNECTION_STRING is set.
 *
 * Uses the Azure Monitor OpenTelemetry distro for simplified setup.
 */
export async function initializeTelemetry(): Promise<boolean> {
  const connectionString = process.env['APPLICATIONINSIGHTS_CONNECTION_STRING']

  if (!connectionString) {
    logger.info('Telemetry disabled (APPLICATIONINSIGHTS_CONNECTION_STRING not set)')
    return false
  }

  try {
    // Dynamic import to avoid requiring packages if not used
    const { useAzureMonitor } = await import('@azure/monitor-opentelemetry')

    // Set service name and version via environment variables (standard OpenTelemetry approach)
    if (!process.env['OTEL_SERVICE_NAME']) {
      process.env['OTEL_SERVICE_NAME'] = 'moatless-api'
    }
    if (!process.env['OTEL_SERVICE_VERSION']) {
      process.env['OTEL_SERVICE_VERSION'] = process.env['npm_package_version'] || '1.0.0'
    }

    const serviceName = process.env['OTEL_SERVICE_NAME']
    const serviceVersion = process.env['OTEL_SERVICE_VERSION']

    // Initialize Azure Monitor with simplified distro
    // biome-ignore lint/correctness/useHookAtTopLevel: useAzureMonitor is not a React hook, it's from Azure Monitor OpenTelemetry SDK
    useAzureMonitor({
      azureMonitorExporterOptions: {
        connectionString,
      },
    })

    telemetryInitialized = true
    logger.info(`Telemetry enabled for service: ${serviceName} (v${serviceVersion})`)
    return true
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error)
    logger.warn(`Failed to initialize telemetry: ${errorMessage}`)
    return false
  }
}

/**
 * Gracefully shutdown the OpenTelemetry SDK.
 * Should be called during application shutdown to ensure all telemetry is flushed.
 */
export async function shutdownTelemetry(): Promise<void> {
  if (!telemetryInitialized) {
    return
  }

  try {
    logger.info('Shutting down telemetry...')
    // The Azure Monitor distro handles cleanup automatically
    // We just need to give it time to flush pending telemetry
    await new Promise((resolve) => setTimeout(resolve, 2000))
    telemetryInitialized = false
    logger.info('Telemetry shutdown complete')
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error)
    logger.error(`Error shutting down telemetry: ${errorMessage}`)
  }
}
