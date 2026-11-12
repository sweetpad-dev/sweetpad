# Synthetic Apple archive vectors

These fixtures contain generated text, fake version metadata, a hard link, an empty file, and a symbolic link.
They contain no Apple SDK files, signing identities, or account credentials.

`generate_xcode_archive_fixtures.py` runs Apple's `aa` and `aea` tools on macOS.
Regeneration also requires Go 1.27 or later for the independent FCS envelope sender.
The shared-cluster fixtures also require a host C compiler and the macOS AppleArchive headers.
The XIP vectors use synthetic XAR/CPIO metadata and Apple's LZRAVEN encoder.
The symmetric fixture key is the public test sequence `bytes(range(32))`.
Signing private keys are temporary and are not retained.

The fixtures cover all supported Apple Archive compression modes, signed and symmetric AEA profiles, ODC/newc CPIO, and hard-link data outside the SDK selection.

`rfc9180-p256.json` contains published, non-production test material from the [RFC 9180 test vectors](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/5f503c564da00b0687b3de75f1dfbdfc4079ad31/test-vectors.json).
The FCS fixture uses that known test recipient and Go's standard HPKE sender.
Tests mock the Apple key download; they do not request a live key or use an Apple account.

`generate-clusters.c` calls Apple's AppleArchive API with `AA_FLAG_ARCHIVE_DEDUPLICATE_DAT`.
The cluster fixtures include hard links, identical contents with different permissions, and empty shared files.
Their only shared payload can occur outside the selected SDK paths.
Run `generate_xcode_archive_fixtures.py --clusters-only` to regenerate these fixtures without replacing the other vectors.
