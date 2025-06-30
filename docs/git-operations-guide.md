# Git Worktree Operations - LLM Instructions

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

Execute this workflow in order:

1. **Ensure clean working directory:**
   ```bash
   git status
   ```

2. **Handle any unstaged changes:**
   - For generated files: `git checkout -- *.gen.ts`
   - For work-in-progress: `git stash push -u -m "WIP"`
   - For important changes: `git add . && git commit -m "description"`

3. **Update and rebase:**
   ```bash
   git fetch origin main
   git rebase origin/main
   ```

4. **Push to main:**
   ```bash
   git push origin HEAD:main
   ```

## When Rebase Encounters Conflicts

### For "unstaged changes" errors:
```bash
# Check what's blocking
git status --porcelain

# Discard generated files
git checkout -- *.gen.ts

# Stash other changes
git stash push -u -m "Auto-stash before rebase"

# Retry rebase
git rebase origin/main

# Restore stashed changes after success
git stash pop
```

### For merge conflicts:
```bash
# Fix conflicts in editor, then:
git add .
git rebase --continue
```

### If rebase fails completely:
```bash
git rebase --abort
# Then investigate the issue manually
```

## When Push Fails

### Pre-push hooks failing:
```bash
git push origin HEAD:main --no-verify
```
Tell user: "Bypassed hooks due to errors. Please fix TypeScript/linting issues in next commit."

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

## Pre-Operation Checklist

Before any git operation, always:
1. Run `git status` and report current state
2. Identify and handle generated files
3. Ensure user's work is safely committed or stashed
4. Fetch latest changes before rebasing
5. Provide clear feedback on each step's success/failure

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