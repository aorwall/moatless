#!/usr/bin/env python3
"""
Docker Workflow E2E Test

This script validates the complete Docker workflow:
0. Connect to SSE event stream for event verification
1. Verify Docker environment is running
2. Create a vibe task
3. Wait for workspace to be ready (via SSE events)
4. Start and verify preview server is running
5. Run the agent with a request (via SSE events)
6. Verify the preview content was updated
7. Verify SSE events were received (task lifecycle)
8. Clean up

SSE Event Verification:
- Connects to /api/v1/events/stream (NDJSON format)
- Collects events in background thread
- Verifies expected lifecycle events: task, environment.ready,
  taskIteration.active, agent.started, agent.message, agent.completed,
  task.metadataUpdated, taskIteration.completed

Usage: python docker-compose/e2e-test.py
"""

import json
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple

try:
    import requests
except ImportError:
    print("Error: 'requests' package is required. Install with: pip install requests")
    sys.exit(1)


@dataclass
class SSEEvent:
    """Parsed event from SSE stream."""
    event_id: str
    event_type: str
    sub_type: Optional[str]
    task_id: str
    timestamp: Optional[str]
    is_error: bool
    payload: Dict
    raw: Dict


class SSEEventCollector:
    """Background SSE event collector with thread-safe storage."""

    def __init__(self, backend_url: str, debug: bool = False):
        self.backend_url = backend_url
        self.debug = debug
        self.events: List[SSEEvent] = []
        self.events_lock = threading.Lock()
        self.running = False
        self.thread: Optional[threading.Thread] = None
        self.error: Optional[Exception] = None
        self._stop_event = threading.Event()

    def start(self) -> None:
        """Start background SSE collection."""
        self.running = True
        self._stop_event.clear()
        self.thread = threading.Thread(target=self._stream_events, daemon=True)
        self.thread.start()

    def stop(self) -> None:
        """Stop SSE collection."""
        self.running = False
        self._stop_event.set()
        if self.thread:
            self.thread.join(timeout=2)

    def get_events_for_task(self, task_id: str) -> List[SSEEvent]:
        """Get all events for a specific task."""
        with self.events_lock:
            return [e for e in self.events if e.task_id == task_id]

    def wait_for_event(
        self,
        event_type: str,
        sub_type: Optional[str] = None,
        task_id: Optional[str] = None,
        timeout: float = 60.0
    ) -> Optional[SSEEvent]:
        """Wait for specific event with polling."""
        start = time.time()
        while time.time() - start < timeout:
            with self.events_lock:
                for event in reversed(self.events):
                    if event.event_type != event_type:
                        continue
                    if sub_type and event.sub_type != sub_type:
                        continue
                    if task_id and event.task_id != task_id:
                        continue
                    return event
            time.sleep(0.5)
        return None

    def _stream_events(self) -> None:
        """Background thread: process NDJSON stream."""
        try:
            response = requests.get(
                f"{self.backend_url}/api/v1/events/stream",
                headers={"Accept": "text/event-stream"},
                stream=True,
                timeout=None
            )
            response.raise_for_status()

            for line in response.iter_lines(decode_unicode=True):
                if not self.running or self._stop_event.is_set():
                    break
                if not line or not line.strip():
                    continue
                try:
                    data = json.loads(line)
                    if data.get("type") == "ping":
                        continue
                    event = SSEEvent(
                        event_id=data.get("eventId", ""),
                        event_type=data.get("type", ""),
                        sub_type=data.get("subType"),
                        task_id=data.get("taskId", ""),
                        timestamp=data.get("timestamp"),
                        is_error=data.get("isError", False),
                        payload=data.get("payload", {}),
                        raw=data
                    )

                    if self.debug and event.task_id:
                        print(f"  [SSE] {event.event_type}.{event.sub_type or '*'} for {event.task_id[:8]}")

                    with self.events_lock:
                        self.events.append(event)
                except json.JSONDecodeError:
                    pass
        except Exception as e:
            self.error = e
        finally:
            self.running = False


