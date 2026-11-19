# Isolated VM only. Pass the verified artifact directory and full commit ID.
{ pkgs ? import <nixpkgs> {}, artifact, revision }:
let
  package = import ./package.nix { inherit pkgs artifact revision; };
in pkgs.testers.runNixOSTest {
  name = "knoted-t70-lifecycle";
  nodes.machine = { pkgs, ... }: {
    imports = [ ./module.nix ];
    virtualisation.memorySize = 4096;
    virtualisation.cores = 4;
    programs.nix-ld.enable = true;
    services.postgresql = {
      enable = true;
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
      apiPort = 16602;
      intelligencePort = 16603;
      doclingArtifactsDir = "/models/docling";
      bgeM3Dir = "/models/bge";
      gpuUuid = null;
      runtimeLibraries = with pkgs; [ stdenv.cc.cc.lib zlib glib libGL ];
    };
    system.activationScripts.t70.text = ''
      touch /run/knoted-t70-isolated-vm
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
      [intelligence]
      base_url = "http://127.0.0.1:16603"
      EOF
    '';
    systemd.services.knoted-intelligence.environment = {
      KNOTED_INTELLIGENCE_CHILD_HOLD_MS = "60000";
      KNOTED_INTELLIGENCE_CHILD_IGNORE_SIGTERM = "1";
      KNOTED_INTELLIGENCE_CANCEL_GRACE_SECONDS = "0.2";
    };
    environment.systemPackages = [ pkgs.curl pkgs.python3 ];
  };
  testScript = ''
    machine.start()
    machine.wait_for_unit("knoted-api.service")
    machine.wait_for_unit("knoted-worker.service")
    machine.wait_until_succeeds("curl -fsS http://127.0.0.1:16602/health/ready")
    machine.wait_until_succeeds("curl -s http://127.0.0.1:16603/internal/v1/health | grep status")
    machine.succeed("ss -Hltn | grep -F 127.0.0.1:16603")
    address = machine.succeed("hostname -I").split()[0]
    for port in [16602, 16603]:
        machine.fail(f"curl --noproxy '*' -sS --max-time 2 http://{address}:{port}")
    migration_id = machine.succeed("systemctl show knoted-migrate -p InvocationID --value")
    print(machine.succeed("python3 ${./test-guardian.py} /var/lib/knoted 16603"))
    machine.succeed("systemctl stop knoted-intelligence")
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
