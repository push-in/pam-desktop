#!/bin/sh

set -eu

VERSION='@VERSION@'
TARGET='@TARGET@'
PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)

fail()
{
    printf 'pam-desktop install: %s\n' "$1" >&2
    exit 1
}

[ "$(uname -s)" = 'Linux' ] || fail 'this package only supports Linux'
case "$(uname -m)" in
    x86_64|amd64) ;;
    *) fail "unsupported architecture; expected x86_64 for ${TARGET}" ;;
esac

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
[ ! -e "$DESTINATION" ] && [ ! -L "$DESTINATION" ] \
    || fail "version ${VERSION} is already installed"

for file in pam-desktop pam-desktop-launcher
do
    [ -f "${PACKAGE_DIR}/bin/${file}" ] \
        || fail "archive is missing bin/${file}"
    [ -x "${PACKAGE_DIR}/bin/${file}" ] \
        || fail "bin/${file} is not executable"
    [ ! -L "${PACKAGE_DIR}/bin/${file}" ] \
        || fail "bin/${file} must not be a symbolic link"
done

HOST_VERSION=$("${PACKAGE_DIR}/bin/pam-desktop" --version)
[ "$HOST_VERSION" = "pam-desktop ${VERSION}" ] \
    || fail "host reports an unexpected version: ${HOST_VERSION}"

mkdir -p "$INSTALL_ROOT" "$BIN_HOME"

for command in pam-desktop pam-desktop-launcher
do
    link="${BIN_HOME}/${command}"
    if [ -e "$link" ] || [ -L "$link" ]; then
        [ -L "$link" ] \
            || fail "${link} exists and is not managed by PAM Desktop"
        current=$(readlink "$link")
        case "$current" in
            "${INSTALL_ROOT}/"*/bin/"${command}") ;;
            *) fail "${link} does not point to a managed PAM Desktop version" ;;
        esac
    fi
done

STAGING="${INSTALL_ROOT}/.install-${VERSION}-$$"
HOST_LINK="${BIN_HOME}/.pam-desktop-$$"
LAUNCHER_LINK="${BIN_HOME}/.pam-desktop-launcher-$$"

cleanup()
{
    [ -n "${STAGING:-}" ] && [ -d "$STAGING" ] && rm -rf "$STAGING"
    [ -n "${HOST_LINK:-}" ] && [ -L "$HOST_LINK" ] && rm -f "$HOST_LINK"
    [ -n "${LAUNCHER_LINK:-}" ] && [ -L "$LAUNCHER_LINK" ] && rm -f "$LAUNCHER_LINK"
}
trap cleanup EXIT HUP INT TERM

[ ! -e "$STAGING" ] && [ ! -L "$STAGING" ] \
    || fail "temporary installation path already exists"
mkdir -p "${STAGING}/bin"
install -m 0755 "${PACKAGE_DIR}/bin/pam-desktop" "${STAGING}/bin/pam-desktop"
install -m 0755 \
    "${PACKAGE_DIR}/bin/pam-desktop-launcher" \
    "${STAGING}/bin/pam-desktop-launcher"
install -m 0644 "${PACKAGE_DIR}/manifest.json" "${STAGING}/manifest.json"
install -m 0644 "${PACKAGE_DIR}/README.md" "${STAGING}/README.md"
install -m 0644 "${PACKAGE_DIR}/LICENSE" "${STAGING}/LICENSE"
install -m 0755 "${PACKAGE_DIR}/uninstall.sh" "${STAGING}/uninstall.sh"

[ "$("${STAGING}/bin/pam-desktop" --version)" = "pam-desktop ${VERSION}" ] \
    || fail 'installed host verification failed'

mv "$STAGING" "$DESTINATION"
STAGING=''

ln -s "${DESTINATION}/bin/pam-desktop" "$HOST_LINK"
ln -s "${DESTINATION}/bin/pam-desktop-launcher" "$LAUNCHER_LINK"
mv -f "$HOST_LINK" "${BIN_HOME}/pam-desktop"
HOST_LINK=''
mv -f "$LAUNCHER_LINK" "${BIN_HOME}/pam-desktop-launcher"
LAUNCHER_LINK=''

trap - EXIT HUP INT TERM
printf '[ok] PAM Desktop %s installed in %s\n' "$VERSION" "$DESTINATION"
printf '[ok] Commands linked in %s\n' "$BIN_HOME"
