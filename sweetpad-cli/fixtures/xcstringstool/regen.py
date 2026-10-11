"""Regenerate the xcstringstool stand-in's expected output from Xcode's own tool.

For each probe catalog here, runs `xcrun xcstringstool` the ways Swift Build
does: `compile` (and its `--dry-run`), `compile --force-build-all-strings`
for Probe, and `generate-symbols --language swift`. What each run writes goes
under expected/<catalog>/, and the errors it prints go in a `.errors` file
beside it, sorted, with the catalog's directory written as $(DIR). A compile
that fails keeps only its errors. Run it on a Mac after an Xcode update and
review the diff:

    python3 -I sweetpad-cli/fixtures/xcstringstool/regen.py
"""

import glob
import os
import shutil
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
EXPECTED = os.path.join(HERE, "expected")


def run(args, cwd):
    result = subprocess.run(["xcrun", "xcstringstool", *args], cwd=cwd, capture_output=True, text=True)
    return result.stdout, result.stderr


def errors(stderr, directory):
    lines = sorted(line.replace(directory, "$(DIR)") for line in stderr.splitlines() if line.strip())
    return "".join(line + "\n" for line in lines)


def save(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def main():
    shutil.rmtree(EXPECTED, ignore_errors=True)
    for catalog in sorted(glob.glob(os.path.join(HERE, "*.xcstrings"))):
        name = os.path.splitext(os.path.basename(catalog))[0]
        out = os.path.join(EXPECTED, name)
        with tempfile.TemporaryDirectory() as tmp:
            tmp = os.path.realpath(tmp)
            shutil.copy(catalog, tmp)
            source = os.path.join(tmp, os.path.basename(catalog))
            modes = [("compile", [])]
            if name == "Probe":
                modes.append(("forced", ["--force-build-all-strings"]))
            for mode, flags in modes:
                dest = os.path.join(tmp, mode)
                listed, _ = run(["compile", "--dry-run", *flags, "-o", dest, source], tmp)
                paths = sorted(os.path.relpath(p, dest) for p in listed.splitlines() if p.strip())
                save(os.path.join(out, f"{mode}.dry-run"), "".join(p + "\n" for p in paths))
                _, stderr = run(["compile", *flags, "-o", dest, source], tmp)
                save(os.path.join(out, f"{mode}.errors"), errors(stderr, tmp))
                # After an error, which languages xcstringstool still writes varies from run to run.
                if os.path.isdir(dest) and not stderr.strip():
                    shutil.copytree(dest, os.path.join(out, mode))
            symbols = os.path.join(tmp, "symbols")
            os.makedirs(symbols)
            _, stderr = run(["generate-symbols", "--language", "swift", "-o", symbols, source], tmp)
            save(os.path.join(out, "symbols.errors"), errors(stderr, tmp))
            for generated in os.listdir(symbols):
                shutil.copy(os.path.join(symbols, generated), out)


if __name__ == "__main__":
    main()
