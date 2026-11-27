# Isolated Chromium + Caddy test. Use only a verified release package.
{ pkgs ? import <nixpkgs> {}
, artifact
, revision
}:
let
  package = import ./package.nix { inherit pkgs artifact revision; };
  browserPython = pkgs.python3.withPackages (ps: [ ps.selenium ]);
  image = ../tests/fixtures/ocr/sample_zh_en.png;
in pkgs.testers.runNixOSTest {
  name = "knoted-t70-browser-restart";
  nodes.machine = { pkgs, ... }: {
    imports = [ ./module.nix ];
    virtualisation.memorySize = 4096;
    virtualisation.cores = 4;
    programs.nix-ld.enable = true;
    services.postgresql = {
      enable = true;
      ensureDatabases = [ "knoted" ];
      ensureUsers = [{ name = "knoted"; ensureDBOwnership = true; }];
    };
    services.knoted = {
      enable = true;
      inherit package;
      configFile = "/run/t70-private/app.toml";
      instanceRoot = "/var/lib/knoted";
      apiPort = 16602;
    };
    system.activationScripts.t70.text = ''
      touch /run/t70-browser-vm
      install -d -o root -g knoted -m 0750 /run/t70-private
      cat > /run/t70-private/app.toml <<'CONFIG'
      [server]
      bind = "127.0.0.1:16602"
      [database]
      url = "postgresql://knoted@localhost/knoted?host=/run/postgresql"
      [auth]
      rp_id = "knoted.test"
      origin = "https://t70.knoted.test:16601"
      [assets]
      root = "/var/lib/knoted/assets"
      CONFIG
      chown root:knoted /run/t70-private/app.toml
      chmod 0640 /run/t70-private/app.toml
    '';
    systemd.services.t70-https = let
      caddyfile = pkgs.writeText "t70-browser-Caddyfile" ''
        {
          admin 127.0.0.1:16610
          auto_https off
        }
        https://t70.knoted.test:16601 {
          bind 127.0.0.1
          tls /var/lib/t70-tls/server.crt /var/lib/t70-tls/server.key
          @backend path /api/* /health/*
          handle @backend {
            reverse_proxy 127.0.0.1:16602
          }
          handle {
            root * ${package}/share/knoted/web
            try_files {path} /index.html
            file_server
          }
        }
      '';
    in {
      serviceConfig = {
        Environment = "HOME=/var/lib/t70-tls";
        ExecStart = "${pkgs.caddy}/bin/caddy run --config ${caddyfile} --adapter caddyfile";
        ExecReload = "${pkgs.caddy}/bin/caddy reload --config ${caddyfile} --adapter caddyfile --force";
      };
    };
    environment.systemPackages = with pkgs; [
      browserPython chromium chromedriver curl jq openssl
    ];
  };
  testScript = ''
    machine.start()
    machine.wait_for_unit("knoted-worker.service")
    machine.succeed("""umask 077; mkdir -p /var/lib/t70-tls; cd /var/lib/t70-tls
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
        -subj /CN=t70-test-ca -keyout ca.key -out ca.crt > /dev/null 2>&1
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
        -subj /CN=t70.knoted.test -keyout server.key -out server.csr > /dev/null 2>&1
    printf 'subjectAltName=DNS:t70.knoted.test\nextendedKeyUsage=serverAuth\n' > server.ext
    openssl x509 -req -days 1 -in server.csr -CA ca.crt -CAkey ca.key -set_serial 1 \
        -extfile server.ext -out server.crt > /dev/null 2>&1
    openssl pkey -in server.key -pubout -outform DER | openssl dgst -binary -sha256 | openssl base64 -A > pin
    """)
    machine.succeed("systemctl start t70-https")
    tls = "curl -fsS --noproxy '*' --resolve t70.knoted.test:16601:127.0.0.1 --cacert /var/lib/t70-tls/ca.crt https://t70.knoted.test:16601"
    readiness = tls + "/health/ready | jq -e '.status == \"ready\"'"
    machine.wait_until_succeeds(readiness)
    browser = "${browserPython}/bin/python3 ${./test-production-browser.py}"
    pin = "$(cat /var/lib/t70-tls/pin)"
    print(machine.succeed(f"{browser} before-reboot ${package} ${image} {pin}"))
    machine.succeed('test "$(cat ${package}/share/knoted/REVISION)" = "${revision}"')
    machine.succeed('test "$(readlink -f /proc/$(systemctl show knoted-api -p MainPID --value)/exe)" = "${package}/bin/knoted-api"')

    boot = machine.succeed("cat /proc/sys/kernel/random/boot_id")
    machine.shutdown()
    machine.start()
    machine.wait_for_unit("knoted-worker.service")
    assert machine.succeed("cat /proc/sys/kernel/random/boot_id") != boot
    machine.succeed("systemctl start t70-https")
    machine.wait_until_succeeds(readiness)
    print(machine.succeed(f"{browser} after-reboot ${package} ${image} {pin}"))

    certificate = """openssl s_client -connect 127.0.0.1:16601 -servername t70.knoted.test \
       </dev/null 2>/dev/null | openssl x509 -noout -serial"""
    assert machine.succeed(certificate).strip() == "serial=01"
    machine.succeed("""cd /var/lib/t70-tls
    openssl x509 -req -days 2 -in server.csr -CA ca.crt -CAkey ca.key -set_serial 2 \
        -extfile server.ext -out server-new.crt > /dev/null 2>&1
    mv server-new.crt server.crt
    systemctl reload t70-https
    """)
    assert machine.succeed(certificate).strip() == "serial=02"
    machine.succeed(readiness)
    print("T70 renewed certificate observed before and after Caddy reload")
  '';
}
