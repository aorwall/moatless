---
description: Build, push, and deploy dev images to moatless K8s environment
allowed-tools: Bash, Read
---

# Deploy Dev Images to Kubernetes

Build Docker images locally, push to GHCR, deploy to the moatless environment, and verify the deployment.

## Arguments

- `$ARGUMENTS` - Optional flags: `--tag <name>`, `--app-only`, `--skip-build`

## Workflow

### Step 0: Pre-Deploy Checks

Before deploying, verify database migrations are properly generated:

```bash
cd apps/backend

# Check if there are uncommitted schema changes that need migrations
# This compares schema files against generated migrations
ls -la drizzle-postgres/meta/

# Verify each SQL file has a corresponding journal entry
# Journal should have entries matching all SQL files (0000, 0001, etc.)
cat drizzle-postgres/meta/_journal.json
```

**Migration Checklist:**
- Every `.sql` file in `drizzle-postgres/` should have a matching entry in `_journal.json`
- If schema files changed, run `bun run db:generate:pg` to generate migrations
- NEVER manually create SQL files - always use `drizzle-kit generate`

If migrations are out of sync, the app will crash on startup with errors like:
- `column "X" already exists` (migration re-running)
- `column "X" does not exist` (migration not applied)

### Step 1: Run Deploy Script

Execute the deploy script with any provided arguments:

```bash
./scripts/deploy-dev.sh $ARGUMENTS
```

If no `--tag` is provided, it will auto-generate one using git SHA and timestamp.

### Step 2: Verify Deployment

After deployment, verify the pod is running with the correct image:

```bash
# Check pod status
kubectl get pods -n vibe -l app.kubernetes.io/name=moatless-vibe

# Verify image tag
kubectl get deployment moatless-vibe -n vibe -o jsonpath='{.spec.template.spec.containers[0].image}'
```

### Step 3: Health Check

Verify the application is responding:

```bash
# Check frontend responds
curl -s -o /dev/null -w "%{http_code}" https://moatless.xyz/

# Check API responds (401 = auth working)
curl -s https://moatless.xyz/api/v1/feature-flags | head -c 100
```

### Step 4: Check Logs for Errors

Look for any startup errors:

```bash
kubectl logs deployment/moatless-vibe -n vibe --tail=20 | grep -E '"level":"(info|error)"'
```

## Success Criteria

Report deployment as successful when:
- Pod is `1/1 Running`
- Image tag matches the deployed tag
- Frontend returns HTTP 200
- No critical errors in logs (ignore ZodError for workspace snapshots - that's pre-existing data)

## Example Usage

```
/deploy-dev                        # Auto-generated tag
/deploy-dev --tag my-feature       # Custom tag
/deploy-dev --app-only             # Skip worker image
/deploy-dev --skip-build           # Deploy existing image only
```

## Troubleshooting

### Migration Errors on Startup

If pod crashes with migration errors:

1. **Check the drizzle tracking table** (uses `drizzle` schema, NOT `public`):
   ```bash
   # Get database URL
   kubectl get secret vibe-secrets -n vibe -o jsonpath='{.data.DATABASE_URL}' | base64 -d

   # Query tracking table (replace connection details)
   psql $DATABASE_URL -c "SELECT * FROM drizzle.__drizzle_migrations ORDER BY id;"
   ```

2. **Compare with journal** - entries should match by index (id 1 = idx 0, etc.)

3. **If migration needs to be manually tracked** (last resort):
   ```bash
   # Compute hash of SQL file
   cat drizzle-postgres/XXXX_migration.sql | shasum -a 256 | cut -d' ' -f1

   # Insert into tracking table
   psql $DATABASE_URL -c "INSERT INTO drizzle.__drizzle_migrations (id, hash, created_at) VALUES (N, 'HASH', TIMESTAMP);"
   ```

**Important:** Drizzle stores SHA256 hashes of SQL content, not migration names.
