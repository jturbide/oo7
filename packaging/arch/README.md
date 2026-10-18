# Pinned Arch Linux package

`oo7-local-git` builds the daemon, login helper, PAM module, CLI and Secret portal from the full Git commit recorded in `PKGBUILD`, with the committed Cargo.lock. It is a personal integration package based on upstream main, not an upstream stable release. The `-git` suffix identifies its source; upgrading deliberately requires reviewing and changing the pin.

Run `makepkg` from this directory after publishing the pinned source commit. Cargo uses upstream default features and release settings. `OO7_DAEMON_LOGIN_PATH` is explicitly `/usr/lib/oo7-daemon-login`. User-specific native build flags, stripping and injected LTO are disabled so packaging preserves the tested executables. A shared `CARGO_HOME` or absolute `CARGO_TARGET_DIR` may be supplied; no cache path is embedded in the recipe. Byte-for-byte reproducibility across toolchains or different build paths is not asserted.

The data renderer follows `server/data/meson.build` and `portal/data/meson.build`, using `/usr` as prefix and `lib` as libexecdir. It rejects unknown template substitutions, compiles only upstream LINGUAS translations, and installs both systemd activation aliases. The Secret Service D-Bus descriptor follows the existing Arch integration: `Exec=/bin/false`, with activation delegated to `dbus-org.freedesktop.secrets.service`.

The package has no install script, ALPM hooks, or `/etc` files. It does not enable, restart or stop services; edit PAM, portals or desktop configuration; read or migrate keyrings; or change memory-lock limits. Those are separately reviewed host integration tasks. Package-manager global hooks may still perform their normal housekeeping.

It conflicts with the other Secret Service providers `oo7`, `gnome-keyring` and the previous `oo7-local-beta`; only the latter is declared replaced. Moving to an official future package is an explicit package replacement after compatibility testing. Existing PAM and keyring configuration is outside this package and remains the administrator's responsibility.

For a pre-publication local build, `build-local.py` checks the recipe pin against the supplied worktree, rejects tracked source differences outside this packaging directory, and prepares makepkg's source directory as a link to that exact worktree. It never checks out, resets, commits, installs, or publishes anything. Use a new empty output directory. It defaults to running the recipe checks; `--nocheck` is available when the exact source and ELF files have already passed the separately recorded isolated acceptance tests.
