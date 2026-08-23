---
sidebar_position: 14
sidebar_label: Git merge drivers
---

# Git merge drivers

Two files in every Xcode repo conflict constantly and merge badly: `project.pbxproj` and
`Package.resolved`. Neither is meant to be read by a human, both record structure rather than lines,
and git's line-based merge has no idea which lines belong together. The usual outcomes are a conflict
that has to be resolved by hand in a file nobody understands, or a "successful" textual merge that
produces a project Xcode refuses to open.

SweetPad ships merge drivers that understand both formats. Two people adding different files to the
same target is not a conflict once you're merging objects instead of lines.

## Installing them

```bash
sweetpad merge install
```

That registers the drivers with git for the current repository: it defines them in your git config and
adds the matching entries to `.gitattributes`. From then on, git calls SweetPad instead of its own
text merge whenever one of those files conflicts, and most merges simply stop producing conflicts.

For every repository you work in:

```bash
sweetpad merge install --global
```

The global form uses your global git config and global attributes file, so a repo you clone tomorrow
is covered without another step.

:::warning

The two halves live in different places. `.gitattributes` is committed and reaches your whole team;
the driver definition is in git config and is local to whoever ran the command. A teammate who pulls
the `.gitattributes` but hasn't run `sweetpad merge install` names a driver git doesn't have, and git
silently falls back to its default text merge, which is the behavior they had before rather than an error.

So either have everyone run it once, or use `--global` yourself and tell the team it exists.

:::

## Resolving a conflict by hand

When a merge is already in progress and left conflicts, resolve them without the driver installed:

```bash
sweetpad merge run
```

With no arguments it finds every conflicted file of either kind and resolves them. Name paths to
narrow it:

```bash
sweetpad merge run MyApp.xcodeproj/project.pbxproj
```

Each file's kind is detected from the file itself, so you don't say which is which.

`--force` re-merges from `HEAD` and `MERGE_HEAD` even when git already merged the file textually,
which helps when git *thought* it succeeded and the result doesn't open.

## Checking the result

The merge is semantic, but the output is still a project file, so treat it the way you'd treat any
resolved conflict: open the project, or at least confirm it still builds.

```bash
sweetpad build
```

A `.pbxproj` that parses but has lost a file reference will build fine and fail at runtime, so a quick
`sweetpad project info` to confirm your targets still look right is worth the two seconds.

## The plumbing underneath

`sweetpad pbxproj resolve` is the same `.pbxproj` merge as a standalone command, in the
[plumbing namespace](./project.md#the-pbxproj-plumbing) with the rest of the low-level project
editing. `merge run` is the porcelain that covers both file kinds and finds the conflicts for you.
Prefer it unless you're scripting something specific.
