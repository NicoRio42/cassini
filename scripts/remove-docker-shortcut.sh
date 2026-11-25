# Inserted into dist's cassini-installer.sh by patch-installers.sh, and
# called right after the binary is installed.
#
# Cassini used to be run with Docker, through a `cassini` shell function that
# the installation guide had users add to their ~/.bashrc:
#
#     cassini() {
#       docker run --rm -it --name cassini -v "$(pwd)":/app nicorio42/cassini "$@"
#     }
#
# A function takes precedence over the executable in the PATH, so it would
# keep running the Docker image. This removes it, along with one-line
# versions of it and `alias cassini=...` lines that run Docker, keeping a copy
# of each file it changes. A `cassini` function that doesn't run Docker is
# left alone. With CASSINI_NO_MODIFY_PATH=1 (`$1` is 1), it only tells the
# user where the shortcut is.
remove_docker_shortcut() {
    local _warn_only="$1"
    local _home="${INFERRED_HOME:-$HOME}"
    local _found=""
    local _rcfile _file _tmp

    for _rcfile in .bashrc .bash_profile .bash_login .profile .zshrc .zshenv; do
        _file="$_home/$_rcfile"
        if [ ! -f "$_file" ]; then
            continue
        fi

        _tmp="$_file.cassini-tmp"
        awk '
            # The start of a multi-line `cassini` function: `cassini() {`,
            # `function cassini {`, or the same with the brace on the next line.
            !in_block && /^[ \t]*(function[ \t]+)?cassini[ \t]*(\([ \t]*\))?[ \t]*[{]?[ \t]*$/ {
                in_block = 1
                block = ""
                depth = 0
                opened = 0
            }
            in_block {
                block = block $0 "\n"
                opens = gsub(/[{]/, "&")
                closes = gsub(/[}]/, "&")
                depth += opens - closes
                if (opens > 0) opened = 1
                if (opened && depth <= 0) {
                    if (block ~ /docker/) removed = 1
                    else printf "%s", block
                    in_block = 0
                }
                next
            }
            /^[ \t]*(function[ \t]+)?cassini[ \t]*(\([ \t]*\))?[ \t]*[{].*[}][ \t]*;?[ \t]*$/ && /docker/ { removed = 1; next }
            /^[ \t]*alias[ \t]+cassini=/ && /docker/ { removed = 1; next }
            { print }
            END {
                # An unterminated block is not a function: keep it.
                if (in_block) printf "%s", block
                exit !removed
            }
        ' "$_file" > "$_tmp" 2>/dev/null
        # awk fails when it removed nothing.
        if [ $? -ne 0 ]; then
            rm -f "$_tmp"
            continue
        fi

        _found="$_found $_file"
        if [ "$_warn_only" = "1" ]; then
            rm -f "$_tmp"
            continue
        fi
        # `cat >` rather than `mv`, to keep the file's permissions, and the
        # file a symbolic link points to.
        if cp "$_file" "$_file.cassini-backup" && cat "$_tmp" > "$_file"; then
            say "removed the Docker cassini shortcut from $_file (backup: $_file.cassini-backup)"
        else
            warn "could not remove the Docker cassini shortcut from $_file"
        fi
        rm -f "$_tmp"
    done

    if [ -z "$_found" ]; then
        return 0
    fi
    if [ "$_warn_only" = "1" ]; then
        warn "A cassini shell function or alias that runs Docker is defined in:$_found"
        warn "Remove it, or the cassini command will keep running the Docker image."
    else
        say "Open a new terminal, so that the cassini command runs the installed executable."
    fi
}
