import glob
import hashlib
import json
import os
import sys

manifest_path = sys.argv[1]

# Update .sha256 files
for pattern in ["target/distrib/r2x-*.tar.xz", "target/distrib/r2x-*.zip"]:
    for file in glob.glob(pattern):
        print(f"Updating checksum for {file}")
        with open(file, "rb") as f:
            h = hashlib.sha256(f.read()).hexdigest()
        with open(file + ".sha256", "w") as f:
            f.write(f"{h}  {file}\n")

# Update manifest
if os.path.exists(manifest_path):
    with open(manifest_path, "r") as f:
        data = json.load(f)
    for release in data.get("releases", []):
        for art in release.get("artifacts", []):
            path = art.get("path")
            if path and os.path.exists(path):
                with open(path, "rb") as f:
                    art["checksums"]["sha256"] = hashlib.sha256(f.read()).hexdigest()
    with open(manifest_path, "w") as f:
        json.dump(data, f, indent=2)
