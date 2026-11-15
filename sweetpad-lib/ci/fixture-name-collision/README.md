Two projects in one workspace, each with a framework target named `Core`.
The shared `BCore` scheme builds B's `Core`, which defines `fromB`; A's
defines `fromA`. A build that picks targets by name alone builds A's.
