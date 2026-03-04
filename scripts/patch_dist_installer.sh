#!/usr/bin/env bash
set -euo pipefail

installer_path="${1:-}"

if [[ -z "${installer_path}" ]]; then
    echo "Usage: $0 <path-to-cargo-dist-installer.sh>"
    exit 1
fi

if [[ ! -f "${installer_path}" ]]; then
    echo "Installer not found: ${installer_path}"
    exit 1
fi

if grep -q "ensure_python_runtime_for_r2x()" "${installer_path}"; then
    echo "Installer already patched: ${installer_path}"
    exit 0
fi

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

awk '
BEGIN {
    inserted_function = 0
    inserted_call = 0
}
{
    if ($0 == "check_for_shadowed_bins() {") {
        print "ensure_python_runtime_for_r2x() {"
        print "    local _install_dir=\"$1\""
        print "    local _arch=\"$2\""
        print ""
        print "    if [ \"$APP_NAME\" != \"r2x\" ]; then"
        print "        return 0"
        print "    fi"
        print ""
        print "    case \"$_arch\" in"
        print "        *-unknown-linux-gnu)"
        print "            local _python_version=\"3.12\""
        print "            local _primary_lib=\"libpython3.12.so.1.0\""
        print "            local _secondary_lib=\"libpython3.12.so\""
        print "            ;;"
        print "        *-apple-darwin)"
        print "            local _python_version=\"3.12\""
        print "            local _primary_lib=\"libpython3.12.dylib\""
        print "            local _secondary_lib=\"\""
        print "            ;;"
        print "        *)"
        print "            return 0"
        print "            ;;"
        print "    esac"
        print ""
        print "    if \"$_install_dir/$APP_NAME\" --version >/dev/null 2>&1; then"
        print "        return 0"
        print "    fi"
        print ""
        print "    if ! command -v uv >/dev/null 2>&1; then"
        print "        say \"warning: $APP_NAME requires Python ${_python_version} shared libraries.\""
        print "        say \"Install uv and run: uv python install ${_python_version}\""
        print "        return 0"
        print "    fi"
        print ""
        print "    uv python install \"$_python_version\" >/dev/null 2>&1 || true"
        print "    local _python_bin"
        print "    _python_bin=\"$(uv python find \"$_python_version\" 2>/dev/null || true)\""
        print "    if [ -z \"$_python_bin\" ]; then"
        print "        say \"warning: unable to locate Python ${_python_version} via uv\""
        print "        return 0"
        print "    fi"
        print ""
        print "    local _python_prefix"
        print "    _python_prefix=\"$(dirname \"$(dirname \"$_python_bin\")\")\""
        print "    local _python_lib_dir=\"$_python_prefix/lib\""
        print ""
        print "    if [ ! -f \"$_python_lib_dir/$_primary_lib\" ]; then"
        print "        say \"warning: expected Python library not found at $_python_lib_dir/$_primary_lib\""
        print "        return 0"
        print "    fi"
        print ""
        print "    ensure cp -f \"$_python_lib_dir/$_primary_lib\" \"$_install_dir/$_primary_lib\""
        print "    ensure chmod +x \"$_install_dir/$_primary_lib\""
        print ""
        print "    if [ -n \"$_secondary_lib\" ] && [ -f \"$_python_lib_dir/$_secondary_lib\" ]; then"
        print "        ensure cp -f \"$_python_lib_dir/$_secondary_lib\" \"$_install_dir/$_secondary_lib\""
        print "        ensure chmod +x \"$_install_dir/$_secondary_lib\""
        print "    fi"
        print ""
        print "    if \"$_install_dir/$APP_NAME\" --version >/dev/null 2>&1; then"
        print "        say_verbose \"installed Python runtime libraries for $APP_NAME\""
        print "    fi"
        print "}"
        print ""
        inserted_function = 1
    }

    if ($0 ~ /^    say "everything/ && $0 ~ /installed!"$/) {
        print "    ensure_python_runtime_for_r2x \"$_install_dir\" \"$_arch\""
        inserted_call = 1
    }

    print $0
}
END {
    if (inserted_function != 1) {
        print "Failed to insert runtime helper function" > "/dev/stderr"
        exit 1
    }
    if (inserted_call != 1) {
        print "Failed to insert runtime helper call" > "/dev/stderr"
        exit 1
    }
}
' "${installer_path}" > "${tmp_file}"

mv "${tmp_file}" "${installer_path}"
chmod +x "${installer_path}"
echo "Patched installer: ${installer_path}"
