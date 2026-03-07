#!/usr/bin/env python3
"""
Access Control Smoke Test

Validates team management, scope-based access control, and credential
resolution against the Rust backend running in Docker Compose.

Simulates multiple users via X-User-Id / X-Team-Ids / X-User-Role headers.
No OAuth or API keys needed — works in Docker Compose dev mode.

Usage: python docker-compose/access-control-test.py
"""

import json
import os
import random
import string
import sys
import time
from typing import Any, Dict, List, Optional

try:
    import requests
except ImportError:
    print("Error: 'requests' package is required. Install with: pip install requests")
    sys.exit(1)


# --- Configuration ---
BACKEND_URL = os.environ.get("BACKEND_URL", "http://localhost:8080")
RUN_SUFFIX = "".join(random.choices(string.ascii_lowercase + string.digits, k=6))

# Simulated users (sent via ForwardAuth headers)
USER_ALICE = {"X-User-Id": "alice", "X-User-Login": "alice"}
USER_BOB = {"X-User-Id": "bob", "X-User-Login": "bob"}
ADMIN_CAROL = {"X-User-Id": "carol", "X-User-Login": "carol", "X-User-Role": "admin"}

# --- Output helpers ---
RED = "\033[0;31m"
GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
BLUE = "\033[0;34m"
NC = "\033[0m"

tests_passed = 0
tests_failed = 0
tests_skipped = 0


def print_header(msg: str) -> None:
    print(f"\n{BLUE}{'=' * 65}{NC}")
    print(f"{BLUE}  {msg}{NC}")
    print(f"{BLUE}{'=' * 65}{NC}\n")


def print_step(msg: str) -> None:
    print(f"{YELLOW}>>> {msg}{NC}")


def ok(msg: str) -> None:
    global tests_passed
    print(f"{GREEN}  PASS: {msg}{NC}")
    tests_passed += 1


def fail(msg: str) -> None:
    global tests_failed
    print(f"{RED}  FAIL: {msg}{NC}")
    tests_failed += 1


def skip(msg: str) -> None:
    global tests_skipped
    print(f"{YELLOW}  SKIP: {msg}{NC}")
    tests_skipped += 1


def info(msg: str) -> None:
    print(f"  {msg}")


# --- HTTP helpers ---

def api(
    method: str,
    path: str,
    user: Dict[str, str],
    json_body: Any = None,
    team_ids: Optional[List[str]] = None,
) -> requests.Response:
    """Make an API call with simulated user headers."""
    headers = {**user, "Content-Type": "application/json"}
    if team_ids:
        headers["X-Team-Ids"] = ",".join(team_ids)
    url = f"{BACKEND_URL}{path}"
    return requests.request(method, url, headers=headers, json=json_body, timeout=15)


def check_backend() -> bool:
    try:
        r = requests.get(f"{BACKEND_URL}/health", timeout=5)
        return r.json().get("status") == "ok"
    except Exception:
        return False