def verify_task_events(events: List[SSEEvent], task_id: str) -> Tuple[bool, List[str]]:
    """Verify expected task lifecycle events occurred."""
    errors = []

    # Required events for full task lifecycle
    required = [
        ("task", None, "Task event"),
        ("environment", "ready", "Environment ready"),
        ("server", "started", "Server started"),
        ("taskIteration", "active", "Iteration active"),
        ("agent", "started", "Agent started"),
        ("agent", "completed", "Agent completed"),
        ("task", "metadataUpdated", "Task metadata updated"),
        ("taskIteration", "completed", "Iteration completed"),
    ]

    task_events = [e for e in events if e.task_id == task_id]

    for event_type, sub_type, description in required:
        found = any(
            e.event_type == event_type and (sub_type is None or e.sub_type == sub_type)
            for e in task_events
        )
        if not found:
            errors.append(f"Missing: {description} ({event_type}.{sub_type or '*'})")

    # Check for agent messages
    messages = [e for e in task_events if e.event_type == "agent" and e.sub_type == "message"]
    if not messages:
        errors.append("Missing: Agent messages (agent.message)")

    return len(errors) == 0, errors


def print_event_summary(events: List[SSEEvent], task_id: str) -> None:
    """Print summary of events received for task."""
    task_events = [e for e in events if e.task_id == task_id]
    print_info(f"Total events collected: {len(task_events)}")

    # Group by type
    by_type: Dict[str, int] = {}
    for e in task_events:
        key = f"{e.event_type}.{e.sub_type}" if e.sub_type else e.event_type
        by_type[key] = by_type.get(key, 0) + 1

    for key, count in sorted(by_type.items()):
        print_info(f"  {key}: {count}")

# Configuration
BACKEND_URL = "http://localhost:8000"
DOCKER_NETWORK = "docker-compose_moatless"
TIMEOUT_WORKSPACE = 180  # 3 minutes
TIMEOUT_PREVIEW = 120    # 2 minutes
TIMEOUT_AGENT = 300      # 5 minutes for agent execution
POLL_INTERVAL = 2        # seconds

# Colors for output
RED = '\033[0;31m'
GREEN = '\033[0;32m'
YELLOW = '\033[1;33m'
BLUE = '\033[0;34m'
NC = '\033[0m'  # No Color

# Test state
tests_passed = 0
tests_failed = 0
task_id: Optional[str] = None
container_name: Optional[str] = None


def print_header(message: str) -> None:
    print(f"\n{BLUE}{'=' * 67}{NC}")
    print(f"{BLUE}  {message}{NC}")
    print(f"{BLUE}{'=' * 67}{NC}\n")


def print_step(message: str) -> None:
    print(f"{YELLOW}▶ {message}{NC}")


def print_success(message: str) -> None:
    global tests_passed
    print(f"{GREEN}✓ {message}{NC}")
    tests_passed += 1


def print_failure(message: str) -> None:
    global tests_failed
    print(f"{RED}✗ {message}{NC}")
    tests_failed += 1


def print_info(message: str) -> None:
    print(f"  {message}")


def dump_sandbox_logs() -> None:
    """Dump sandbox container logs for debugging."""
    global container_name
    if not container_name:
        print(f"{YELLOW}No container name available for log dump{NC}")
        return

    print(f"\n{YELLOW}{'=' * 67}{NC}")
    print(f"{YELLOW}  SANDBOX CONTAINER LOGS: {container_name}{NC}")
    print(f"{YELLOW}{'=' * 67}{NC}\n")

    try:
        # Get container inspect info
        result = subprocess.run(
            ["docker", "inspect", container_name, "--format", "{{.State.Status}} {{.State.ExitCode}}"],
            capture_output=True,
            text=True,
            timeout=10
        )
        if result.returncode == 0:
            print(f"Container state: {result.stdout.strip()}")

        # Get container logs
        result = subprocess.run(
            ["docker", "logs", container_name],
            capture_output=True,
            text=True,
            timeout=30
        )
        if result.stdout:
            print(f"\n--- stdout ---")
            print(result.stdout[-20000:] if len(result.stdout) > 20000 else result.stdout)
        if result.stderr:
            print(f"\n--- stderr ---")
            print(result.stderr[-20000:] if len(result.stderr) > 20000 else result.stderr)

        if not result.stdout and not result.stderr:
            print("(no logs available)")

    except subprocess.TimeoutExpired:
        print(f"{RED}Timeout getting container logs{NC}")
    except Exception as e:
        print(f"{RED}Error getting container logs: {e}{NC}")

    print(f"\n{YELLOW}{'=' * 67}{NC}\n")


