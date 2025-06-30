# Git Worktree Rebase and Merge Guide

This guide explains how to properly rebase and merge changes from a git worktree back to the main branch.

## Prerequisites

- Git worktree set up (e.g., `moatless-swarm-ui-workspace-*`)
- Changes committed in your worktree branch
- Access to push to the main repository

## Step-by-Step Process

### 1. Save Your Work

First, ensure all your changes are committed in the worktree:

```bash
# Check current status
git status

# Stage and commit any changes
git add .
git commit -m "your commit message"
```

### 2. Handle Unstaged Changes

If you have unstaged changes that you don't want to commit yet:

```bash
# Option A: Stash everything including untracked files
git stash push -u -m "Work in progress before rebase"

# Option B: Only stash tracked files
git stash push -m "WIP before rebase"
```

### 3. Fetch Latest Changes

Always fetch the latest changes from the remote repository:

```bash
git fetch origin main
```

### 4. Rebase onto Main

Rebase your worktree branch onto the latest main:

```bash
git rebase origin/main
```

### 5. Handle Rebase Issues

If you encounter "unstaged changes" errors during rebase:

```bash
# Check what's preventing the rebase
git status --porcelain

# For generated files (like routeTree.gen.ts), discard changes
git checkout -- path/to/generated/file

# For other files, either commit or stash them
git stash push -u -m "Stashing all changes"

# Retry the rebase
git rebase origin/main
```

### 6. Push Changes to Main

Once successfully rebased, you have several options:

#### Option A: Direct Push to Main (if you have permissions)
```bash
git push origin HEAD:main
```

#### Option B: Push with Pre-push Hook Bypass (if hooks are failing)
```bash
git push origin HEAD:main --no-verify
```

#### Option C: Create a Pull Request
```bash
# Push your branch first
git push -u origin your-branch-name

# Then create PR using GitHub CLI
gh pr create --title "Your PR title" --body "PR description"
```

### 7. Restore Stashed Changes (if any)

After successfully pushing:

```bash
# List all stashes
git stash list

# Apply the most recent stash
git stash pop

# Or apply a specific stash
git stash apply stash@{0}
```

## Common Issues and Solutions

### Issue: "Cannot rebase: You have unstaged changes"

**Solution:**
```bash
# See exactly what's changed
git status --porcelain

# For each file, decide whether to:
# 1. Commit it
git add file && git commit -m "Save changes"

# 2. Stash it
git stash push -u

# 3. Discard it (careful!)
git checkout -- file
```

### Issue: "fatal: 'main' is already checked out"

**Solution:** This happens when the main repository has main checked out. Just push directly:
```bash
git push origin HEAD:main
```

### Issue: Pre-push hooks failing

**Solution:** If the hooks are failing due to TypeScript errors or tests:
```bash
# Bypass hooks temporarily
git push origin HEAD:main --no-verify

# Note: Fix the errors in a follow-up commit
```

### Issue: Generated files keep changing

**Solution:** Generated files like `routeTree.gen.ts` often change automatically:
```bash
# Always discard changes to generated files before rebasing
git checkout -- client/src/routeTree.gen.ts
git checkout -- "*.gen.ts"  # All generated TypeScript files
```

## Best Practices

1. **Commit Often**: Make small, focused commits in your worktree
2. **Pull Regularly**: Keep your worktree up to date with main
3. **Test Before Push**: Run `pnpm type:check` and `pnpm biome:check` before pushing
4. **Clean Working Directory**: Resolve all changes before rebasing
5. **Document Changes**: Write clear commit messages

## Quick Command Reference

```bash
# Full rebase and push workflow
git status                          # Check current state
git add .                          # Stage changes
git commit -m "feat: description"  # Commit changes
git fetch origin main              # Get latest main
git rebase origin/main             # Rebase onto main
git push origin HEAD:main          # Push to main

# With stashing workflow
git stash push -u -m "WIP"         # Stash everything
git fetch origin main              # Get latest
git rebase origin/main             # Rebase
git push origin HEAD:main          # Push
git stash pop                      # Restore stashed work
```

## Verifying Success

After pushing, verify your changes:

1. Check GitHub/GitLab to see your commit on main
2. In the main repository (not worktree), run:
   ```bash
   cd /path/to/main/repo
   git pull origin main
   git log --oneline -5
   ```

Your commits should now appear in the main repository's history.