# Git Worktree Operations - LLM Instructions

## TL;DR - Safest Git Workflow for LLMs

**For commits:**
```bash
git status
git add .
git commit -m "feat: description"  # ≤100 chars
```

**For push to main (safest approach):**
```bash
git status
git fetch origin main
git merge origin/main --no-edit  # Safest - preserves all changes
git push origin HEAD:main
```

**If push fails:** Create PR instead of force pushing
```bash
git push -u origin feature/branch-name
gh pr create --title "feat: description" --body "Description"
```

---

## When User Requests to Commit Changes

Always follow this sequence:

1. **Check current status first:**
   ```bash
   git status
   ```

2. **Stage and commit changes:**
   ```bash
   git add .
   git commit -m "feat: [user's description]"  # Keep ≤100 chars
   ```

3. **Use conventional commit format:**
   - `feat:` for new features
   - `fix:` for bug fixes  
   - `docs:` for documentation
   - `refactor:` for code refactoring
   - `test:` for adding tests
   - `chore:` for maintenance
   
   **Important:** Commit message header must be ≤100 characters due to commitlint rules.

## When User Requests to Push/Merge to Main

### Recommended Workflow: Branch First, Then Main

**Always follow this sequence:**

1. **Ensure clean working directory:**
   ```bash
   git status
   ```

2. **Handle any unstaged changes:**
   - For generated files: `git checkout -- *.gen.ts`
   - For work-in-progress: `git stash push -u -m "WIP"`
   - For important changes: `git add . && git commit -m "description"`

3. **Create feature branch if not already on one:**
   ```bash
   # Check current branch
   git branch --show-current
   
   # If on workspace branch, create descriptive feature branch
   git checkout -b feature/descriptive-name
   ```

4. **Update and integrate safely (choose safest option):**
   ```bash
   git fetch origin main
   
   # Option A: Try rebase first (only if it can fast-forward)
   git rebase origin/main
   
   # If rebase fails with conflicts, abort and use merge instead
   if [ $? -ne 0 ]; then
     git rebase --abort
     git merge origin/main --no-edit
   fi
   ```
   
   **Alternative simple approach for LLMs:**
   ```bash
   # Safest universal approach - always preserves changes
   git fetch origin main
   git merge origin/main --no-edit
   ```

5. **FIRST: Push to feature branch (backup & safety):**
   ```bash
   git push -u origin feature/descriptive-name
   ```

6. **THEN: Attempt push to main:**
   ```bash
   git push origin HEAD:main
   ```

### When Push to Main Fails

**If step 6 fails, DO NOT use `--force`. Instead:**

**Option A: Create Pull Request (Recommended)**
```bash
# Feature branch is already pushed, so create PR
gh pr create --title "feat: your feature" --body "Description of changes"
```
Tell user: "Push to main failed due to [reason]. Created PR instead - this maintains code quality and allows proper review/CI checks."

**Option B: Ask User for Guidance**
```bash
# Explain the situation to user
```
Tell user: "Push to main failed due to [specific reason: hooks/conflicts/divergent history]. Options:
1. **Create PR** to resolve issues properly (recommended)
2. **Force push** (bypasses safety checks - your choice)  
3. **Fix issues** locally and retry
4. **Investigate** the specific failure first

What would you like me to do?"

**Option C: Emergency Force Push (Only if user explicitly chooses)**
```bash
git push --force origin HEAD:main
```
⚠️ **Always warn user:** "Force pushing bypasses safety checks (tests, linting, conflict detection). This can break CI/CD and affect other developers. Proceeding as requested."

## Key: When You DON'T Need Force Push or PR

**Normal `git push` works without `--force` when:**

1. **Fast-Forward Merge Possible:**
   - Your commits are ahead of main
   - No divergent history exists
   - Main hasn't moved since you branched

2. **Clean Integration (Rebase OR Merge):**
   - Either `git rebase origin/main` succeeds without conflicts (creates linear history)
   - Or `git merge origin/main` succeeds (preserves all changes, creates merge commit)

3. **All Checks Pass:**
   - Pre-push hooks succeed (tests, linting, type checking)
   - No permission issues exist

**The safest approach:** 
- **Try rebase first** (cleaner history) - if it can fast-forward
- **Fall back to merge** (safer, preserves all changes) - if rebase has conflicts
- **Always use merge** if you want to guarantee no changes are lost

**Example timeline with rebase (when possible):**
```
Before rebase:
main:    A---B---C
feature:     D---E (branched from B)

After rebase:
main:    A---B---C
feature:         D'---E' (rebased onto C)

After push:
main:    A---B---C---D'---E' (fast-forward)
```

