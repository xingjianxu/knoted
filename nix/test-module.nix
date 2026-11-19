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
      extensions = ps: [ ps.pgvector ];
      initialScript = pkgs.writeText "knoted-vm-vector.sql" ''
        \connect template1
        CREATE EXTENSION vector;
      '';
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
      intelligencePort = 16603;
      doclingArtifactsDir = "/models/docling";
      bgeM3Dir = "/models/bge";
      gpuUuid = null;
      runtimeLibraries = with pkgs; [ stdenv.cc.cc.lib zlib glib libGL ];
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
      [intelligence]
      base_url = "http://127.0.0.1:16603"
      EOF
    '';
    systemd.services.knoted-intelligence.environment = {
      KNOTED_INTELLIGENCE_CHILD_HOLD_MS = "60000";
      KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM = "1";
      KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS = "0.2";
    };
    # Start through normal dependencies after installing real competing listeners.
    systemd.sockets.knoted-intelligence-port.wantedBy = lib.mkForce [];
    systemd.services.knoted-api.wantedBy = lib.mkForce [];
    systemd.services.knoted-worker.wantedBy = lib.mkForce [];
    systemd.services.knoted-intelligence.wantedBy = lib.mkForce [];
    environment.systemPackages = [ pkgs.curl pkgs.python3 ];
  };
  testScript = ''
    machine.start()
    machine.wait_for_unit("postgresql.target")
    # Run the checks in the service main process, then exec its original
    # command in-place. systemd runs ExecStartPre outside the private PID ns.
    machine.succeed("mkdir -p /run/systemd/system/knoted-intelligence.service.d")
    machine.succeed("""(echo '[Service]'; echo 'ExecStart='; sed -n 's|^ExecStart=|ExecStart=${pkgs.python3}/bin/python3 ${./test-sandbox.py} |p' /etc/systemd/system/knoted-intelligence.service) > /run/systemd/system/knoted-intelligence.service.d/check.conf""")
    machine.succeed("systemctl daemon-reload")
    machine.succeed("test -S /run/postgresql/.s.PGSQL.5432 && pg_isready -h 127.0.0.1")
    app_units = "knoted-api knoted-worker knoted-intelligence"
    for section, key in [("credentials", "credential_file"), ("chat.local", "credential_file"),
                         ("chat.external.test", "credential_file"), ("office", "jwt_secret_file")]:
        machine.succeed(f"""cp /run/t70.toml /run/t70-clean; printf '\\n[{section}]\\n{key} = "/run/outside"\\n' >> /run/t70.toml""")
        machine.fail("systemctl start knoted-migrate")
        machine.succeed("""test "$(sudo -u postgres psql -At -d knoted -c "select to_regclass('_sqlx_migrations') is null")" = t""")
        machine.succeed("mv /run/t70-clean /run/t70.toml; systemctl stop knoted-intelligence-port.socket; systemctl reset-failed")
    for port in [16602, 16603]:
        machine.succeed(f"systemd-run --unit=t70-holder-{port} ${pkgs.python3}/bin/python3 -m http.server --bind 127.0.0.1 {port}")
        machine.wait_until_succeeds(f"curl -fsS http://127.0.0.1:{port}/ >/dev/null")
        machine.fail(f"systemctl start {app_units}")
        machine.succeed(f"systemctl is-active t70-holder-{port}")
        for unit in app_units.split():
            machine.fail(f"systemctl is-active {unit}")
        machine.succeed("""test "$(sudo -u postgres psql -At -d knoted -c "select to_regclass('_sqlx_migrations') is null")" = t""")
        machine.succeed("test ! -e /var/lib/knoted/python/executions.db")
        machine.succeed(f"systemctl stop {app_units} knoted-migrate knoted-intelligence-port.service knoted-intelligence-port.socket t70-holder-{port}")
        machine.succeed("systemctl reset-failed")
    machine.succeed(f"systemctl start {app_units}")
    machine.wait_for_unit("knoted-api.service")
    machine.wait_for_unit("knoted-worker.service")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/health/ready")
    machine.wait_until_succeeds("curl -s http://127.0.0.1:16603/internal/v1/health | grep status", timeout=30)
    machine.succeed("ss -Hltn | grep -F 127.0.0.1:16603")
    address = machine.succeed("hostname -I").split()[0]
    for port in [16602, 16603]:
        machine.fail(f"curl --noproxy '*' -sS --max-time 2 http://{address}:{port}")
    migration_id = machine.succeed("systemctl show knoted-migrate -p InvocationID --value")
    print(machine.succeed("python3 ${./test-guardian.py} /var/lib/knoted 16603"))
    machine.succeed("systemctl stop knoted-intelligence")
    machine.fail("${pkgs.python3}/bin/python3 -c \"import socket; socket.socket().bind(('127.0.0.1',16603))\"")
    machine.fail("curl -fsS --max-time 2 http://127.0.0.1:16603/internal/v1/health")
    machine.succeed("systemctl restart knoted-worker knoted-api")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/health/ready")
    machine.succeed("systemctl is-active knoted-worker")
    machine.fail("systemctl is-active knoted-intelligence")
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
