#!/usr/bin/python
"""Install upstream Meson data using exactly its documented substitutions."""

import pathlib
import re
import subprocess
import sys

source = pathlib.Path(sys.argv[1])
package = pathlib.Path(sys.argv[2])


def render(relative, destination, values):
    text = (source / relative).read_text()
    tokens = set(re.findall(r"@([A-Za-z_][A-Za-z_0-9]*)@", text))
    if not tokens <= values.keys():
        raise ValueError(
            f"Unknown upstream template substitutions: {relative}: "
            f"{tokens - values.keys()}"
        )
    for token in tokens:
        text = text.replace(f"@{token}@", values[token])
    target = package / destination
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text)
    target.chmod(0o644)


server = {
    "binary": "oo7-daemon",
    "libexecdir": "/usr/lib",
    "dbus_known_name": "org.freedesktop.secrets",
}
portal = {
    "libexecdir": "/usr/lib",
    "bin_name": "oo7-portal",
    "service_name": "oo7-portal",
    "dbus_name": "org.freedesktop.impl.portal.desktop.oo7",
}
render(
    "server/data/oo7-daemon.service.in",
    "usr/lib/systemd/user/oo7-daemon.service",
    server,
)
for filename, directory in [
    ("oo7-portal.service", "usr/lib/systemd/user"),
    ("org.freedesktop.impl.portal.desktop.oo7.service", "usr/share/dbus-1/services"),
    ("oo7-portal.desktop", "usr/share/applications"),
    ("oo7-portal.portal", "usr/share/xdg-desktop-portal/portals"),
]:
    render("portal/data/" + filename + ".in", directory + "/" + filename, portal)
for name, target in [
    ("dbus-org.freedesktop.secrets.service", "oo7-daemon.service"),
    ("dbus-org.freedesktop.impl.portal.desktop.oo7.service", "oo7-portal.service"),
]:
    (package / "usr/lib/systemd/user" / name).symlink_to(target)
for language in (source / "server/po/LINGUAS").read_text().splitlines():
    language = language.split("#", 1)[0].strip()
    if not language:
        continue
    if not re.fullmatch(r"[A-Za-z_@0-9.-]+", language):
        raise ValueError("Invalid upstream translation name")
    target = package / f"usr/share/locale/{language}/LC_MESSAGES/oo7-daemon.mo"
    target.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["msgfmt", "-o", str(target), str(source / "server/po" / (language + ".po"))],
        check=True,
    )
    target.chmod(0o644)
