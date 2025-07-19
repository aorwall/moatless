# Scripts Directory

## Docker Workspace Testing and Management

### Test Scripts

**Usage:**
```bash
bun test:docker:workspace
```

**Expected Behavior:**
- Creates DockerWorkspace that wraps LocalWorkspace
- Validates file operations delegation
- Attempts to setup Docker container (expected to fail with "Worker backend failed to start" due to base node:18 image)
- Properly cleans up all containers and resources
- Process exits cleanly without hanging

### Cleanup Scripts

#### `cleanup-docker-containers.sh`
Cleanup script for Docker containers created by moatless:

- **Label-Based Cleanup**: Uses `created-by=moatless` label to find containers
- **Legacy Support**: Also cleans up older containers by name patterns
- **Detailed Reporting**: Shows container details before cleanup
- **Safe Operation**: Gracefully handles missing containers

**Usage:**
```bash
bun cleanup:docker
# or directly:
./scripts/cleanup-docker-containers.sh
```

## Docker Container Labeling

All Docker containers created by moatless are automatically labeled:

```bash
--label "created-by=moatless"
--label "moatless.task-id=${taskId}"
--label "moatless.workspace-type=docker"
--label "moatless.created-at=${timestamp}"
```

This enables:
- Easy identification of moatless containers
- Automated cleanup by label filters
- Better container lifecycle management
- Debugging and monitoring capabilities

## Architecture Overview

```
WorkspaceService.createWorkspace()
    ↓
1. Creates LocalWorkspace (always first)
    ↓
2. Creates DockerWorkspace(localWorkspace) 
    ↓
3. DockerWorkspace.setup() creates labeled Docker container
    ↓
4. All file operations delegate to LocalWorkspace
    ↓
5. Command execution happens in Docker container
    ↓
6. Cleanup removes Docker container via DockerWorkspace.teardown()
```

## Troubleshooting

### "Worker backend failed to start within 30 seconds"
This is **expected behavior** when testing with basic Docker images (like `node:18`) that don't contain the moatless backend. The test validates the architecture without requiring a full worker setup.

### Orphaned Containers
If containers are left running after tests:
```bash
bun cleanup:docker
```

### Container Not Cleaning Up
Check that the `DockerWorkspace.teardown()` method is being called. The test includes assertions to verify proper cleanup.