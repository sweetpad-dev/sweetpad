---
sidebar_position: 18
sidebar_label: Agent skills
---

# Agent skills

SweetPad ships a set of **agent skills**: short instruction files that teach an
AI coding agent how to drive the `sweetpad` CLI on your behalf. With them
installed, your agent already knows to build with the right destination, read
diagnostics from a failed build without rebuilding, and inspect resolved build
settings, instead of you spelling out the commands every session.

The skills are vendor-neutral. The same files install into any agent that reads
the open skills format (Claude Code, Cursor, Codex, Copilot, Gemini, and
others) through one installer. There is no Claude-specific step.

:::tip

This page is for people who develop with an AI coding agent. If you drive
SweetPad by hand from the sidebar or the terminal, you can skip it.

:::

## What a skill is

A skill is a small markdown file with a one-line description of what it's for.
Your agent reads those descriptions up front, and when a request matches one, it
loads the fuller instructions on demand. So the skills cost almost nothing until
they're relevant, then they hand the agent a tested recipe rather than a guess.

SweetPad's skills wrap the CLI's machine-readable surface, the `-o json`
envelope and the specifier-based destination selection, so the agent reads
structured output instead of scraping human text.

## Prerequisites

The skills teach an agent to run `sweetpad`, so the CLI has to be installed and
on your `PATH` first:

```bash
brew install sweetpad-dev/tap/sweetpad
```

See [Get started with the CLI](./getting-started.md) if you haven't set it
up yet. You also need an AI coding agent that supports the open skills format.

## Install

Install every SweetPad skill into the agents on your machine with one command:

```bash
npx skills add sweetpad-dev/sweetpad
```

This uses the open [`skills`](https://github.com/vercel-labs/skills) installer.
It fetches SweetPad's skills straight from the repository, detects the agents you
have, and writes the skills into each one's own format.

To take every skill without the interactive picker, add `--skill '*' --yes`:

```bash
npx -y skills add sweetpad-dev/sweetpad --skill '*' --yes
```

### Target a single agent

Pass `--agent` to install into just one tool, for example Claude Code:

```bash
npx skills add sweetpad-dev/sweetpad --agent claude-code
```

Leave `--agent` off to install into every supported agent the installer finds.

## Using the skills

Once installed, your agent selects a skill on its own when your request matches
one. Asking it to "build and run this on the booted simulator", "figure out why
the build failed", or "show the resolved build settings" is enough to trigger
the matching skill. Most agents also let you invoke a skill by name from their
own slash-command menu; check your agent's documentation for how it lists them.

## Keeping them updated

The skills don't update themselves the way a Homebrew formula does. Re-run the
install command to pull the latest versions:

```bash
npx skills add sweetpad-dev/sweetpad
```

Running it again overwrites the installed copies with the current ones from the
repository.
