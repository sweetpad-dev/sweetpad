---
sidebar_position: 14
---

# Git Worktrees

If you keep multiple branches of the same Xcode project checked out side by side via
[`git worktree`](https://git-scm.com/docs/git-worktree) — one for the feature you're working on, another for the
release branch you're stabilizing, a third for a hotfix — SweetPad can switch the build target between them without
opening another VSCode window.

## Why use worktrees with SweetPad

A few situations where this saves real time:

- You're debugging a flaky test on `main` while a feature branch is mid-build. With worktrees you keep both checked
  out; SweetPad's switcher flips between them.
- You need to compare behavior between `release/1.4` and `main`. Switch the worktree, hit ▶️, repeat — no
  `git checkout` dance, no DerivedData thrash from rewriting the same paths.
- You're reviewing a PR locally. Add a worktree for the PR branch (`git worktree add ../myapp-pr-123 pr-branch`),
  point SweetPad at it, run the app, then drop the worktree when you're done.

## Switch the active worktree

1. Set up your worktrees once with the usual git commands:

   ```bash
   git worktree add ../myapp-feature feature/new-onboarding
   git worktree add ../myapp-release release/1.4
   ```

   Each worktree should contain its own copy of the `.xcworkspace` / `.xcodeproj` / `Package.swift`.

2. From the root of *any* worktree, open the command palette and run:

   ```
   > SweetPad: Switch Git Worktree
   ```

3. Pick the worktree you want to build from. SweetPad finds the Xcode workspace inside it, updates
   `sweetpad.build.xcodeWorkspacePath` in your `.vscode/settings.json`, and refreshes the scheme list.

   A check ✓ marks the currently-active worktree, and each entry shows the branch name plus the absolute path.

## What changes when you switch

- `sweetpad.build.xcodeWorkspacePath` is rewritten to a path relative to the VSCode workspace root.
- The Build view re-reads schemes from the new workspace.
- Recent destinations and your selected scheme are preserved per-workspace, so switching back later returns you to
  where you left off.

Builds, runs, and tests after the switch use the new worktree. Nothing else (your open editor tabs, terminals, source
control state) is touched — VSCode keeps the same window.

## Limitations

- Each worktree needs a working Xcode workspace inside it — a bare `git worktree add` of a branch that has no Xcode
  project will be skipped.
- If two worktrees expose schemes with the same name, your scheme selection follows the workspace path, not the
  scheme name. After switching you may need to re-pick the scheme.