def setup_users() -> bool:
    """Seed test users directly in postgres via docker exec."""
    import subprocess

    users = [
        ("alice", "alice", "test", "user"),
        ("bob", "bob", "test", "user"),
        ("carol", "carol", "test", "admin"),
    ]
    for uid, login, provider, role in users:
        sql = (
            f"INSERT INTO users (id, login, provider, role, created_at, last_login_at) "
            f"VALUES ('{uid}', '{login}', '{provider}', '{role}', NOW(), NOW()) "
            f"ON CONFLICT (id) DO UPDATE SET login = EXCLUDED.login, role = EXCLUDED.role;"
        )
        result = subprocess.run(
            [
                "docker", "exec", "moatless-postgres",
                "psql", "-U", "moatless", "-d", "moatless", "-c", sql,
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode != 0:
            info(f"Failed to create user {uid}: {result.stderr.strip()}")
            return False
    return True


# --- Cleanup registry ---
cleanup_actions: List[tuple] = []  # (method, path, user, team_ids)


def register_cleanup(method: str, path: str, user: Dict[str, str], team_ids: Optional[List[str]] = None):
    cleanup_actions.append((method, path, user, team_ids))


def run_cleanup():
    if not cleanup_actions:
        return
    print_step("Cleaning up created resources...")
    for method, path, user, team_ids in reversed(cleanup_actions):
        try:
            api(method, path, user, team_ids=team_ids)
        except Exception:
            pass
    info("Cleanup done")


# ====================================================================
# Chain 1: Team CRUD  (AUTH-01 through AUTH-04.1)
# ====================================================================

def chain_team_crud() -> Optional[str]:
    """Returns team_id if chain succeeds, None on critical failure."""
    print_header("Chain 1: Team CRUD")

    # AUTH-01: Create a team
    print_step("AUTH-01: Create a team")
    team_slug = f"eng-test-{RUN_SUFFIX}"
    r = api("POST", "/api/v1/teams", USER_ALICE, {
        "name": "Engineering",
        "slug": team_slug,
        "description": "Test team",
    })
    if r.status_code == 201:
        team = r.json()
        team_id = team.get("id")
        ok(f"AUTH-01: Team created (id={team_id})")
        register_cleanup("DELETE", f"/api/v1/teams/{team_id}", USER_ALICE)
    else:
        fail(f"AUTH-01: Create team failed: {r.status_code} {r.text[:200]}")
        return None

    # AUTH-01.1: Team appears in listing
    print_step("AUTH-01.1: Team appears in listing")
    r = api("GET", "/api/v1/teams", USER_ALICE)
    teams = r.json() if r.status_code == 200 else []
    if any(t.get("id") == team_id for t in teams):
        ok("AUTH-01.1: Team found in listing")
    else:
        fail("AUTH-01.1: Team not found in listing")

    # AUTH-02: Get team by ID
    print_step("AUTH-02: Get team by ID")
    r = api("GET", f"/api/v1/teams/{team_id}", USER_ALICE)
    if r.status_code == 200 and r.json().get("slug") == team_slug:
        ok("AUTH-02: Team fetched by ID")
    else:
        fail(f"AUTH-02: Get team failed: {r.status_code}")

    # AUTH-03: Update team
    print_step("AUTH-03: Update team")
    r = api("PUT", f"/api/v1/teams/{team_id}", USER_ALICE, {
        "name": "Platform Engineering",
        "description": "Updated description",
    })
    if r.status_code == 200:
        updated = r.json()
        if updated.get("name") == "Platform Engineering" and updated.get("slug") == team_slug:
            ok("AUTH-03: Team updated (name changed, slug preserved)")
        else:
            fail(f"AUTH-03: Unexpected update result: {updated}")
    else:
        fail(f"AUTH-03: Update team failed: {r.status_code}")

    # We keep the team for other chains — don't delete yet
    # AUTH-04 and AUTH-04.1 tested at the end as part of cleanup verification

    return team_id


def chain_team_crud_delete(team_id: str) -> None:
    """AUTH-04 and AUTH-04.1: Delete team and verify removal."""
    print_header("Chain 1 (cont): Team Delete")

    # AUTH-04: Delete team
    # First remove all members and clear FK references (soft-deleted repos keep team_id)
    r_members = api("GET", f"/api/v1/teams/{team_id}/members", USER_ALICE)
    if r_members.status_code == 200:
        for m in r_members.json():
            uid = m.get("userId")
            if uid:
                api("DELETE", f"/api/v1/teams/{team_id}/members/{uid}", USER_ALICE)

    # Clear team_id on soft-deleted repos and other FK references
    import subprocess
    for table in ["repositories", "channel_configs", "provider_tokens"]:
        subprocess.run(
            ["docker", "exec", "moatless-postgres", "psql", "-U", "moatless", "-d", "moatless",
             "-c", f"UPDATE {table} SET team_id = NULL WHERE team_id = '{team_id}';"],
            capture_output=True, timeout=10,
        )

    print_step("AUTH-04: Delete team")
    r = api("DELETE", f"/api/v1/teams/{team_id}", USER_ALICE)
    if r.status_code == 204:
        ok("AUTH-04: Team deleted")
    else:
        fail(f"AUTH-04: Delete team failed: {r.status_code}")
        return

    # AUTH-04.1: Team removed from listing
    print_step("AUTH-04.1: Team removed from listing")
    r = api("GET", "/api/v1/teams", USER_ALICE)
    teams = r.json() if r.status_code == 200 else []
    if not any(t.get("id") == team_id for t in teams):
        ok("AUTH-04.1: Team no longer in listing")
    else:
        fail("AUTH-04.1: Deleted team still in listing")


# ====================================================================
# Chain 2: Team Membership  (AUTH-05 through AUTH-07.1)
# ====================================================================

def chain_team_membership(team_id: str) -> None:
    print_header("Chain 2: Team Membership")

    # AUTH-05: Add member to team
    print_step("AUTH-05: Add member to team")
    r = api("POST", f"/api/v1/teams/{team_id}/members", USER_ALICE, {
        "userId": "bob",
        "role": "member",
    })
    if r.status_code == 204:
        ok("AUTH-05: Member added")
    else:
        fail(f"AUTH-05: Add member failed: {r.status_code} {r.text[:200]}")
        return

    # AUTH-05.1: Member appears in team member listing
    print_step("AUTH-05.1: Member appears in listing")
    r = api("GET", f"/api/v1/teams/{team_id}/members", USER_ALICE)
    members = r.json() if r.status_code == 200 else []
    if any(m.get("userId") == "bob" and m.get("role") == "member" for m in members):
        ok("AUTH-05.1: Bob found in member listing with role 'member'")
    else:
        fail(f"AUTH-05.1: Bob not found in listing: {members}")

    # AUTH-07: Remove member from team
    print_step("AUTH-07: Remove member from team")
    r = api("DELETE", f"/api/v1/teams/{team_id}/members/bob", USER_ALICE)
    if r.status_code == 204:
        ok("AUTH-07: Member removed")
    else:
        fail(f"AUTH-07: Remove member failed: {r.status_code}")

    # AUTH-07.1: Removed member no longer in listing
    print_step("AUTH-07.1: Removed member not in listing")
    r = api("GET", f"/api/v1/teams/{team_id}/members", USER_ALICE)
    members = r.json() if r.status_code == 200 else []
    if not any(m.get("userId") == "bob" for m in members):
        ok("AUTH-07.1: Bob no longer in listing")
    else:
        fail("AUTH-07.1: Removed member still in listing")


# ====================================================================
# Chain 3: Team Credentials  (AUTH-08 through AUTH-08.2.1)
# ====================================================================

def chain_team_credentials(team_id: str) -> None:
    print_header("Chain 3: Team Credentials")

    # AUTH-08: Upsert team credential
    print_step("AUTH-08: Upsert team credential")
    r = api("PUT", f"/api/v1/teams/{team_id}/credentials/github", USER_ALICE, {
        "accessToken": "ghp_test_token_12345",
    }, team_ids=[team_id])
    if r.status_code == 204:
        ok("AUTH-08: Credential stored")
    else:
        fail(f"AUTH-08: Upsert credential failed: {r.status_code} {r.text[:200]}")
        return

    # AUTH-08.1: Credential appears in listing
    print_step("AUTH-08.1: Credential appears in listing")
    r = api("GET", f"/api/v1/teams/{team_id}/credentials", USER_ALICE, team_ids=[team_id])
    creds = r.json() if r.status_code == 200 else []
    github_cred = next((c for c in creds if c.get("provider") == "github"), None)
    if github_cred:
        if "accessToken" not in json.dumps(github_cred) or "ghp_" not in json.dumps(github_cred):
            ok("AUTH-08.1: Github credential in listing (token not exposed)")
        else:
            fail("AUTH-08.1: Token value exposed in listing response!")
    else:
        fail(f"AUTH-08.1: Github credential not in listing: {creds}")

    # AUTH-08.2: Delete team credential
    print_step("AUTH-08.2: Delete team credential")
    r = api("DELETE", f"/api/v1/teams/{team_id}/credentials/github", USER_ALICE, team_ids=[team_id])
    if r.status_code == 204:
        ok("AUTH-08.2: Credential deleted")
    else:
        fail(f"AUTH-08.2: Delete credential failed: {r.status_code}")

    # AUTH-08.2.1: Credential removed from listing
    print_step("AUTH-08.2.1: Credential removed from listing")
    r = api("GET", f"/api/v1/teams/{team_id}/credentials", USER_ALICE, team_ids=[team_id])
    creds = r.json() if r.status_code == 200 else []
    if not any(c.get("provider") == "github" for c in creds):
        ok("AUTH-08.2.1: Credential no longer in listing")
    else:
        fail("AUTH-08.2.1: Deleted credential still in listing")


# ====================================================================
# Standalone: AUTH-09, AUTH-10
# ====================================================================

def standalone_duplicate_slug(team_id: str) -> None:
    print_header("Standalone: AUTH-09")

    print_step("AUTH-09: Create team with duplicate slug")
    # Use the same slug as the already-created team
    dup_slug = f"eng-test-{RUN_SUFFIX}"
    r = api("POST", "/api/v1/teams", USER_ALICE, {
        "name": "Duplicate",
        "slug": dup_slug,
    })
    if r.status_code in (409, 400, 422):
        ok(f"AUTH-09: Duplicate slug rejected ({r.status_code})")
    elif r.status_code == 201:
        # Clean up the accidentally created team
        dup_id = r.json().get("id")
        if dup_id:
            api("DELETE", f"/api/v1/teams/{dup_id}", USER_ALICE)
        fail("AUTH-09: Duplicate slug was accepted (should fail)")
    else:
        fail(f"AUTH-09: Unexpected status: {r.status_code} {r.text[:200]}")


def standalone_non_member_credentials(team_id: str) -> None:
    print_header("Standalone: AUTH-10")

    print_step("AUTH-10: Non-member cannot manage team credentials")
    # Bob is not a member of the team — should fail
    r = api("PUT", f"/api/v1/teams/{team_id}/credentials/github", USER_BOB, {
        "accessToken": "ghp_unauthorized",
    })
    if r.status_code in (403, 401):
        ok(f"AUTH-10: Non-member credential upsert rejected ({r.status_code})")
    elif r.status_code == 204:
        # Clean up
        api("DELETE", f"/api/v1/teams/{team_id}/credentials/github", USER_BOB)
        fail("AUTH-10: Non-member was able to store credential")
    else:
        fail(f"AUTH-10: Unexpected status: {r.status_code} {r.text[:200]}")


# ====================================================================
# Chain 1 (Access Control): Repository Scope Filtering
#   AUTH-20 through AUTH-22.2
# ====================================================================

def chain_repo_scope(team_id: str) -> Dict[str, str]:
    """Returns dict of scope -> repo_id for created repos."""
    print_header("Chain 1 (Access Control): Repository Scope Filtering")
    repos = {}

    # AUTH-20: Create global-scoped repository
    print_step("AUTH-20: Create global-scoped repository")
    r = api("POST", "/api/v1/repositories", USER_ALICE, {
        "name": f"global-test-repo-{RUN_SUFFIX}",
        "kind": "template",
        "scope": "global",
    })
    if r.status_code == 201:
        repo = r.json()
        repos["global"] = repo["id"]
        register_cleanup("DELETE", f"/api/v1/repositories/{repo['id']}", USER_ALICE)
        if repo.get("scope") == "global":
            ok(f"AUTH-20: Global repo created (id={repo['id']})")
        else:
            fail(f"AUTH-20: Repo scope is '{repo.get('scope')}', expected 'global'")
    else:
        fail(f"AUTH-20: Create global repo failed: {r.status_code} {r.text[:200]}")

    # AUTH-20.1: Other user can see global repository
    if "global" in repos:
        print_step("AUTH-20.1: Other user can see global repository")
        r = api("GET", "/api/v1/repositories", USER_BOB)
        repo_list = r.json() if r.status_code == 200 else []
        if any(rp.get("id") == repos["global"] for rp in repo_list):
            ok("AUTH-20.1: Bob can see global repo")
        else:
            fail("AUTH-20.1: Bob cannot see global repo")

    # AUTH-21: Create team-scoped repository
    print_step("AUTH-21: Create team-scoped repository")
    r = api("POST", "/api/v1/repositories", USER_ALICE, {
        "name": f"team-test-repo-{RUN_SUFFIX}",
        "kind": "template",
        "scope": "team",
        "teamId": team_id,
    }, team_ids=[team_id])
    if r.status_code == 201:
        repo = r.json()
        repos["team"] = repo["id"]
        register_cleanup("DELETE", f"/api/v1/repositories/{repo['id']}", USER_ALICE, [team_id])
        if repo.get("scope") == "team" and repo.get("teamId") == team_id:
            ok(f"AUTH-21: Team repo created (id={repo['id']})")
        else:
            fail(f"AUTH-21: Unexpected scope/team: scope={repo.get('scope')}, teamId={repo.get('teamId')}")
    else:
        fail(f"AUTH-21: Create team repo failed: {r.status_code} {r.text[:200]}")

    # AUTH-21.1: Team member can see team repository
    if "team" in repos:
        print_step("AUTH-21.1: Team member can see team repository")
        # Alice is creator and team member
        r = api("GET", "/api/v1/repositories", USER_ALICE, team_ids=[team_id])
        repo_list = r.json() if r.status_code == 200 else []
        if any(rp.get("id") == repos["team"] for rp in repo_list):
            ok("AUTH-21.1: Alice (team member) can see team repo")
        else:
            fail(f"AUTH-21.1: Alice cannot see team repo (got {len(repo_list)} repos)")

    # AUTH-21.2: Non-member cannot see team repository
    if "team" in repos:
        print_step("AUTH-21.2: Non-member cannot see team repository")
        r = api("GET", "/api/v1/repositories", USER_BOB)
        repo_list = r.json() if r.status_code == 200 else []
        if not any(rp.get("id") == repos["team"] for rp in repo_list):
            ok("AUTH-21.2: Bob (non-member) cannot see team repo")
        else:
            fail("AUTH-21.2: Bob can see team repo (should not)")

    # AUTH-22: Create user-scoped repository
    print_step("AUTH-22: Create user-scoped repository")
    r = api("POST", "/api/v1/repositories", USER_ALICE, {
        "name": f"alice-private-repo-{RUN_SUFFIX}",
        "kind": "template",
        "scope": "user",
    })
    if r.status_code == 201:
        repo = r.json()
        repos["user"] = repo["id"]
        register_cleanup("DELETE", f"/api/v1/repositories/{repo['id']}", USER_ALICE)
        if repo.get("scope") == "user":
            ok(f"AUTH-22: User repo created (id={repo['id']})")
        else:
            fail(f"AUTH-22: Unexpected scope: {repo.get('scope')}")
    else:
        fail(f"AUTH-22: Create user repo failed: {r.status_code} {r.text[:200]}")

    # AUTH-22.1: Creator can see user repository
    if "user" in repos:
        print_step("AUTH-22.1: Creator can see user repository")
        r = api("GET", "/api/v1/repositories", USER_ALICE, team_ids=[team_id])
        repo_list = r.json() if r.status_code == 200 else []
        if any(rp.get("id") == repos["user"] for rp in repo_list):
            ok("AUTH-22.1: Alice can see her user repo")
        else:
            fail("AUTH-22.1: Alice cannot see her user repo")

    # AUTH-22.2: Other user cannot see user repository
    if "user" in repos:
        print_step("AUTH-22.2: Other user cannot see user repository")
        r = api("GET", "/api/v1/repositories", USER_BOB)
        repo_list = r.json() if r.status_code == 200 else []
        if not any(rp.get("id") == repos["user"] for rp in repo_list):
            ok("AUTH-22.2: Bob cannot see Alice's user repo")
        else:
            fail("AUTH-22.2: Bob can see Alice's user repo (should not)")

    return repos


# ====================================================================
# Chain 2 (Access Control): Task Scope Inheritance
#   AUTH-23 through AUTH-23.3
# ====================================================================

def chain_task_scope(team_id: str, team_repo_id: str) -> None:
    print_header("Chain 2 (Access Control): Task Scope Inheritance")

    # AUTH-23: Create task under team-scoped repository
    print_step("AUTH-23: Create task under team-scoped repository")
    r = api("POST", "/api/v1/tasks", USER_ALICE, {
        "repositoryId": team_repo_id,
        "name": "team-scope-test-task",
    }, team_ids=[team_id])

    if r.status_code != 200:
        fail(f"AUTH-23: Create task failed: {r.status_code} {r.text[:200]}")
        return

    task = r.json()
    task_id = task.get("taskId") or task.get("id")
    if not task_id:
        fail(f"AUTH-23: No task ID in response: {task}")
        return
    ok(f"AUTH-23: Task created (id={task_id})")
    register_cleanup("DELETE", f"/api/v1/tasks/{task_id}", USER_ALICE, [team_id])

    # AUTH-23.1: Task inherits team scope from repository
    print_step("AUTH-23.1: Task inherits team scope")
    r = api("GET", f"/api/v1/tasks/{task_id}", USER_ALICE, team_ids=[team_id])
    if r.status_code == 200:
        t = r.json()
        if t.get("scope") == "team" and t.get("teamId") == team_id:
            ok("AUTH-23.1: Task scope=team, teamId matches repo")
        else:
            fail(f"AUTH-23.1: Unexpected scope={t.get('scope')}, teamId={t.get('teamId')}")
    else:
        fail(f"AUTH-23.1: Get task failed: {r.status_code}")

    # AUTH-23.2: Team member can see the task
    print_step("AUTH-23.2: Team member can see the task")
    # Add Bob to team first
    api("POST", f"/api/v1/teams/{team_id}/members", USER_ALICE, {"userId": "bob"})
    r = api("GET", "/api/v1/tasks", USER_BOB, team_ids=[team_id])
    if r.status_code == 200:
        tasks_data = r.json()
        task_list = tasks_data.get("items", tasks_data.get("tasks", tasks_data)) if isinstance(tasks_data, dict) else tasks_data
        if any(t.get("taskId") == task_id or t.get("id") == task_id for t in task_list):
            ok("AUTH-23.2: Bob (team member) can see team task")
        else:
            fail(f"AUTH-23.2: Bob cannot see team task (got {len(task_list)} tasks)")
    else:
        fail(f"AUTH-23.2: List tasks failed: {r.status_code}")

    # AUTH-23.3: Non-member cannot see the task
    print_step("AUTH-23.3: Non-member cannot see the task")
    # Remove Bob from team
    api("DELETE", f"/api/v1/teams/{team_id}/members/bob", USER_ALICE)
    r = api("GET", "/api/v1/tasks", USER_BOB)
    if r.status_code == 200:
        tasks_data = r.json()
        task_list = tasks_data.get("items", tasks_data.get("tasks", tasks_data)) if isinstance(tasks_data, dict) else tasks_data
        if not any(t.get("taskId") == task_id or t.get("id") == task_id for t in task_list):
            ok("AUTH-23.3: Bob (non-member) cannot see team task")
        else:
            fail("AUTH-23.3: Bob can see team task (should not)")
    else:
        fail(f"AUTH-23.3: List tasks failed: {r.status_code}")


# ====================================================================
# Chain 3 (Access Control): Admin Bypass
#   AUTH-24, AUTH-24.1
# ====================================================================

def chain_admin_bypass(user_repo_id: str, team_repo_id: str) -> None:
    print_header("Chain 3 (Access Control): Admin Bypass")

    # AUTH-24: Admin can see all scoped resources
    print_step("AUTH-24: Admin can see all scoped resources")
    r = api("GET", "/api/v1/repositories", ADMIN_CAROL)
    if r.status_code == 200:
        repo_list = r.json()
        has_user = any(rp.get("id") == user_repo_id for rp in repo_list)
        has_team = any(rp.get("id") == team_repo_id for rp in repo_list)
        if has_user and has_team:
            ok("AUTH-24: Admin sees both user-scoped and team-scoped repos")
        else:
            fail(f"AUTH-24: Admin missing repos (user={has_user}, team={has_team}, total={len(repo_list)})")
    else:
        fail(f"AUTH-24: List repos failed: {r.status_code}")

    # AUTH-24.1: Admin can fetch user-scoped repo by ID
    if user_repo_id:
        print_step("AUTH-24.1: Admin can fetch user-scoped repo by ID")
        r = api("GET", f"/api/v1/repositories/{user_repo_id}", ADMIN_CAROL)
        if r.status_code == 200:
            ok("AUTH-24.1: Admin can fetch Alice's user-scoped repo")
        else:
            fail(f"AUTH-24.1: Admin cannot fetch user-scoped repo: {r.status_code}")


# ====================================================================
# Main
# ====================================================================

def main() -> int:
    print_header("ACCESS CONTROL SMOKE TEST")

    # Prerequisites
    print_step("Checking backend health...")
    if not check_backend():
        fail("Backend not healthy")
        info("Run: ./docker-compose/dev-docker.sh")
        return 1
    ok("Backend healthy")

    print_step("Seeding test users (alice, bob, carol)...")
    if setup_users():
        ok("Test users created")
    else:
        fail("Could not create test users")
        return 1

    # ---- Team CRUD (AUTH-01 through AUTH-03) ----
    team_id = chain_team_crud()
    if not team_id:
        fail("Cannot continue without team — aborting")
        return 1

    # ---- Team Membership (AUTH-05 through AUTH-07.1) ----
    chain_team_membership(team_id)

    # ---- Team Credentials (AUTH-08 through AUTH-08.2.1) ----
    chain_team_credentials(team_id)

    # ---- Standalone: duplicate slug, non-member credentials ----
    standalone_duplicate_slug(team_id)
    standalone_non_member_credentials(team_id)

    # ---- Repository Scope Filtering (AUTH-20 through AUTH-22.2) ----
    repos = chain_repo_scope(team_id)

    # ---- Task Scope Inheritance (AUTH-23 through AUTH-23.3) ----
    if repos.get("team"):
        chain_task_scope(team_id, repos["team"])
    else:
        skip("AUTH-23 chain: no team repo available")

    # ---- Admin Bypass (AUTH-24, AUTH-24.1) ----
    if repos.get("user") and repos.get("team"):
        chain_admin_bypass(repos["user"], repos["team"])
    else:
        skip("AUTH-24 chain: missing repos")

    # ---- Cleanup created resources BEFORE deleting team (FK constraints) ----
    run_cleanup()

    # ---- Team Delete (AUTH-04, AUTH-04.1) — run after dependents removed ----
    chain_team_crud_delete(team_id)

    # ---- Summary ----
    print_header("TEST SUMMARY")
    print(f"  {GREEN}Passed: {tests_passed}{NC}")
    print(f"  {RED}Failed: {tests_failed}{NC}")
    if tests_skipped:
        print(f"  {YELLOW}Skipped: {tests_skipped}{NC}")
    print(f"  Total:  {tests_passed + tests_failed + tests_skipped}")
    print()

    if tests_failed == 0:
        print(f"{GREEN}{'=' * 65}{NC}")
        print(f"{GREEN}  ALL TESTS PASSED!{NC}")
        print(f"{GREEN}{'=' * 65}{NC}")
        return 0
    else:
        print(f"{RED}{'=' * 65}{NC}")
        print(f"{RED}  {tests_failed} TEST(S) FAILED{NC}")
        print(f"{RED}{'=' * 65}{NC}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
