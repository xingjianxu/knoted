"""Validate the host's runtime TOML before Rust services start."""
import json
import os
import socket
from pathlib import Path
import sys
import tomllib
from urllib.parse import urlsplit


def main():
    config_file, instance, source, api_port, intelligence_port, rust_only_json, *migration = sys.argv[1:]
    with open(config_file, "rb") as stream:
        config = tomllib.load(stream)
    expected = [
        ("server", "bind", f"127.0.0.1:{api_port}"),
        ("intelligence", "base_url", f"http://127.0.0.1:{intelligence_port}"),
        ("assets", "root", str(Path(instance) / "assets")),
    ]
    files = config.get("files", {})
    if files.get("enabled", False) or "staging_root" in files:
        expected.append(("files", "staging_root", str(Path(instance) / "staging")))
    if source or files.get("enabled", False):
        if not source:
            sys.exit("enabled file sources require sourceRoot")
        expected.append(("files", "root", source))
    for section, field, value in expected:
        if config.get(section, {}).get(field) != value:
            sys.exit(f"unexpected {section}.{field} in runtime TOML")
    protected = [Path(path) for path in json.loads(rust_only_json)]
    if any(path != path.resolve() for path in protected):
        sys.exit("rustOnlyPaths must name stable canonical files or directories")
    resolved_config = Path(config_file).resolve()
    if resolved_config != Path(config_file) and not any(resolved_config.is_relative_to(path) for path in protected):
        sys.exit("a symlinked runtime configuration requires a protected canonical target")
    providers = config.get("chat", {})
    sections = [config.get("credentials", {}), providers.get("local", {}), *providers.get("external", {}).values()]
    secrets = [section["credential_file"] for section in sections if "credential_file" in section]
    if "jwt_secret_file" in config.get("office", {}):
        secrets.append(config["office"]["jwt_secret_file"])
    for filename in secrets:
        secret = Path(filename)
        if not secret.is_absolute() or not any(secret.resolve().is_relative_to(p) for p in protected):
            sys.exit("declare each Rust credential path under services.knoted.rustOnlyPaths")
    database = config["database"]["url"]
    parsed = urlsplit(database)
    if parsed.scheme not in {"postgres", "postgresql"} or parsed.path in {"", "/"}:
        sys.exit("runtime TOML must name an existing PostgreSQL database")
    if migration:
        sqlx, migrations = migration
        # Keep the API port reserved until sqlx exits. The Python TCP port is
        # already owned by the required systemd socket throughout migration.
        reservation = socket.socket()
        reservation.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            reservation.bind(("127.0.0.1", int(api_port)))
            reservation.listen()
        except OSError:
            sys.exit("configured API port is already occupied")
        reservation.set_inheritable(True)
        os.environ["DATABASE_URL"] = database
        os.execv(sqlx, [sqlx, "migrate", "run", "--source", migrations])


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError):
        sys.exit("invalid or unreadable runtime configuration")
