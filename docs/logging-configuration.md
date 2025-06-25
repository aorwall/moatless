# Logging Configuration

## Request Logging

By default, the server only logs completed requests to reduce log verbosity. This shows you the status code and response time for each request:

```
[12:25:18 UTC] INFO: request completed
    reqId: "req-2"
    res: {
      "statusCode": 200
    }
    responseTime: 3.9444169998168945
```

### Enabling Incoming Request Logs

If you need to see incoming request logs for debugging purposes, you can enable them by setting the `DISABLE_REQUEST_LOGGING` environment variable to `false`:

```bash
# In your .env file
DISABLE_REQUEST_LOGGING=false

# Or when running the server
DISABLE_REQUEST_LOGGING=false pnpm dev:server
```

With incoming request logs enabled, you'll see both incoming and completed logs:

```
[12:25:18 UTC] INFO: incoming request
    reqId: "req-2"
    req: {
      "method": "GET",
      "url": "/api/v1/tasks",
      "host": "localhost:8000",
      "remoteAddress": "127.0.0.1",
      "remotePort": 57453
    }
[12:25:18 UTC] INFO: request completed
    reqId: "req-2"
    res: {
      "statusCode": 200
    }
    responseTime: 3.9444169998168945
```

### Log Levels

You can control the overall log level using the `LOG_LEVEL` environment variable:

```bash
# Available levels: trace, debug, info, warn, error, fatal
LOG_LEVEL=debug pnpm dev:server
```