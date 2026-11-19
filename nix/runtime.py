"""Validate the host's runtime TOML before Rust services start."""
import os
from pathlib import Path
import sys
import tomllib
from urllib.parse import urlsplit


def main():
    config_file, instance, source, api_port, intelligence_port, *migration = sys.argv[1:]
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
    database = config["database"]["url"]
    parsed = urlsplit(database)
    if parsed.scheme not in {"postgres", "postgresql"} or parsed.path in {"", "/"}:
        sys.exit("runtime TOML must name an existing PostgreSQL database")
    if migration:
        sqlx, migrations = migration
        os.environ["DATABASE_URL"] = database
        os.execv(sqlx, [sqlx, "migrate", "run", "--source", migrations])


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError):
        sys.exit("invalid or unreadable runtime configuration")
