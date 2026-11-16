# Host owns database provisioning/migrations, runtime configuration, TLS and
# proxying. This module starts only the implemented API.
{ config, lib, ... }:
let
  cfg = config.services.knoted;
in {
  options.services.knoted = {
    enable = lib.mkEnableOption "Knoted API";
    package = lib.mkOption {
      type = lib.types.package;
      description = "Verified fixed-revision Knoted artifact package.";
    };
    configFile = lib.mkOption {
      type = lib.types.str;
      description = "Absolute runtime TOML path; secrets must stay outside the store.";
    };
    user = lib.mkOption { type = lib.types.str; default = "knoted"; };
    group = lib.mkOption { type = lib.types.str; default = "knoted"; };
  };
  config = lib.mkIf cfg.enable {
    assertions = [{
      assertion = lib.hasPrefix "/" cfg.configFile;
      message = "services.knoted.configFile must be absolute";
    }];
    systemd.services.knoted-api = {
      description = "Knoted API";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${cfg.package}/bin/knoted-api --config ${lib.escapeShellArg cfg.configFile}";
        Restart = "on-failure";
        RestartSec = 3;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        UMask = "0077";
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
      };
    };
  };
}