**Example timeline with merge (when rebase conflicts):**
```
Before merge:
main:    A---B---C---F
feature:     D---E (branched from B)

After merge:
main:    A---B---C---F
feature:     D---E---M (M is merge commit)

After push:
main:    A---B---C---F---D---E---M (includes all changes)
```

### Alternative: Pull Request Workflow (When Checks Fail)

If pre-push hooks or tests fail:

1. **Push to feature branch:**
   ```bash
   git push -u origin feature/descriptive-name
   ```

2. **Create pull request:**
   ```bash
   # Using GitHub CLI
   gh pr create --title "feat: your feature" --body "Description"
   
   # Or direct user to the URL provided by git push
   ```

3. **Fix issues in separate commits:**
   ```bash
   # Fix the issues
   git add .
   git commit -m "fix: resolve test failures"
   git push
   ```

4. **Merge PR when green:**
   - Via GitHub UI after all checks pass
   - Or via CLI: `gh pr merge --merge`

## When Integration Encounters Issues

### For "unstaged changes" errors:
```bash
# Check what's blocking
git status --porcelain

# Discard generated files
git checkout -- *.gen.ts

# Stash other changes
git stash push -u -m "Auto-stash before integration"

# Try rebase first, fall back to merge
git rebase origin/main || {
  git rebase --abort
  git merge origin/main --no-edit
}

# Restore stashed changes after success
git stash pop
```

### For merge conflicts during rebase:
```bash
# Option 1: Fix conflicts and continue rebase
git add .
git rebase --continue

# Option 2: Abort rebase and use merge instead (safer)
git rebase --abort
git merge origin/main --no-edit
# Then resolve merge conflicts normally
git add .
git commit --no-edit
```

### For merge conflicts during merge:
```bash
# Resolve conflicts in editor, then:
git add .
git commit --no-edit  # Complete the merge
```

### **Safest approach when conflicts occur:**
```bash
# Always abort rebase and use merge if conflicts arise
git rebase --abort
git merge origin/main --no-edit
# This preserves all changes and is much safer
```

## When Push Fails (Legacy Reference)

### Pre-push hooks failing:

**NEW APPROACH: Follow the "Branch First, Then Main" workflow above.**

**Legacy approaches for reference:**

~~**Recommended approach:**~~
```bash
# OLD: Don't do this anymore
git push -u origin feature/your-feature-name
gh pr create --title "feat: your feature" --body "WIP: Fixing test failures"
```

**Emergency bypass (absolute last resort):**
```bash
git push origin HEAD:main --no-verify
```
⚠️ **Only use if user explicitly chooses this option** after being presented with alternatives. Always explain: "Bypassed safety checks. Tests/linting issues remain and must be fixed immediately to maintain CI/CD pipeline health."

### Permission issues:
Suggest creating a pull request instead:
```bash
git push -u origin branch-name
# Then guide user to create PR
```

## Error Handling Responses

**"Cannot rebase: You have unstaged changes"**
→ Execute stashing workflow above

**"fatal: 'main' is already checked out"** 
→ This is normal in worktrees, continue with push

**Generated files keep changing**
→ Always run `git checkout -- *.gen.ts` before operations

**Hook failures**
→ Use `--no-verify` and remind user to fix issues

**"header must not be longer than 100 characters"**
→ Shorten commit message and retry commit

## Best Practices to Avoid Force Pushing

1. **Always work on feature branches** when implementing new features
2. **Run `pnpm check:turbo` locally** before attempting to push to main
3. **Use pull requests** when tests fail - this maintains CI/CD integrity
4. **Fix issues in separate commits** rather than amending and force pushing
5. **Only bypass hooks** when explicitly requested and document the technical debt
6. **Prefer merge over rebase** when in doubt - it's safer and preserves all changes
7. **Always create backup branches** before attempting risky operations
8. **Use `--no-edit` with merge** to avoid unnecessary editor prompts

## Pre-Operation Checklist

Before any git operation, always:
1. Run `git status` and report current state
2. Identify and handle generated files
3. Ensure user's work is safely committed or stashed
4. Fetch latest changes before rebasing
5. Run local checks (`pnpm check:turbo`) before pushing
6. Provide clear feedback on each step's success/failure

## Emergency Recovery

If something goes wrong:
```bash
# Reset to last commit
git reset --hard HEAD~1

# Reset to remote main
git reset --hard origin/main

# Check reflog for recovery
git reflog
```

Always explain to user what went wrong and what you're doing to fix it. 