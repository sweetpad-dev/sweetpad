"""Regenerate the momc stand-in's expected output from Xcode's own momc.

For each data model here, runs `xcrun momc` the ways Swift Build does: a
compile into a folder, and the code generation with `--swift-version`, with
and without `--dry-run`. What each writes goes under expected/<model>/: the
compiled files in out/ (without the .omo file momc adds for the current one
of several versions, which Core Data only reads when it's there), what the
compile prints in compile.log with this folder written as $(DIR), the listed
files in generate.dry-run and the generated ones in generated/, with the
line that names who made them and when written as Xcode's template has it.
Run it on a Mac after an Xcode update and review the diff:

    python3 -I sweetpad-cli/fixtures/momc/regen.py
"""

import glob
import os
import re
import shutil
import subprocess
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
EXPECTED = os.path.join(HERE, "expected")
CREATED = re.compile(r"^//  Created by .* on .*\.$", re.M)


def momc(args):
    sdk = subprocess.run(
        ["xcrun", "--sdk", "iphonesimulator", "--show-sdk-path"], capture_output=True, text=True, check=True
    ).stdout.strip()
    flags = ["--sdkroot", sdk, "--iphonesimulator-deployment-target", "17.0", "--module", "Fixture"]
    result = subprocess.run(["xcrun", "momc", *flags, *args], capture_output=True, text=True)
    return result.stdout, result.stderr


def save(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb" if isinstance(data, bytes) else "w") as f:
        f.write(data)


def main():
    shutil.rmtree(EXPECTED, ignore_errors=True)
    models = sorted(glob.glob(os.path.join(HERE, "*.xcdatamodel")) + glob.glob(os.path.join(HERE, "*.xcdatamodeld")))
    for model in models:
        name = os.path.basename(model)
        out = os.path.join(EXPECTED, name)
        with tempfile.TemporaryDirectory() as tmp:
            compiled = os.path.join(tmp, "out")
            os.makedirs(compiled)
            _, stderr = momc([model, compiled + "/"])
            save(os.path.join(out, "compile.log"), stderr.replace(HERE, "$(DIR)"))
            for root, _, files in os.walk(compiled):
                for file in files:
                    if file.endswith(".omo"):
                        continue
                    source = os.path.join(root, file)
                    with open(source, "rb") as f:
                        save(os.path.join(out, "out", os.path.relpath(source, compiled)), f.read())
            generated = os.path.join(tmp, "generated")
            listed, _ = momc(["--action", "generate", "--swift-version", "5.0", "--dry-run", model, generated])
            paths = [os.path.relpath(p, generated) for p in listed.splitlines() if p.strip()]
            save(os.path.join(out, "generate.dry-run"), "".join(p + "\n" for p in paths))
            momc(["--action", "generate", "--swift-version", "5.0", model, generated])
            for path in paths:
                with open(os.path.join(generated, path)) as f:
                    text = CREATED.sub("//  Created by $(AUTHOR) on $(DATE).", f.read())
                save(os.path.join(out, "generated", path), text)


if __name__ == "__main__":
    main()