def cleanup() -> None:
    """Clean up task and container on exit."""
    global task_id, container_name
    if task_id:
        print(f"\n{YELLOW}Cleaning up...{NC}")
        try:
            requests.delete(f"{BACKEND_URL}/api/v1/tasks/{task_id}", timeout=10)
        except Exception:
            pass
        if container_name:
            try:
                subprocess.run(
                    ["docker", "rm", "-f", container_name],
                    capture_output=True,
                    timeout=10
                )
            except Exception:
                pass
        print(f"{GREEN}Cleanup complete{NC}")


def check_docker_running() -> bool:
    """Check if Docker daemon is running."""
    try:
        result = subprocess.run(
            ["docker", "info"],
            capture_output=True,
            timeout=5
        )
        return result.returncode == 0
    except Exception:
        return False


def check_backend_healthy() -> bool:
    """Check if backend is healthy."""
    try:
        response = requests.get(f"{BACKEND_URL}/health", timeout=5)
        data = response.json()
        return data.get("status") == "ok"
    except Exception:
        return False


def get_template_id() -> Optional[str]:
    """Get the first template repository ID."""
    try:
        response = requests.get(f"{BACKEND_URL}/api/v1/repositories", timeout=10)
        data = response.json()
        repos = data.get("repositories", [])
        if repos:
            return repos[0].get("id")
        return None
    except Exception:
        return None


def create_vibe_task(template_id: str) -> Optional[dict]:
    """Create a new vibe task."""
    try:
        response = requests.post(
            f"{BACKEND_URL}/api/v1/tasks/vibe",
            json={
                "agentType": "claude-code",
                "templateId": template_id,
                "message": "Add a heading that says 'E2E Test Success' to the main page",
                "start": False  # We'll start manually after verifying workspace
            },
            headers={"Content-Type": "application/json"},
            timeout=30
        )
        return response.json()
    except Exception as e:
        print_info(f"Error creating task: {e}")
        return None


def wait_for_workspace_ready(tid: str, timeout: int) -> bool:
    """Wait for workspace to become ready (container running)."""
    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            response = requests.get(
                f"{BACKEND_URL}/api/v1/tasks/{tid}/workspace-status",
                timeout=10
            )
            data = response.json()
            env_status = data.get("environmentStatus")
            workspace_ready = data.get("workspaceReady", False)
            if env_status == "ready" or workspace_ready:
                return True
            print(".", end="", flush=True)
        except Exception:
            print("x", end="", flush=True)
        time.sleep(POLL_INTERVAL)
    print("")
    return False


def start_preview_server(tid: str) -> bool:
    """Start the preview server."""
    try:
        response = requests.post(
            f"{BACKEND_URL}/api/v1/tasks/{tid}/server/start",
            timeout=30
        )
        # Accept 200 OK or 202 Accepted (async operation)
        return response.status_code in (200, 202)
    except Exception:
        return False


def wait_for_preview_ready(tid: str, timeout: int) -> Optional[dict]:
    """Wait for preview server to be ready and return status."""
    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            response = requests.get(
                f"{BACKEND_URL}/api/v1/tasks/{tid}/server/status",
                timeout=10
            )
            data = response.json()
            if data.get("status") == "started":
                return data
            print(".", end="", flush=True)
        except Exception:
            print("x", end="", flush=True)
        time.sleep(POLL_INTERVAL)
    print("")
    return None


def get_preview_content(preview_url: str) -> Optional[str]:
    """Fetch content from preview URL."""
    try:
        response = requests.get(preview_url, timeout=10)
        return response.text
    except Exception:
        return None


def get_file_content(tid: str, filepath: str) -> Optional[str]:
    """Fetch file content from the workspace via backend API."""
    try:
        # URL-encode the filepath to handle slashes
        from urllib.parse import quote
        encoded_path = quote(filepath, safe='')
        response = requests.get(
            f"{BACKEND_URL}/api/v1/tasks/{tid}/files/{encoded_path}",
            timeout=10
        )
        if response.status_code != 200:
            return None
        data = response.json()
        return data.get("content")
    except Exception:
        return None


def list_files(tid: str, path: str = "", recursive: bool = False) -> list:
    """List files in workspace."""
    try:
        response = requests.get(
            f"{BACKEND_URL}/api/v1/tasks/{tid}/files",
            params={"path": path, "recursive": str(recursive).lower()},
            timeout=10
        )
        data = response.json()
        return data.get("files", [])
    except Exception:
        return []


