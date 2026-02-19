#!/usr/bin/env python3
"""
Docker Workflow E2E Test

This script validates the complete Docker workflow with the Rust backend:
0. Connect to SSE event stream for event verification
1. Verify Docker environment is running
2. Create a vibe task (auto-provisions sandbox + sends initial message)
3. Wait for workspace to be ready (via polling)
4. Wait for agent to complete (via SSE events)
5. Verify the source files were updated
6. Verify SSE events were received (task lifecycle)
7. Clean up

SSE Event Verification:
- Connects to /api/v1/events/stream (NDJSON format)
- Collects events in background thread
- Verifies expected lifecycle events: agent.message, agent.completed,
  taskTurn.completed

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
    # Note: event types use "taskTurn" (not "taskIteration") and "sandbox" (not "environment")
    # agent.started may be missed if emitted before SSE collector connects — optional
    required = [
        ("agent", "completed", "Agent completed"),
        ("taskTurn", "completed", "Task turn completed"),
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
BACKEND_URL = "http://localhost:8080"
DOCKER_NETWORK = "docker-compose_moatless"
TIMEOUT_WORKSPACE = 180  # 3 minutes
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
    print(f"{YELLOW}>>> {message}{NC}")


def print_success(message: str) -> None:
    global tests_passed
    print(f"{GREEN}  PASS: {message}{NC}")
    tests_passed += 1


def print_failure(message: str) -> None:
    global tests_failed
    print(f"{RED}  FAIL: {message}{NC}")
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
        repos = response.json()  # Rust backend returns Vec<RepositoryResponse> directly
        if isinstance(repos, list) and repos:
            return repos[0].get("id")
        return None
    except Exception:
        return None


def create_vibe_task(template_id: str) -> Optional[dict]:
    """Create a new vibe task.

    The Rust backend always provisions the sandbox and sends the initial
    message immediately when message is provided. There is no "start" field.
    """
    try:
        response = requests.post(
            f"{BACKEND_URL}/api/v1/tasks/vibe",
            json={
                "templateId": template_id,
                "message": "Add a heading that says 'E2E Test Success' to the main page",
            },
            headers={"Content-Type": "application/json"},
            timeout=30
        )
        if response.status_code != 200:
            print_info(f"Create task failed with status {response.status_code}: {response.text[:500]}")
            return None
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
                f"{BACKEND_URL}/api/sandbox/v1/tasks/{tid}/status",
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


def get_file_content(tid: str, filepath: str) -> Optional[str]:
    """Fetch file content from the workspace via sandbox API."""
    try:
        response = requests.get(
            f"{BACKEND_URL}/api/sandbox/v1/tasks/{tid}/files/read",
            params={"path": filepath},
            timeout=10
        )
        if response.status_code != 200:
            return None
        data = response.json()
        return data.get("content")
    except Exception:
        return None


def list_files(tid: str, path: str = "", recursive: bool = False) -> list:
    """List files in workspace via sandbox API."""
    try:
        response = requests.get(
            f"{BACKEND_URL}/api/sandbox/v1/tasks/{tid}/files/list",
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
        return response.status_code in (200, 201)
    except Exception as e:
        print_info(f"Error sending message: {e}")
        return False


def wait_for_agent_idle(tid: str, timeout: int) -> bool:
    """Wait for agent to become idle (finished processing)."""
    start_time = time.time()
    agent_started = False

    while time.time() - start_time < timeout:
        try:
            # Check full workspace status for agent state
            response = requests.get(
                f"{BACKEND_URL}/api/sandbox/v1/tasks/{tid}/status",
                timeout=10
            )
            data = response.json()
            agent_state = data.get("agentState")

            # Track if agent ever started running
            if agent_state == "running":
                agent_started = True
                print(".", end="", flush=True)
                time.sleep(POLL_INTERVAL)
                continue

            # Agent is idle (or none) - check if it ran or already completed
            if agent_state in ("none", "idle"):
                if agent_started:
                    print("")
                    return True
                # Agent may have already completed before we started polling.
                # Check if there are messages indicating completion.
                try:
                    msg_resp = requests.get(
                        f"{BACKEND_URL}/api/v1/tasks/{tid}/messages",
                        timeout=10
                    )
                    messages = msg_resp.json()
                    if isinstance(messages, list) and len(messages) > 1:
                        # Agent produced messages, it already completed
                        print("")
                        return True
                except Exception:
                    pass

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

        print_step("Creating vibe task (auto-provisions + sends initial message)...")
        result = create_vibe_task(template_id)
        if result and result.get("taskId"):
            task_id = result["taskId"]
            container_name = f"moatless-sandbox-{task_id}"
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

        print_step("Waiting for workspace ready...")
        # The Rust sandbox publishes sandbox.initializing events during init,
        # then health becomes ready. Wait via polling (fast, 2s intervals).
        if wait_for_workspace_ready(task_id, TIMEOUT_WORKSPACE):
            print_success("Workspace ready")
        else:
            print_failure(f"Workspace not ready within {TIMEOUT_WORKSPACE}s")
            dump_sandbox_logs()
            return 1

        print_step("Verifying container network...")
        if verify_container_network(container_name):
            print_success(f"Container is on network: {DOCKER_NETWORK}")
        else:
            print_failure("Container is on wrong network")

        # ============================================
        # PHASE 4: AGENT EXECUTION
        # ============================================
        print_header("PHASE 4: AGENT EXECUTION")

        # Note: The initial message was already sent during task creation.
        # The agent should have started automatically. We just wait for completion.
        print_step("Waiting for agent.started event...")
        if collector.wait_for_event("agent", "started", task_id, timeout=60):
            print_success("Agent started (via SSE)")
        else:
            print_info("No agent.started event, continuing...")

        print_step("Waiting for agent completion...")
        completion = collector.wait_for_event("taskTurn", "completed", task_id, timeout=TIMEOUT_AGENT)
        if completion:
            print_success("Agent completed (via SSE)")
        else:
            # Check for error event
            error = collector.wait_for_event("agent", "error", task_id, timeout=5)
            if error:
                print_failure(f"Agent error: {error.payload}")
                dump_sandbox_logs()
                return 1
            # Fallback to polling
            print_info("No SSE event, falling back to polling...")
            if wait_for_agent_idle(task_id, TIMEOUT_AGENT):
                print_success("Agent completed (via polling)")
            else:
                print_failure(f"Agent did not complete within {TIMEOUT_AGENT}s")
                dump_sandbox_logs()
                return 1

        # ============================================
        # PHASE 5: CONTENT VERIFICATION
        # ============================================
        print_header("PHASE 5: CONTENT VERIFICATION")

        print_step("Fetching source files to verify changes...")
        # Check common source file locations for the expected heading
        source_files = ["src/pages/Index.tsx", "src/App.tsx", "src/App.jsx", "src/main.tsx", "index.html"]
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
            print_info(f"Files in src/: {files[:10]}")

        print_step("Verifying source was updated with heading...")
        if source_content and "E2E Test Success" in source_content:
            print_success("Source contains 'E2E Test Success' heading")
        else:
            print_failure("Source does not contain expected heading")
            print_info("Expected: 'E2E Test Success'")
            if source_content:
                snippet = source_content[:300] if len(source_content) > 300 else source_content
                print_info(f"Source snippet: {snippet}")
            else:
                print_info("No source content found")

        # Check if agent set the task name via updateTaskMetadata tool (non-critical)
        print_step("Checking task name (optional)...")
        try:
            response = requests.get(f"{BACKEND_URL}/api/v1/tasks/{task_id}", timeout=10)
            task_data = response.json()
            task_name = task_data.get("name")
            if task_name and len(task_name) > 0:
                print_success(f"Task name set: '{task_name}'")
            else:
                print_info("Task name not set (agent-dependent, non-critical)")
        except Exception as e:
            print_info(f"Could not verify task name: {e}")

        # ============================================
        # PHASE 6: EVENT VERIFICATION
        # ============================================
        print_header("PHASE 6: EVENT VERIFICATION")

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
            print(f"{GREEN}{'=' * 65}{NC}")
            print(f"{GREEN}  ALL TESTS PASSED!{NC}")
            print(f"{GREEN}{'=' * 65}{NC}")
            print("")
            print("Docker E2E Workflow verified:")
            print("  - Task created and workspace provisioned")
            print("  - Container attached to correct network")
            print("  - Agent executed and modified content")
            print("  - Content verification successful")
            print("  - SSE events verified")
            return 0
        else:
            print(f"{RED}{'=' * 65}{NC}")
            print(f"{RED}  SOME TESTS FAILED{NC}")
            print(f"{RED}{'=' * 65}{NC}")
            return 1

    finally:
        collector.stop()
        cleanup()


if __name__ == "__main__":
    sys.exit(main())
