{ config, lib, pkgs, ... }:
let
  cfg = config.services.knoted;
  inherit (lib) mkOption types;
  # Paths also appear in systemd tmpfiles and command arguments.
  absolute = types.strMatching "/([a-zA-Z0-9_+.-]+/)*[a-zA-Z0-9_+.-]+";
  canonical = p: lib.all (c: c != "." && c != "..") (lib.splitString "/" p);
  external = p: canonical p && p != "/nix/store" && !(lib.hasPrefix "/nix/store/" p);
  root = cfg.instanceRoot;
  dirs = [ "assets" "staging" "python" "cache" "tmp" ];
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
    (toString cfg.apiPort) (toString cfg.intelligencePort)
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
    sourceRoot = mkOption { type = types.nullOr absolute; default = null; };
    apiPort = mkOption { type = types.port; };
    intelligencePort = mkOption { type = types.port; };
    user = mkOption { type = types.strMatching "[a-z_][a-z0-9_-]*"; default = "knoted"; };
    group = mkOption { type = types.strMatching "[a-z_][a-z0-9_-]*"; default = "knoted"; };
    doclingArtifactsDir = mkOption { type = absolute; };
    bgeM3Dir = mkOption { type = absolute; };
    gpuUuid = mkOption { type = types.nullOr (types.strMatching "GPU-[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}"); description = "Embedding GPU UUID, or null for CPU."; };
    runtimeLibraries = mkOption { type = types.listOf types.package; default = []; description = "Runtime libraries required by the bundled Python interpreter and wheels."; };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      { assertion = cfg.apiPort != cfg.intelligencePort; message = "Knoted ports must differ."; }
      { assertion = lib.all external [ root cfg.configFile ]; message = "Knoted runtime configuration and state require canonical external paths."; }
      { assertion = cfg.user != "root" && cfg.group != "root"; message = "Knoted requires a dedicated unprivileged account."; }
      { assertion = lib.all (p: canonical p && !(lib.hasPrefix "${root}/" p) && p != root && !(lib.hasPrefix "${p}/" root))
          ([ cfg.configFile cfg.doclingArtifactsDir cfg.bgeM3Dir ] ++ lib.optional (cfg.sourceRoot != null) cfg.sourceRoot);
        message = "Configuration, source and model paths must be canonical and separate from application state."; }
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
        after = [ "knoted-migrate.service" "knoted-intelligence.service" ];
        environment.KNOTED_INTELLIGENCE_BGE_M3_DIR = cfg.bgeM3Dir;
        serviceConfig.TimeoutStopSec = 75;
      };
      knoted-intelligence = {
        wantedBy = [ "multi-user.target" ];
        path = [ pkgs.bash pkgs.coreutils ];
        environment = {
          KNOTED_INTELLIGENCE_HOST = "127.0.0.1";
          KNOTED_INTELLIGENCE_PORT = toString cfg.intelligencePort;
          KNOTED_INTELLIGENCE_STATE_DIR = "${root}/python";
          KNOTED_INTELLIGENCE_STAGING_DIR = "${root}/staging";
          KNOTED_INTELLIGENCE_DOCLING_ARTIFACTS_DIR = cfg.doclingArtifactsDir;
          KNOTED_INTELLIGENCE_BGE_M3_DIR = cfg.bgeM3Dir;
          KNOTED_INTELLIGENCE_DOCLING_DEVICE = if cfg.gpuUuid == null then "cpu" else "cuda:0";
          KNOTED_INTELLIGENCE_GPU_SLOTS = "1";
          CUDA_VISIBLE_DEVICES = if cfg.gpuUuid == null then "" else cfg.gpuUuid;
          KNOTED_EMBED_GPU_UUID = if cfg.gpuUuid == null then "" else cfg.gpuUuid;
          HOME = "${root}/python";
          TMPDIR = "${root}/tmp";
          XDG_CACHE_HOME = "${root}/cache";
          LD_LIBRARY_PATH = lib.concatStringsSep ":" (
            lib.optional (cfg.gpuUuid != null) "/run/opengl-driver/lib"
            ++ map (pkg: "${lib.getLib pkg}/lib") cfg.runtimeLibraries
          );
        } // lib.optionalAttrs config.programs.nix-ld.enable {
          inherit (config.environment.sessionVariables) NIX_LD NIX_LD_LIBRARY_PATH;
        };
        serviceConfig = common // {
          Type = "exec";
          ExitType = "cgroup";
          KillMode = "mixed";
          TimeoutStopSec = 45;
          Restart = "on-failure";
          RestartSec = 3;
          ExecStart = "${pkgs.bash}/bin/bash ${cfg.package}/bin/knoted-intelligence";
          ReadWritePaths = map (d: "${root}/${d}") [ "python" "staging" "cache" "tmp" ];
          InaccessiblePaths = [ cfg.configFile "${root}/assets" ] ++ lib.optional (cfg.sourceRoot != null) "-${cfg.sourceRoot}";
        };
      };
    };
  };
}
