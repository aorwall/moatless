# GitHub Channel Cutover — org webhook + ingress rules

This runbook walks through the cutover from per-repo `channel_configs` rows
to the new `provider_connections` + `ingress_rules` + `task_subscriptions`
model for GitHub PRs.

> Migration `0034_github_pr_channel_architecture.sql` is one-shot. Apply it
> manually via `psql` BEFORE deploying the new backend image. See the
> migration file's header comment for full context.

## 1. Generate a webhook secret

On the operator's machine:

```bash
openssl rand -hex 32 > /tmp/gh-org-webhook-secret
chmod 600 /tmp/gh-org-webhook-secret
```

## 2. Register the org webhook on `soaplabs`

```bash
gh api -X POST /orgs/soaplabs/hooks \
  -f name=web \
  -f active=true \
  -F 'events[]=pull_request' \
  -F 'events[]=issue_comment' \
  -F 'events[]=pull_request_review' \
  -F 'events[]=pull_request_review_comment' \
  -F 'events[]=check_run' \
  -F 'events[]=check_suite' \
  -f config[url]=https://moatless.example/api/v1/channels/webhooks/github \
  -f config[content_type]=json \
  -f config[secret]="$(cat /tmp/gh-org-webhook-secret)" \
  -f config[insecure_ssl]=0
```

Capture the resulting hook id from the response — needed for rollback.

## 3. Apply migration `0034`

The backend image is distroless, so run `psql` from a temporary pod:

```bash
kubectl -n moatless run pg-migrate --rm -it \
  --image=postgres:17-alpine \
  --restart=Never \
  -- psql "<DATABASE_URL>" \
       -v gh_org_webhook_secret="$(cat /tmp/gh-org-webhook-secret)" \
       -c "SET app.gh_org_webhook_secret = :'gh_org_webhook_secret';" \
       -f /path/to/backend/migrations/0034_github_pr_channel_architecture.sql
```

The `RAISE NOTICE` output must show counts roughly matching:

```
NOTICE:  GitHub channel migration: provider_connections=1, ingress_rules=6,
         task_subscriptions=N, leftover_configs=0, leftover_threads=0
```

`N` should be close to the pre-migration `channel_threads` count — currently
about 306. Threads with missing `owner` / `repo` / `pr_number` properties are
dropped (see the earlier `RAISE NOTICE` line). If the migration aborts with
`Expected exactly 1 github provider_connections row, got X` or
`Legacy github_pr rows still present`, abort the cutover and investigate.

## 4. Deploy the new backend image

Roll out the image that contains the migration, the runtime rewrite, and the
new HTTP/CLI surface. `sqlx::migrate` will see the schema is already in place
(idempotent `CREATE TABLE IF NOT EXISTS`) and the `DO` block skips itself when
the GUC is unset, so `0034` records cleanly in `_sqlx_migrations` without
re-running the backfill.

## 5. Smoke test

```bash
moat channels rules list --provider github     # expect 6 rules
moat channels subscriptions --task <recent-task-id>  # REASON / LAST_EVENT populated
```

Trigger a manual PR comment from a soaplabs repo and confirm the agent
receives it. Backend logs should show the org connection resolving and the
ingress rule routing the event to the right task.

## 6. Delete the 6 legacy per-repo webhooks

```bash
for repo in soap-auth soap-config soap-external-api soap-specs tm-api tm-interface; do
  gh api /repos/soaplabs/$repo/hooks --jq \
    '.[] | select(.config.url | contains("/api/v1/channels/webhooks/github")) | .id' \
    | xargs -I{} gh api -X DELETE /repos/soaplabs/$repo/hooks/{}
done

# Verify
for repo in soap-auth soap-config soap-external-api soap-specs tm-api tm-interface; do
  count=$(gh api /repos/soaplabs/$repo/hooks --jq 'length')
  echo "$repo: $count"
done
```

All counts should be `0`.

## 7. Shred the secret file

```bash
rm -P /tmp/gh-org-webhook-secret
```

## Rollback

```sql
DELETE FROM provider_connections WHERE scope_value = 'soaplabs';
-- This cascades through ingress_rules and task_subscriptions.
```

Then redeploy the previous backend image. Per-repo webhooks must be
re-registered manually — the original secrets are gone, so generate fresh
ones and update GitHub by hand. **This is effectively a one-way migration in
practice**; rolling back will lose the existing thread→subscription history.
