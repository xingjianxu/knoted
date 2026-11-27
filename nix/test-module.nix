# Isolated VM only. Pass the verified artifact directory and full commit ID.
{ pkgs ? import <nixpkgs> {}, artifact, revision }:
let
  package = import ./package.nix { inherit pkgs artifact revision; };
in pkgs.testers.runNixOSTest {
  name = "knoted-t70-lifecycle";
  nodes.machine = { lib, pkgs, ... }: {
    imports = [ ./module.nix ];
    virtualisation.memorySize = 4096;
    virtualisation.cores = 4;
    programs.nix-ld.enable = true;
    services.postgresql = {
      enable = true;
      enableTCPIP = true;
      ensureDatabases = [ "knoted" ];
      ensureUsers = [{ name = "knoted"; ensureDBOwnership = true; }];
    };
    services.knoted = {
      enable = true;
      inherit package;
      configFile = "/run/t70.toml";
      instanceRoot = "/var/lib/knoted";
      rustOnlyPaths = [ "/run/t70-rust" ];
      sourceRoot = "/srv/t70-source";
      apiPort = 16602;
    };
    system.activationScripts.t70.text = ''
      touch /run/knoted-t70-isolated-vm
      mkdir -p /run/t70-rust /srv/t70-source /var/lib/knoted/assets
      touch /run/t70-rust/{strata,pg} /srv/t70-source/marker /var/lib/knoted/assets/marker
      cat > /run/t70.toml <<'EOF'
      [server]
      bind = "127.0.0.1:16602"
      [database]
      url = "postgresql://knoted@localhost/knoted?host=/run/postgresql"
      [auth]
      rp_id = "knoted.test"
      origin = "https://knoted.test"
      [assets]
      root = "/var/lib/knoted/assets"
      [files]
      enabled = false
      root = "/srv/t70-source"
      EOF
    '';
    # Start through normal dependencies after installing a competing API listener.
    systemd.services.knoted-api.wantedBy = lib.mkForce [];
    systemd.services.knoted-worker.wantedBy = lib.mkForce [];
    environment.systemPackages = [ pkgs.curl pkgs.python3 ];
  };
  testScript = ''
    machine.start()
    machine.wait_for_unit("postgresql.target")
    machine.succeed("test -S /run/postgresql/.s.PGSQL.5432 && pg_isready -h 127.0.0.1")
    app_units = "knoted-api knoted-worker"
    for section, key in [("credentials", "credential_file"), ("office", "jwt_secret_file")]:
        machine.succeed(f"""cp /run/t70.toml /run/t70-clean; printf '\n[{section}]\n{key} = "/run/outside"\n' >> /run/t70.toml""")
        machine.fail("systemctl start knoted-migrate")
        machine.succeed("""test "$(sudo -u postgres psql -At -d knoted -c "select to_regclass('_sqlx_migrations') is null")" = t""")
        machine.succeed("mv /run/t70-clean /run/t70.toml; systemctl reset-failed")

    machine.succeed("systemd-run --unit=t70-holder-api ${pkgs.python3}/bin/python3 -m http.server --bind 127.0.0.1 16602")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/ >/dev/null")
    machine.fail(f"systemctl start {app_units}")
    machine.succeed("systemctl is-active t70-holder-api")
    for unit in app_units.split():
        machine.fail(f"systemctl is-active {unit}")
    machine.succeed("""test "$(sudo -u postgres psql -At -d knoted -c "select to_regclass('_sqlx_migrations') is null")" = t""")
    machine.succeed(f"systemctl stop {app_units} knoted-migrate t70-holder-api")
    machine.succeed("systemctl reset-failed")

    # An unrelated listener on the retired port must not affect startup.
    machine.succeed("systemd-run --unit=t70-holder-retired ${pkgs.python3}/bin/python3 -m http.server --bind 127.0.0.1 16603")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16603/ >/dev/null")
    machine.succeed(f"systemctl start {app_units}")
    machine.wait_for_unit("knoted-api.service")
    machine.wait_for_unit("knoted-worker.service")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/health/ready")
    machine.succeed("systemctl is-active t70-holder-retired")
    address = machine.succeed("hostname -I").split()[0]
    machine.fail(f"curl --noproxy '*' -sS --max-time 2 http://{address}:16602")
    migration_id = machine.succeed("systemctl show knoted-migrate -p InvocationID --value")
    machine.succeed("systemctl restart knoted-worker knoted-api")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/health/ready")
    machine.succeed("systemctl is-active knoted-worker")
    machine.succeed("ss -Hltn | grep -F 127.0.0.1:16602")
    assert machine.succeed("systemctl show knoted-migrate -p InvocationID --value") == migration_id
    machine.succeed("systemctl stop knoted-api")
    machine.succeed("systemd-run --unit=t70-holder ${pkgs.python3}/bin/python3 -m http.server --bind 127.0.0.1 16602")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/ > /dev/null")
    machine.execute("systemctl start knoted-api")
    machine.wait_until_succeeds("test $(systemctl show knoted-api -p NRestarts --value) -gt 0")
    machine.succeed("systemctl is-active t70-holder")
    machine.succeed("systemctl stop knoted-api t70-holder; systemctl start knoted-api")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/health/ready")
  '';
}
