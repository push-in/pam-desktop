# Pam Desktop Linux host

This archive contains the official PAM Desktop host for Linux x86-64.

Verify the adjacent `.sha256` file before installing:

```sh
sha256sum --check pam-desktop-*.tar.gz.sha256
```

Install for the current user:

```sh
./install.sh
```

The default locations follow the XDG base-directory convention:

- versions: `${XDG_DATA_HOME:-$HOME/.local/share}/pam-desktop`;
- commands: `${XDG_BIN_HOME:-$HOME/.local/bin}`.

No `sudo` is used. Make sure the command directory is present in `PATH`, then
use the public `pam desktop` workflow. `pam-desktop` is the internal native
host delegated to by PAM.

Run `./uninstall.sh` to remove this exact host version. Removing an older
version never changes command links that already point to a newer version.
