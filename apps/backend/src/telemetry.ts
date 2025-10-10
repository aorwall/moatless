let telemetryInitialized = false

/**
 * Initialize OpenTelemetry with Azure Application Insights integration.
 * Telemetry is only enabled if APPLICATIONINSIGHTS_CONNECTION_STRING is set.
 *
 * Uses the Azure Monitor OpenTelemetry distro for simplified setup.
 *
 * IMPORTANT: This function must be called BEFORE any logger imports to ensure
 * Pino instrumentation is registered before Pino instances are created.
 */
export async function initializeTelemetry(): Promise<boolean> {
  const connectionString = process.env['APPLICATIONINSIGHTS_CONNECTION_STRING']

  if (!connectionString) {
    return false
  }

  try {
    // Dynamic imports to avoid requiring packages if not used
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

    // Register Pino instrumentation BEFORE any Pino imports
    // This is critical - the instrumentation MUST be registered before pino is imported anywhere
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

    // NOW we can safely import logger - it will be instrumented
    const { logger } = await import('./utils/logger.js')
    logger.info('Pino instrumentation registered - logs will include trace context')

    // Initialize Azure Monitor with simplified distro
    const { useAzureMonitor } = await import('@azure/monitor-opentelemetry')
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
  } catch {
    // Silently fail - telemetry is optional
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
    // Import logger dynamically for shutdown (it should already be instrumented)
    const { logger } = await import('./utils/logger.js')
    logger.info('Shutting down telemetry...')
    // The Azure Monitor distro handles cleanup automatically
    // We just need to give it time to flush pending telemetry
    await new Promise((resolve) => setTimeout(resolve, 2000))
    telemetryInitialized = false
    logger.info('Telemetry shutdown complete')
  } catch {
    // Silently fail - shutdown is best-effort
  }
}
