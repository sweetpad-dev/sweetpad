# APT release signing

The public feed is https://sweetpad-dev.github.io/sweetpad/apt/.
It publishes amd64 and arm64 packages from stable CLI releases and keeps the ten most recent releases.
GitHub Pages is independent of the documentation website.

The `Distribute CLI` workflow imports `SWEETPAD_APT_SIGNING_KEY` into a temporary keyring.
It checks the fingerprint against `signing-key.fingerprint`, signs Release and InRelease metadata, and deletes the temporary keyring.
Only the APT job receives this secret.
The public key expires on 2028-11-10. Rotate or extend it before that date.

The private key backup is kept outside the source repository.
Never add a secret key or its revocation certificate to this directory.

To retry publication after a successful release, dispatch `Distribute CLI` from main with its `cli-vX.Y.Z` tag.
This downloads and checksums the existing release packages; it does not rebuild them.
Packslip uses GitHub OIDC signing, and GHCR uses the job's package token.
The APT job requires its dedicated GPG secret.

Before rotating keys, publish the new public key and fingerprint through the documentation and existing authenticated channels.
Update the Actions secret and committed public files together.
Users with `Signed-By` must install the new public key before the feed switches signers.
Keep old public keys available for verification of older metadata.

The feed deliberately has no `Valid-Until` field because it changes only when a CLI release ships.
APT authenticates its metadata and package hashes; HTTPS protects transport.
The local validation script checks normal upgrades and rejection of altered signed metadata.
