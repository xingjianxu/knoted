{ config, lib, pkgs, ... }:
let
  cfg = config.services.knoted;
  inherit (lib) mkOption types;
  # Paths also appear in systemd tmpfiles and command arguments.
  absolute = types.strMatching "/([a-zA-Z0-9_+.-]+/)*[a-zA-Z0-9_+.-]+";
  canonical = p: lib.all (c: c != "." && c != "..") (lib.splitString "/" p);
  external = p: canonical p && p != "/nix/store" && !(lib.hasPrefix "/nix/store/" p);
  root = cfg.instanceRoot;
  proxySocket = "/run/knoted-intelligence/backend.sock";
  portUnit = "knoted-intelligence-port.socket";
  sandboxPaths = [ cfg.configFile "${root}/assets" ] ++ cfg.rustOnlyPaths
    ++ lib.optional (cfg.sourceRoot != null) cfg.sourceRoot;
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
    (toString cfg.apiPort) (toString cfg.intelligencePort) (builtins.toJSON cfg.rustOnlyPaths)
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
      description = "External credential files or directories accessible only to the Rust services.";
    };
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
      { assertion = lib.all external cfg.rustOnlyPaths;
        message = "Knoted credential paths must be canonical external paths."; }
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
    systemd.sockets.knoted-intelligence-port = {
      wantedBy = [ "sockets.target" ];
      socketConfig.ListenStream = "127.0.0.1:${toString cfg.intelligencePort}";
    };
    systemd.services = {
      knoted-intelligence-port = {
        # Keep ownership of the worker endpoint when inference is unavailable.
        bindsTo = [ portUnit ];
        after = [ portUnit ];
        serviceConfig = common // {
          ExecStart = "${config.systemd.package}/lib/systemd/systemd-socket-proxyd ${proxySocket}";
        };
      };
      knoted-migrate = {
        requires = [ portUnit ];
        after = [ portUnit "network.target" "postgresql.service" "postgresql.target" ];
        serviceConfig = common // {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = "${check} ${cfg.sqlxPackage}/bin/sqlx ${cfg.package}/share/knoted/migrations";
        };
      };
      knoted-api = rust "api";
      knoted-worker = lib.recursiveUpdate (rust "worker") {
        bindsTo = [ portUnit ];
        after = [ portUnit "knoted-migrate.service" "knoted-intelligence.service" ];
        environment.KNOTED_INTELLIGENCE_BGE_M3_DIR = cfg.bgeM3Dir;
        serviceConfig.TimeoutStopSec = 75;
      };
      knoted-intelligence = {
        requires = [ "knoted-migrate.service" ];
        after = [ "knoted-migrate.service" ];
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
          PYTHONDONTWRITEBYTECODE = "1";
          HF_HOME = "${root}/cache/huggingface";
          TORCH_HOME = "${root}/cache/torch";
          MPLCONFIGDIR = "${root}/cache/matplotlib";
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
          Type = "notify";
          PrivatePIDs = true;
          PrivateNetwork = true;
          RestrictNamespaces = true;
          SystemCallArchitectures = "native";
          SystemCallFilter = "~connect io_uring_setup";
          SystemCallErrorNumber = "EPERM";
          RuntimeDirectory = "knoted-intelligence";
          RuntimeDirectoryMode = "0700";
          ExitType = "cgroup";
          # The interpreter is a child of the namespace init. Drain it and
          # its guardians before systemd applies its mixed-mode final kill.
          ExecStop = "${pkgs.python3}/bin/python3 ${./intelligence-stop.py} $MAINPID";
          KillMode = "mixed";
          TimeoutStopSec = 45;
          Restart = "always";
          RestartSec = 3;
          ExecStart = lib.escapeShellArgs [
            "${pkgs.python3}/bin/python3" "${./intelligence-init.py}"
            "${cfg.package}/share/knoted/intelligence/python/bin/python3.12"
            "-I" "-B" "${./intelligence-server.py}" proxySocket
          ];
          ReadWritePaths = map (d: "${root}/${d}") [ "python" "staging" "cache" "tmp" ];
          InaccessiblePaths = sandboxPaths;
        };
      };
    };
  };
}
