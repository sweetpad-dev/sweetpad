# Hybrid fixture dependency locks

These files supplement the source pins in `../linux-projects.json`.
They capture generated locks missing from the upstream pins.
The preparation recipe also validates tracked Bun and npm locks.

`linux-project-prepare.py` installs the frozen graphs on macOS.
Both build hosts then use the prepared source tree.
It records tool versions, source patches, and lock hashes in a receipt.

The Pod specification JSON files come from the pinned React Native and MMKV packages.
They retain their upstream dependency metadata and licenses.
`@SOURCE_ROOT@` replaces the captured checkout path.
The recipe restores that path before CocoaPods computes checksums.
It then uses deployment mode to validate the complete Pod graph.
The MMKV Pod lock also includes the explicitly recorded source patches.

Fresh Flutter, React Native, MMKV, and Capacitor builds pass on both hosts.
These locks improve repeatability; they do not preserve copies of external registries.
