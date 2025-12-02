import glob
import hashlib
import json
import os
import sys

manifest_path = sys.argv[1]

hashes = {}

old_checksums = {}

# Update .sha256 files
for pattern in ["target/distrib/r2x-*.tar.xz", "target/distrib/r2x-*.zip"]:
    for file in glob.glob(pattern):
        print(f"Updating checksum for {file}")
        with open(file, "rb") as f:
            h = hashlib.sha256(f.read()).hexdigest()
        hashes[os.path.basename(file)] = h
        with open(file + ".sha256", "w") as f:
            f.write(f"{h}  {file}\n")

# Update manifest
if os.path.exists(manifest_path):
    with open(manifest_path, "r") as f:
        data = json.load(f)
    # Collect old checksums before updating
    for release in data.get("releases", []):
        for art in release.get("artifacts", []):
            if isinstance(art, str) and art in data.get("artifacts", {}):
                old_checksums[art] = data["artifacts"][art]["checksums"]["sha256"]
    for release in data.get("releases", []):
        for art in release.get("artifacts", []):
            if isinstance(art, str) and art in hashes:
                data["artifacts"][art]["checksums"]["sha256"] = hashes[art]
    with open(manifest_path, "w") as f:
        json.dump(data, f, indent=2)

# Update checksums in installer scripts

installer_scripts = [
    "target/distrib/r2x-installer.sh",
    "target/distrib/r2x-installer.ps1",
]
for script_path in installer_scripts:
    if os.path.exists(script_path):
        with open(script_path, "r") as f:
            content = f.read()
        for art, old_checksum in old_checksums.items():
            if art in hashes:
                new_checksum = hashes[art]
                content = content.replace(old_checksum, new_checksum)
        with open(script_path, "w") as f:
            f.write(content)