def send_agent_message(tid: str, message: str) -> bool:
    """Send a message to start the agent."""
    try:
        response = requests.post(
            f"{BACKEND_URL}/api/v1/tasks/{tid}/messages",
            json={"message": message},
            headers={"Content-Type": "application/json"},
            timeout=30
        )
        return response.status_code == 200 or response.status_code == 201
    except Exception as e:
        print_info(f"Error sending message: {e}")
        return False


def wait_for_agent_idle(tid: str, timeout: int) -> bool:
    """Wait for agent to become idle (finished processing)."""
    start_time = time.time()
    agent_started = False

    while time.time() - start_time < timeout:
        try:
            # Check workspace status for agent state
            response = requests.get(
                f"{BACKEND_URL}/api/v1/tasks/{tid}/workspace-status",
                timeout=10
            )
            data = response.json()
            agent_state = data.get("agentState")  # Top-level field in response

            # Track if agent ever started running
            if agent_state == "running":
                agent_started = True
                print(".", end="", flush=True)
                time.sleep(POLL_INTERVAL)
                continue

            # Agent is idle (or none) - check if it ran via numTurns
            if agent_started and (agent_state == "none" or agent_state == "idle"):
                # Verify agent did some work
                task_response = requests.get(
                    f"{BACKEND_URL}/api/v1/tasks/{tid}",
                    timeout=10
                )
                task_data = task_response.json()
                num_turns = task_data.get("numTurns", 0)
                if num_turns > 0:
                    print("")
                    return True

            print(".", end="", flush=True)
        except Exception:
            print("x", end="", flush=True)
        time.sleep(POLL_INTERVAL)
    print("")
    return False


def verify_container_network(cname: str) -> bool:
    """Verify container is on the correct network."""
    try:
        result = subprocess.run(
            ["docker", "inspect", cname, "--format",
             "{{range $k, $v := .NetworkSettings.Networks}}{{$k}}{{end}}"],
            capture_output=True,
            text=True,
            timeout=10
        )
        network = result.stdout.strip()
        return network == DOCKER_NETWORK
    except Exception:
        return False


def verify_port_mapping(cname: str) -> Optional[int]:
    """Verify port 3000 is mapped and return host port."""
    try:
        result = subprocess.run(
            ["docker", "port", cname, "3000"],
            capture_output=True,
            text=True,
            timeout=10
        )
        if result.returncode == 0 and result.stdout.strip():
            # Format: 0.0.0.0:PORT or [::]:PORT
            port_str = result.stdout.strip().split(":")[-1]
            return int(port_str)
        return None
    except Exception:
        return None


