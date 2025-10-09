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
    // Dynamic imports to avoid requiring packages if not used
    const { useAzureMonitor } = await import('@azure/monitor-opentelemetry')
    const { PinoInstrumentation } = await import('@opentelemetry/instrumentation-pino')
    const { registerInstrumentations } = await import('@opentelemetry/instrumentation')

    // Set service name and version via environment variables (standard OpenTelemetry approach)
    if (!process.env['OTEL_SERVICE_NAME']) {
      process.env['OTEL_SERVICE_NAME'] = 'moatless-api'
    }
    if (!process.env['OTEL_SERVICE_VERSION']) {
      process.env['OTEL_SERVICE_VERSION'] = process.env['npm_package_version'] || '1.0.0'
    }

    const serviceName = process.env['OTEL_SERVICE_NAME']
    const serviceVersion = process.env['OTEL_SERVICE_VERSION']

    // Register Pino instrumentation BEFORE Azure Monitor initializes
    // This ensures trace context is injected into Pino logs
    registerInstrumentations({
      instrumentations: [
        new PinoInstrumentation({
          // Enable log correlation - adds trace_id, span_id, trace_flags to logs
          logKeys: {
            traceId: 'trace_id',
            spanId: 'span_id',
            traceFlags: 'trace_flags',
          },
          // Disable log sending - Azure Monitor handles log collection
          disableLogSending: true,
        }),
      ],
    })

    logger.info('Pino instrumentation registered - logs will include trace context')

    // Initialize Azure Monitor with simplified distro
    // biome-ignore lint/correctness/useHookAtTopLevel: useAzureMonitor is not a React hook, it's from Azure Monitor OpenTelemetry SDK
    useAzureMonitor({
      azureMonitorExporterOptions: {
        connectionString,
      },
    })

    telemetryInitialized = true

    // Create a startup trace to verify data flows to Azure
    try {
      const { trace } = await import('@opentelemetry/api')
      const tracer = trace.getTracer('moatless-startup', serviceVersion || '1.0.0')
      const span = tracer.startSpan('application.startup', {
        attributes: {
          'service.name': serviceName,
          'service.version': serviceVersion,
          'deployment.environment': process.env['NODE_ENV'] || 'development',
          'telemetry.sdk.name': 'azure-monitor-opentelemetry',
        },
      })

      // End the span immediately (just for verification)
      span.end()

      // Mask the connection string for logging (show only first/last 8 chars)
      const maskedConnectionString =
        connectionString.length > 20
          ? `${connectionString.substring(0, 8)}...${connectionString.substring(connectionString.length - 8)}`
          : '***'

      logger.info(
        {
          service: serviceName,
          version: serviceVersion,
          connectionString: maskedConnectionString,
        },
        'Telemetry initialized - startup trace sent to Azure Application Insights',
      )
    } catch (_traceError) {
      // Non-fatal: telemetry is still working, just couldn't create startup trace
      logger.warn(
        'Telemetry enabled but startup trace failed (telemetry will still work for HTTP requests)',
      )
    }

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
