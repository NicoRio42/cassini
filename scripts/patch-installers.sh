#!/bin/sh
# Adds the removal of the Docker `cassini` shortcut (remove-docker-shortcut.sh
# and .ps1, next to this script) to the install scripts that dist generates,
# which have no hook for it:
#
#     scripts/patch-installers.sh <directory with cassini-installer.sh and .ps1>
#
# It inserts each function before the line that starts the installation, and
# its call after the line printing "everything's installed!". It fails if one
# of these lines is missing, for example after a dist upgrade changed them,
# or if the scripts are already patched.
set -eu

dir="$1"
here="$(cd "$(dirname "$0")" && pwd)"

# patch <installer> <function file> <line before which to insert the function>
#       <line after which to insert the call> <call>
patch() {
    installer="$dir/$1"
    for anchor in "$3" "$4"; do
        count="$(grep -cxF -- "$anchor" "$installer" || true)"
        if [ "$count" != 1 ]; then
            echo "$installer: expected one line '$anchor', found $count" >&2
            exit 1
        fi
    done
    if grep -qE "remove_docker_shortcut|Remove-DockerShortcut" "$installer"; then
        echo "$installer is already patched" >&2
        exit 1
    fi

    awk -v function_file="$2" -v function_anchor="$3" \
        -v call_anchor="$4" -v call="$5" '
        $0 == function_anchor {
            while ((getline line < function_file) > 0) print line
            print ""
        }
        { print }
        $0 == call_anchor { print call }
    ' "$installer" > "$installer.patched"
    mv "$installer.patched" "$installer"
    echo "patched $installer"
}

patch cassini-installer.sh "$here/remove-docker-shortcut.sh" \
    'download_binary_and_run_installer "$@" || exit 1' \
    '    say "everything'"'"'s installed!"' \
    '    remove_docker_shortcut "$NO_MODIFY_PATH"'

patch cassini-installer.ps1 "$here/remove-docker-shortcut.ps1" \
    '# PSScriptAnalyzer doesn'"'"'t like how we use our params as globals, this calms it' \
    '  Write-Information "everything'"'"'s installed!"' \
    '  Remove-DockerShortcut $NoModifyPath'
