#!/usr/bin/env python3
"""
Script to modify the r2x installer scripts (shell or PowerShell) by inserting logic to copy contents of python-shim/{target} to install dir root.
Usage: python modify_installer.py /path/to/r2x-installer.sh or /path/to/r2x-installer.ps1
"""

import os
import sys


def modify_installer_script(script_path):
    if not os.path.isfile(script_path):
        print(f"Error: File '{script_path}' not found.")
        return False

    # Determine file type based on extension
    file_ext = os.path.splitext(script_path)[1].lower()

    if file_ext == ".sh":
        # Code to insert for shell script: Copy from python-shim/{target}/* to install dir root
        insert_code = (
            "    # Link contents of python-shim/{target} to install dir root\n"
            '    if [ -d "$_src_dir/python-shim/$_arch" ]; then\n'
            '        cp "$_src_dir/python-shim/$_arch"/* "$_install_dir/"\n'
            '        rm -rf "$_src_dir/python-shim"\n'
            "    fi\n"
        )
        target_line = 'say "everything\'s installed!"'
    elif file_ext == ".ps1":
        # Code to insert for PowerShell script
        insert_code = (
            "    # Copy contents of python-shim/{target} to install dir bin\n"
            "    $tmp_dir = Split-Path $bin_path\n"
            '    $shim_dir = "$tmp_dir\\python-shim\\$arch"\n'
            "    if (Test-Path $shim_dir) {\n"
            '        Copy-Item "$shim_dir\\*" -Destination "$dest_dir" -Recurse\n'
            '        Remove-Item "$tmp_dir\\python-shim" -Recurse -Force\n'
            "    }\n"
        )
        target_line = 'Write-Information "everything\'s installed!"'
    else:
        print(f"Error: Unsupported file type '{file_ext}'. Supported: .sh or .ps1")
        return False

    # Read the file
    with open(script_path, "r") as f:
        content = f.read()

    # Find the insertion point: after the target_line
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
        print(
            "Usage: python modify_installer.py /path/to/r2x-installer.sh or /path/to/r2x-installer.ps1"
        )
        sys.exit(1)

    script_path = sys.argv[1]
    success = modify_installer_script(script_path)
    sys.exit(0 if success else 1)
