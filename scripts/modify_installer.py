#!/usr/bin/env python3
"""
Script to modify the r2x-installer.sh script by inserting logic to copy contents of python-shim/{target} to install dir root.
Usage: python modify_installer.py /path/to/r2x-installer.sh
"""

import os
import sys


def modify_installer_script(script_path):
    if not os.path.isfile(script_path):
        print(f"Error: File '{script_path}' not found.")
        return False

    # Code to insert: Copy from python-shim/{target}/* to install dir root
    insert_code = (
        "    # Link contents of python-shim/{target} to install dir root\n"
        '    if [ -d "$_src_dir/python-shim/$_arch" ]; then\n'
        '        cp "$_src_dir/python-shim/$_arch"/* "$_install_dir/"\n'
        '        say "  python-shim/{target} contents (copied to root)"\n'
        "    fi\n"
    )

    # Read the file
    with open(script_path, "r") as f:
        content = f.read()

    # Find the insertion point: after "say \"everything's installed!\""
    target_line = 'say "everything\'s installed!"'
    if target_line not in content:
        print(
            f"Error: Could not find '{target_line}' in the script. Make sure it's the correct file."
        )
        return False

    # Replace the target line with the line + insert code
    new_content = content.replace(target_line, target_line + "\n" + insert_code)

    # Write back
    with open(script_path, "w") as f:
        f.write(new_content)

    print(f"Successfully modified '{script_path}' to include python-shim copy logic.")
    return True


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python modify_installer.py /path/to/r2x-installer.sh")
        sys.exit(1)

    script_path = sys.argv[1]
    success = modify_installer_script(script_path)
    sys.exit(0 if success else 1)
