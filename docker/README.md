# Moatless Worker Docker Image

This directory contains the Docker configuration for building the Moatless Worker image, which is used to run the backend in worker mode inside Docker containers.

## Building the Image

### Using the build script (recommended):
```bash
# Build with default settings
./docker/build-worker.sh

# Build with custom tag
./docker/build-worker.sh --tag v1.0.0

# Build for specific platform
./docker/build-worker.sh --platform linux/amd64

# Build and push to registry
./docker/build-worker.sh --push
```

### Using docker-compose:
```bash
# Build the image
docker-compose -f docker/docker-compose.yml --profile build build

# Build with no cache
docker-compose -f docker/docker-compose.yml --profile build build --no-cache
```

### Using docker directly:
```bash
# From repository root
docker build -f docker/Dockerfile -t moatless-vibe-universal:latest .
```

## Worker Mode

The Docker image is configured to run the backend in worker mode automatically:

- **Environment**: `WORKER_MODE=true` is set by default
- **Port**: Listens on port 8001 (different from main backend)
- **API Communication**: Connects back to main backend via `WORKER_API_URL`
- **Data Storage**: Mounts `/data` directory for shared storage
- **Workspace**: Mounts repository to `/workspace`

## Configuration

The image uses the following environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `WORKER_MODE` | Enable worker mode | `true` |
| `WORKER_API_URL` | Main backend API URL | `http://host.docker.internal:8000` |
| `WORKER_API_KEY` | API key for authentication | Required |
| `WORKER_TASK_ID` | Task ID for this worker | Required |
| `WORKSPACE_DIR` | Workspace directory | `/workspace` |
| `DATA_DIR` | Data storage directory | `/data` |
| `PORT` | Worker API port | `8001` |
| `ANTHROPIC_API_KEY` | Claude API key | Optional |
| `CLAUDE_AUTH_DIR` | Claude auth directory | Optional |

## Usage in Repository Configuration

To use the worker image in your repository configuration:

```json
{
  "codeExecution": {
    "workspaceType": "docker",
    "dockerImage": "moatless-vibe-universal:latest",
    "setupCommands": "npm install",
    "workerResources": {
      "cpuLimit": "2",
      "memoryLimit": "4g"
    }
  }
}
```

## Testing the Worker

You can test the worker image using docker-compose:

```bash
# Set required environment variables
export WORKER_API_KEY=your-api-key
export WORKER_TASK_ID=test-task-123
export WORKSPACE_PATH=/path/to/your/workspace
export DATA_PATH=/path/to/data

# Run test worker
docker-compose -f docker/docker-compose.yml --profile test up
```

## Multi-stage Build

The Dockerfile uses a multi-stage build:

1. **Builder stage**: Installs dependencies and builds the application
2. **Runtime stage**: Contains only the built application and runtime dependencies

This approach reduces the final image size and improves security by not including build tools in the runtime image.

## Entrypoint

The image uses a custom entrypoint script (`docker-entrypoint.sh`) that:

1. Initializes firewall rules (if needed)
2. Ensures required directories exist
3. Starts the backend in worker mode
4. Logs startup information

## Troubleshooting

### Image not found
If you get "image not found" errors, make sure you've built the image first:
```bash
./docker/build-worker.sh
```

### Connection refused
If the worker can't connect to the main backend:
- Check that `WORKER_API_URL` is correct
- On Mac/Windows, use `host.docker.internal` instead of `localhost`
- Ensure the main backend is running and accessible

### Permission issues
If you encounter permission issues with mounted volumes:
- The image runs as the `node` user (UID 1000)
- Ensure mounted directories have appropriate permissions
- Consider using `:z` or `:Z` flags for SELinux systems