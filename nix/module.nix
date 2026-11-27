{ config, lib, pkgs, ... }:
let
  cfg = config.services.knoted;
  inherit (lib) mkOption types;
  # Paths also appear in systemd tmpfiles and command arguments.
  absolute = types.strMatching "/([a-zA-Z0-9_+.-]+/)*[a-zA-Z0-9_+.-]+";
  canonical = p: lib.all (c: c != "." && c != "..") (lib.splitString "/" p);
  external = p: canonical p && p != "/nix/store" && !(lib.hasPrefix "/nix/store/" p);
  root = cfg.instanceRoot;
  dirs = [ "assets" "staging" ];
  common = {
    User = cfg.user;
    Group = cfg.group;
    UMask = "0077";
    NoNewPrivileges = true;
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" "AF_NETLINK" ];
  };
  check = lib.escapeShellArgs [
    "${pkgs.python3}/bin/python3" "${./runtime.py}" cfg.configFile root
    (if cfg.sourceRoot == null then "" else cfg.sourceRoot)
    (toString cfg.apiPort) (builtins.toJSON cfg.rustOnlyPaths)
  ];
  rust = name: {
    wantedBy = [ "multi-user.target" ];
    requires = [ "knoted-migrate.service" ];
    after = [ "knoted-migrate.service" ];
    serviceConfig = common // {
      Type = "exec";
      ExecStartPre = check;
      ExecStart = "${cfg.package}/bin/knoted-${name} --config ${cfg.configFile}";
      Restart = "on-failure";
      RestartSec = 3;
      ReadWritePaths = [ "${root}/assets" "${root}/staging" ];
    };
  };
in {
  options.services.knoted = {
    enable = lib.mkEnableOption "Knoted production application";
    package = mkOption { type = types.package; };
    sqlxPackage = mkOption {
      type = types.package;
      # sqlx-cli 0.9 enables SQLite alongside PostgreSQL in nixpkgs.
      default = pkgs.sqlx-cli.overrideAttrs (old: {
        buildInputs = old.buildInputs ++ [ pkgs.sqlite ];
      });
    };
    configFile = mkOption { type = absolute; description = "External runtime TOML with database and service credentials."; };
    instanceRoot = mkOption { type = absolute; description = "Dedicated application state directory."; };
    rustOnlyPaths = mkOption {
      type = types.listOf absolute;
      default = [];
      description = "Canonical external credential files or directories for the Rust services.";
    };
    sourceRoot = mkOption { type = types.nullOr absolute; default = null; };
    apiPort = mkOption { type = types.port; };
    user = mkOption { type = types.strMatching "[a-z_][a-z0-9_-]*"; default = "knoted"; };
    group = mkOption { type = types.strMatching "[a-z_][a-z0-9_-]*"; default = "knoted"; };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      { assertion = lib.all external cfg.rustOnlyPaths;
        message = "Knoted credential paths must be canonical external paths."; }
      { assertion = lib.all external [ root cfg.configFile ]; message = "Knoted runtime configuration and state require canonical external paths."; }
      { assertion = cfg.user != "root" && cfg.group != "root"; message = "Knoted requires a dedicated unprivileged account."; }
      { assertion = lib.all (p: canonical p && !(lib.hasPrefix "${root}/" p) && p != root && !(lib.hasPrefix "${p}/" root))
          ([ cfg.configFile ] ++ lib.optional (cfg.sourceRoot != null) cfg.sourceRoot);
        message = "Configuration and source paths must be canonical and separate from application state."; }
    ];
    users.groups.${cfg.group} = {};
    users.users.${cfg.user} = { isSystemUser = true; inherit (cfg) group; };
    systemd.tmpfiles.rules = map (p: "d ${p} 0700 ${cfg.user} ${cfg.group} -") ([ root ] ++ map (d: "${root}/${d}") dirs);
    systemd.services = {
      knoted-migrate = {
        after = [ "network.target" "postgresql.service" "postgresql.target" ];
        serviceConfig = common // {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = "${check} ${cfg.sqlxPackage}/bin/sqlx ${cfg.package}/share/knoted/migrations";
        };
      };
      knoted-api = rust "api";
      knoted-worker = lib.recursiveUpdate (rust "worker") {
        serviceConfig.TimeoutStopSec = 75;
      };
    };
  };
}
