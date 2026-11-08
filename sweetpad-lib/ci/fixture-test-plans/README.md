# Automatic test-plan manifest fixture

`Probe.xctestplan` runs UI and logic tests in French and English configurations.
The tests assert environment values, quoted arguments, build-setting expansion, and UI interaction.
Selected and skipped identifiers exclude deliberately failing tests.

`Smoke.xctestplan` selects only the logic target and overrides the region without an explicit language.
Its region control expects an English host locale and validates `en_FR` launch arguments.
The Linux C/POSIX locale supplies English.

Regenerate the project with `xcodegen generate` from this directory.
Run the default-plan comparison from the repository root:

```bash
python3 -B sweetpad-lib/ci/linux-test-plan-checks.py \
  --out /path/to/fresh-report --simulator SIMULATOR_UDID
```

The comparison runs native and Linux products without editing their manifests.
It retains configuration results, selected manifest fields, and executable hashes.
Apple SDK products stay in the local report directory.
