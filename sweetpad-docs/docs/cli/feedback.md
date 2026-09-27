---
sidebar_position: 18
sidebar_label: Feedback reports
---

# Feedback reports

When an AI agent drives SweetPad for you and SweetPad itself goes wrong, the agent can offer to send
the maintainer a short report about it. This page covers when it offers one, what the report
contains, and how to turn reports off.

## When your agent offers one

The agent follows `sweetpad help feedback`. It offers a report when:

- SweetPad crashed, hung, or reported an internal error.
- Its output contradicted itself, or contradicted Xcode.
- A tip or command it suggested didn't work.
- Something was missing, and the agent had to fall back to raw xcodebuild, simctl or devicectl.
- Its help or docs disagreed with what it did.

It doesn't offer one for problems in your project or setup, such as compile errors, failing tests,
code signing, or a missing simulator runtime. It asks once per issue. If you say no, it doesn't ask
about that issue again.

## Two approvals

Nothing is sent until you have said yes twice:

1. The agent asks whether you'd like to send a report.
2. If you would, it writes the report to a file and runs `sweetpad feedback submit <file> --dry-run`.
   This sends nothing. It prints the exact payload and a digest, and the agent shows you that output.
3. If you approve what you saw, the agent runs `sweetpad feedback submit <file> --approve <digest>`.

The send goes ahead only if the payload still matches the digest you approved. If the file changed in
between, SweetPad sends nothing and asks for a new dry run. SweetPad never prompts you itself; the
questions come from your agent.

## What is sent

The report goes to the maintainer's Sentry project, the one the VS Code extension's error reports use,
as user feedback. It contains:

- The report the agent wrote: the kind of problem and its severity, what the agent was doing, the
  command it ran, what it expected, what happened, and what it thinks went wrong.
- The SweetPad version, the Xcode version, the macOS version, and your Mac's architecture.

## What isn't sent

SweetPad adds nothing that names you or your Mac: no user name, host name, file path, or device. The
report has no IP address, and it tells Sentry not to fill one in from the connection.

SweetPad doesn't edit the report's text. The agent is told to take out project, workspace, scheme,
target and package names, bundle and team ids, paths, device names and UDIDs, user and host names, email
addresses, private URLs, and tokens, and to put placeholders in their place. The dry run is where you
check that it did.

:::note

Like any server, Sentry sees the network address a report comes from. It doesn't store that address
with the report, but it does look up an approximate location (country, region and city) from it.

:::

## Sending through a proxy

If your network allows HTTPS only through a proxy, SweetPad reads the same variables curl does. Set
`HTTPS_PROXY` (or `https_proxy`) to the proxy, for example `http://proxy.example.com:3128`, or
`http://user:password@proxy.example.com:3128` when it needs credentials. SweetPad opens a tunnel
through the proxy, so the report stays encrypted between your Mac and Sentry. Hosts listed in
`NO_PROXY` (or `no_proxy`) skip the proxy.

SweetPad works with HTTP proxies only. A SOCKS proxy, or a proxy you reach over HTTPS, isn't
supported.

## Turning reports off

```bash
sweetpad feedback off
```

This writes `enabled = false` under `[feedback]` in `~/.config/sweetpad/config.toml`, and leaves the
rest of the file as it was, comments included. After that, `sweetpad feedback submit` refuses to send,
and `sweetpad help feedback` tells agents not to offer a report.

`sweetpad feedback on` turns reports back on, and `sweetpad feedback status` says which one is in
effect.
