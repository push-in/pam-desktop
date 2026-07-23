#!/bin/sh

set -eu

VERSION='@VERSION@'

fail()
{
    printf 'pam-desktop uninstall: %s\n' "$1" >&2
    exit 1
}

[ "$(uname -s)" = 'Linux' ] || fail 'this package only supports Linux'

DATA_HOME=${XDG_DATA_HOME:-"${HOME:?HOME must be set}/.local/share"}
BIN_HOME=${XDG_BIN_HOME:-"${HOME:?HOME must be set}/.local/bin"}
INSTALL_ROOT=${PAM_DESKTOP_INSTALL_ROOT:-"${DATA_HOME}/pam-desktop"}

case "$INSTALL_ROOT" in
    /*) ;;
    *) fail 'the installation root must be absolute' ;;
esac
case "$INSTALL_ROOT" in
    */../*|*/..|*/./*|*/.) fail 'the installation root must not contain dot segments' ;;
esac
case "$BIN_HOME" in
    /*) ;;
    *) fail 'the command directory must be absolute' ;;
esac
case "$BIN_HOME" in
    */../*|*/..|*/./*|*/.) fail 'the command directory must not contain dot segments' ;;
esac

DESTINATION="${INSTALL_ROOT}/${VERSION}"
case "$DESTINATION" in
    "${INSTALL_ROOT}/"*) ;;
    *) fail 'unsafe installation destination' ;;
esac
[ "$DESTINATION" != "$INSTALL_ROOT" ] || fail 'unsafe installation destination'

for command in pam-desktop pam-desktop-launcher
do
    link="${BIN_HOME}/${command}"
    if [ -L "$link" ] \
        && [ "$(readlink "$link")" = "${DESTINATION}/bin/${command}" ]; then
        rm -f "$link"
    fi
done

if [ -e "$DESTINATION" ] || [ -L "$DESTINATION" ]; then
    [ -d "$DESTINATION" ] && [ ! -L "$DESTINATION" ] \
        || fail 'the installed version is not a regular directory'
    rm -rf "$DESTINATION"
fi

rmdir "$INSTALL_ROOT" 2>/dev/null || true
printf '[ok] PAM Desktop %s uninstalled\n' "$VERSION"