def main() -> int:
    global task_id, container_name

    # Initialize SSE collector with debug logging
    collector = SSEEventCollector(BACKEND_URL, debug=True)

    try:
        # ============================================
        # PHASE 0: SSE EVENT COLLECTOR
        # ============================================
        print_header("PHASE 0: SSE EVENT COLLECTOR")

        print_step("Starting SSE event collector...")
        collector.start()
        time.sleep(1)  # Allow connection to establish

        if collector.error:
            print_failure(f"SSE connection failed: {collector.error}")
            return 1
        print_success("SSE event collector connected")

        # ============================================
        # PHASE 1: PREREQUISITES
        # ============================================
        print_header("PHASE 1: PREREQUISITES")

        print_step("Checking Docker daemon...")
        if check_docker_running():
            print_success("Docker daemon is running")
        else:
            print_failure("Docker daemon is not running")
            return 1

        print_step("Checking backend health...")
        if check_backend_healthy():
            print_success("Backend is healthy")
        else:
            print_failure("Backend is not healthy")
            print_info("Make sure to run: ./docker-compose/dev-docker.sh")
            return 1

        print_step("Getting template repository...")
        template_id = get_template_id()
        if template_id:
            print_success(f"Template ID: {template_id}")
        else:
            print_failure("No template repository found")
            return 1

        # ============================================
        # PHASE 2: TASK CREATION
        # ============================================
        print_header("PHASE 2: TASK CREATION")

        print_step("Creating vibe task...")
        result = create_vibe_task(template_id)
        if result and result.get("taskId"):
            task_id = result["taskId"]
            container_name = f"moatless-moatable-{task_id[:8]}"
            print_success(f"Task created: {task_id}")
            print_info(f"Container name: {container_name}")
        else:
            print_failure("Failed to create task")
            print_info(f"Response: {result}")
            return 1

        # ============================================
        # PHASE 3: WORKSPACE PROVISIONING
        # ============================================
        print_header("PHASE 3: WORKSPACE PROVISIONING")

        print_step("Waiting for environment.ready event...")
        event = collector.wait_for_event("environment", "ready", task_id, timeout=TIMEOUT_WORKSPACE)
        if event:
            print_success("Received environment.ready event")
        else:
            # Fallback to polling
            print_info("No SSE event, falling back to polling...")
            if wait_for_workspace_ready(task_id, TIMEOUT_WORKSPACE):
                print_success("Workspace ready (via polling)")
            else:
                print_failure(f"Workspace not ready within {TIMEOUT_WORKSPACE}s")
                return 1

        print_step("Verifying container network...")
        if verify_container_network(container_name):
            print_success(f"Container is on network: {DOCKER_NETWORK}")
        else:
            print_failure("Container is on wrong network")

        print_step("Verifying port mapping...")
        host_port = verify_port_mapping(container_name)
        if host_port:
            print_success(f"Preview port 3000 mapped to host port {host_port}")
        else:
            print_failure("Preview port 3000 is not mapped")

        # ============================================
        # PHASE 4: PREVIEW SERVER (auto-started by workspace provisioning)
        # ============================================
        print_header("PHASE 4: PREVIEW SERVER")

        # Preview server is auto-started by WorkspaceCoordinator when workspace is provisioned
        # We just wait for the server.started event
        print_step("Waiting for server.started event (auto-started)...")
        server_event = collector.wait_for_event("server", "started", task_id, timeout=TIMEOUT_PREVIEW)
        if server_event:
            print_success("Received server.started event")
        else:
            print_info("No server.started SSE event, falling back to polling...")

        print_step("Waiting for preview server status...")
        preview_status = wait_for_preview_ready(task_id, TIMEOUT_PREVIEW)
        if preview_status:
            print_success("Preview server is running")
            preview_url = preview_status.get("url")
            print_info(f"Preview URL: {preview_url}")
        else:
            print_failure(f"Preview server not ready within {TIMEOUT_PREVIEW}s")
            # Dump sandbox logs before cleanup for debugging
            dump_sandbox_logs()
            return 1

        print_step("Fetching initial preview content...")
        initial_content = get_preview_content(preview_url)
        if initial_content:
            print_success(f"Initial preview accessible ({len(initial_content)} bytes)")
        else:
            print_failure("Failed to fetch preview content")
            # Don't fail here - preview might just be starting up
            print_info("Continuing despite preview fetch failure...")

        # ============================================
        # PHASE 5: AGENT EXECUTION
        # ============================================
        print_header("PHASE 5: AGENT EXECUTION")

        print_step("Sending request to agent...")
        agent_message = "Add a heading that says 'E2E Test Success' to the main page"
        if send_agent_message(task_id, agent_message):
            print_success("Message sent to agent")
        else:
            print_failure("Failed to send message to agent")
            return 1

        print_step("Waiting for agent.started event...")
        if collector.wait_for_event("agent", "started", task_id, timeout=30):
            print_success("Agent started (via SSE)")
        else:
            print_info("No agent.started event, continuing...")

        print_step("Waiting for agent completion...")
        completion = collector.wait_for_event("taskIteration", "completed", task_id, timeout=TIMEOUT_AGENT)
        if completion:
            print_success("Agent completed (via SSE)")
        else:
            # Check for error event
            error = collector.wait_for_event("agent", "error", task_id, timeout=5)
            if error:
                print_failure(f"Agent error: {error.payload}")
                return 1
            # Fallback to polling
            print_info("No SSE event, falling back to polling...")
            if wait_for_agent_idle(task_id, TIMEOUT_AGENT):
                print_success("Agent completed (via polling)")
            else:
                print_failure(f"Agent did not complete within {TIMEOUT_AGENT}s")
                return 1

        # ============================================
        # PHASE 6: CONTENT VERIFICATION
        # ============================================
        print_header("PHASE 6: CONTENT VERIFICATION")

        print_step("Fetching source files to verify changes...")
        # Check the main React component for the expected heading
        # Common locations: src/App.tsx, src/App.jsx, src/main.tsx
        source_files = ["src/App.tsx", "src/App.jsx", "src/main.tsx", "index.html"]
        source_content = None
        source_file_found = None

        for src_file in source_files:
            content = get_file_content(task_id, src_file)
            if content and len(content) > 0:
                print_info(f"Checked {src_file}: {len(content)} bytes")
                if source_content is None:
                    source_content = content
                    source_file_found = src_file
                # Check if this file contains our heading
                if "E2E Test Success" in content:
                    source_content = content
                    source_file_found = src_file
                    break

        if source_content:
            print_success(f"Source file fetched: {source_file_found} ({len(source_content)} bytes)")
        else:
            print_failure("Failed to fetch any source files")
            # Try to list files for debugging
            files = list_files(task_id, "src", recursive=True)
            print_info(f"Files in src/: {[f.get('name') for f in files[:10]]}")

        print_step("Verifying source was updated with heading...")
        if source_content and "E2E Test Success" in source_content:
            print_success("Source contains 'E2E Test Success' heading")
        else:
            print_failure("Source does not contain expected heading")
            print_info("Expected: 'E2E Test Success'")
            if source_content:
                # Show a snippet of what we got
                snippet = source_content[:300] if len(source_content) > 300 else source_content
                print_info(f"Source snippet: {snippet}")
            else:
                print_info("No source content found")

        # Verify agent set the task name via updateTaskMetadata tool
        print_step("Verifying task name was set by agent...")
        try:
            response = requests.get(f"{BACKEND_URL}/api/v1/tasks/{task_id}", timeout=10)
            task_data = response.json()
            task_name = task_data.get("name")
            if task_name and len(task_name) > 0:
                print_success(f"Task name set: '{task_name}'")
            else:
                print_failure("Task name was not set by the agent")
        except Exception as e:
            print_failure(f"Failed to verify task name: {e}")

        # Also verify preview server is still accessible
        print_step("Verifying preview server accessibility...")
        updated_html = get_preview_content(preview_url)
        if updated_html:
            print_success(f"Preview server still accessible ({len(updated_html)} bytes)")
        else:
            print_failure("Preview server not accessible")

        # ============================================
        # PHASE 7: EVENT VERIFICATION
        # ============================================
        print_header("PHASE 7: EVENT VERIFICATION")

        all_events = collector.get_events_for_task(task_id)
        print_event_summary(all_events, task_id)

        print_step("Verifying task lifecycle events...")
        success, errors = verify_task_events(all_events, task_id)
        if success:
            print_success("All expected events received")
        else:
            for error in errors:
                print_failure(error)

        print_step("All events received during test:")
        with collector.events_lock:
            print_info(f"Total events: {len(collector.events)}")
            by_type: Dict[str, int] = {}
            for e in collector.events:
                key = f"{e.event_type}.{e.sub_type}" if e.sub_type else e.event_type
                by_type[key] = by_type.get(key, 0) + 1
            for key, count in sorted(by_type.items()):
                print_info(f"  {key}: {count}")

        # ============================================
        # SUMMARY
        # ============================================
        print_header("TEST SUMMARY")

        print(f"  {GREEN}Passed: {tests_passed}{NC}")
        print(f"  {RED}Failed: {tests_failed}{NC}")
        print(f"  Total:  {tests_passed + tests_failed}")
        print("")

        if tests_failed == 0:
            print(f"{GREEN}╔{'═' * 65}╗{NC}")
            print(f"{GREEN}║{'ALL TESTS PASSED!':^65}║{NC}")
            print(f"{GREEN}╚{'═' * 65}╝{NC}")
            print("")
            print("Docker E2E Workflow verified:")
            print("  - Task created and workspace provisioned")
            print("  - Container attached to correct network")
            print("  - Preview server started and accessible")
            print("  - Agent executed and modified content")
            print("  - Agent set task name via updateTaskMetadata")
            print("  - Content verification successful")
            print("  - SSE events verified")
            return 0
        else:
            print(f"{RED}╔{'═' * 65}╗{NC}")
            print(f"{RED}║{'SOME TESTS FAILED':^65}║{NC}")
            print(f"{RED}╚{'═' * 65}╝{NC}")
            return 1

    finally:
        collector.stop()
        cleanup()


if __name__ == "__main__":
    sys.exit(main())
