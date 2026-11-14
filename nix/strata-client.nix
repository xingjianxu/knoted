# Knoted local provider: the declarative contract for the pre-installed Strata
# server. Knoted never installs or downloads Strata or model files.
#
# Host task (a separate /etc/nixos task, never this repository) owns:
#   /data/llm/strata/bin/strata                  installed Strata 0.2.13.1 CLI
#   /data/llm/strata/models/IQ3_S/               the pinned GGUF set (model.files)
#   /data/llm/strata/config/iq3_s.json           the run config `strata serve` reads
#   /data/llm/strata/state/                      writable Strata state (writable by user knoted)
#   /run/knoted/strata-api-key                   the API key, root:knoted 0440
#
# This module declares paths, GPU UUIDs, the systemd wrapper and the client
# contract only. It deliberately does not fetch binaries and never writes a key
# into the Nix store. The tmpfiles entry below manages the runtime directory
# permissions, not the secret itself.
#
# Why `strata serve` and not `strata launch`: `launch --no-tui` prints the
# address and returns while the server keeps running, so systemd would see the
# main process exit. `strata serve --config <run config>` stays in the
# foreground, keeps one engine child, and accepts --host/--port. The API key is
# read from STRATA_API_KEY, which the unit's wrapper copies out of the systemd
# credential at exec time (never on the command line, never in the store).
#
# Upstream facts this contract was read from (read-only, not vendored):
#   Strata PRODUCT_VERSION 0.2.13.1, checkout commit 13eedf5 (2026-11-11),
#   docs/INSTALL.md and docs/MODELS.md. The release installer is not pinned to a
#   commit, so the host task must record the installed build in
#   docs/plans/knoted-v1/evidence/t12-deployment.md and compare it here.
{ config, lib, pkgs, ... }:

let
  cfg = config.knoted.strata;

  # GPU UUIDs, not nvidia-smi indexes. CUDA_VISIBLE_DEVICES accepts UUIDs and
  # renumbers them to 0..n-1 inside the unit, so Strata's own `--gpu`/`gpu` run
  # config field stays correct after a card is added, removed or swapped.
  visibleDevices = lib.concatStringsSep "," cfg.gpus;

  shellList = args: lib.concatMapStringsSep " " (arg: lib.escapeShellArg arg) args;

  preflight = pkgs.writeShellScript "knoted-strata-preflight" ''
    set -eu
    strata_bin=${lib.escapeShellArg cfg.package}
    runconfig=${lib.escapeShellArg cfg.runConfig}
    modeldir=${lib.escapeShellArg cfg.model.path}
    keyfile=${lib.escapeShellArg cfg.apiKeyFile}

    if [ ! -x "$strata_bin" ]; then
      echo "knoted-strata: Strata CLI is not executable: $strata_bin" >&2
      exit 1
    fi
    for path in "$runconfig" "$modeldir" "$keyfile"; do
      if [ ! -e "$path" ]; then
        echo "knoted-strata: missing host-installed path: $path" >&2
        exit 1
      fi
    done
    if [ ! -r "$keyfile" ]; then
      echo "knoted-strata: API key is not readable by the service user: $keyfile" >&2
      exit 1
    fi
    if [ -z "''${CUDA_VISIBLE_DEVICES:-}" ]; then
      echo "knoted-strata: CUDA_VISIBLE_DEVICES is empty; refusing to use the default cards" >&2
      exit 1
    fi
    if [ ! -d "$modeldir" ] || [ -z "$(${pkgs.coreutils}/bin/ls -A "$modeldir")" ]; then
      echo "knoted-strata: model directory is missing or empty: $modeldir" >&2
      exit 1
    fi
  '';

  serve = pkgs.writeShellScript "knoted-strata-serve" ''
    set -eu
    key_file="$CREDENTIALS_DIRECTORY/${cfg.credentialName}"
    if [ ! -r "$key_file" ]; then
      echo "knoted-strata: systemd credential is missing: $key_file" >&2
      exit 1
    fi
    STRATA_API_KEY="$(${pkgs.coreutils}/bin/cat "$key_file")"
    export STRATA_API_KEY
    exec ${lib.escapeShellArg cfg.package} serve \
      --config ${lib.escapeShellArg cfg.runConfig} \
      --host ${lib.escapeShellArg cfg.listen.address} \
      --port ${toString cfg.listen.port} \
      --offline \
      ${shellList cfg.extraArgs}
  '';

  fileType = lib.types.submodule {
    options = {
      name = lib.mkOption {
        type = lib.types.str;
        description = "File name inside model.path, exactly as the upstream repository names it.";
      };
      sizeBytes = lib.mkOption {
        type = lib.types.ints.positive;
        description = "Expected size in bytes (upstream pin).";
      };
      sha256 = lib.mkOption {
        type = lib.types.strMatching "[0-9a-f]{64}";
        description = "Expected SHA-256 (upstream pin).";
      };
      role = lib.mkOption {
        type = lib.types.enum [ "weights-shard-1" "ngram-table" "vision-projector" ];
        description = "What the file is for.";
      };
    };
  };
in
{
  options.knoted.strata = {
    enable = lib.mkEnableOption "the Knoted local Strata provider (external pre-installed server)";

    package = lib.mkOption {
      type = lib.types.str;
      default = "${cfg.dataDir}/bin/strata";
      description = ''
        Absolute path to the host-installed Strata CLI. A string, not a Nix
        path, so the external install is never copied into the Nix store.
      '';
    };

    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "/data/llm/strata";
      description = "Host-installed Strata root (program, models, engine components).";
    };

    stateDir = lib.mkOption {
      type = lib.types.str;
      default = "${cfg.dataDir}/state";
      description = "Writable Strata state directory (STRATA_HOME); created by the host.";
    };

    runConfig = lib.mkOption {
      type = lib.types.str;
      default = "${cfg.dataDir}/config/iq3_s.json";
      description = ''
        Run config JSON produced by the host install (`strata launch --save-preset`
        or setup.py). It must set `model_name` to `model.alias`; the host verifies
        that with GET /v1/models in evidence t12_2.
      '';
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "knoted";
      description = "Service user; must exist and own stateDir and the runtime key file.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "knoted";
      description = "Service group; owns the runtime key file.";
    };

    listen = {
      address = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        description = ''
          Loopback only. The Knoted plan fixes 127.0.0.1:15604; an assertion
          below refuses any other address so the model is never exposed.
        '';
      };
      port = lib.mkOption {
        type = lib.types.port;
        default = 15604;
        description = "Fixed by the Knoted plan; changing it requires updating the provider contract.";
      };
    };

    gpus = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [
        "GPU-6bc31459-053f-60ef-0615-3aec9593c9e7"
        "GPU-9e2b6db5-acef-fa85-a28a-7214d1f4c6c0"
      ];
      example = [ "GPU-00000000-0000-0000-0000-000000000000" ];
      description = ''
        GPU UUIDs from `nvidia-smi --query-gpu=uuid --format=csv,noheader`
        on wks-lab. Never permanent nvidia-smi indexes. Two RTX 3090s are the
        default Strata slot; the Python service owns another card by UUID.
        The order is the order Strata sees them (0..n-1 after remapping).
      '';
    };

    credentialName = lib.mkOption {
      type = lib.types.str;
      default = "strata-api-key";
      description = "systemd LoadCredential id; also the file name inside CREDENTIALS_DIRECTORY.";
    };

    apiKeyFile = lib.mkOption {
      type = lib.types.str;
      default = "/run/knoted/strata-api-key";
      description = ''
        Runtime credential source for LoadCredential. Must be root:knoted 0440
        and is created by the host (a secret, never declared here). Knoted's API
        process reads the same path to authenticate as a client.
      '';
    };

    manageRuntimeDir = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Create /run/knoted with 0750 root:knoted; the key file itself is host-managed.";
    };

    model = {
      alias = lib.mkOption {
        type = lib.types.str;
        default = "qwen3.8-flash-next-iq3_s";
        description = ''
          The model id the server reports in /v1/models and chat responses
          (Strata's `model_name`). Knoted sends this exact value.
        '';
      };
      family = lib.mkOption {
        type = lib.types.str;
        default = "qwen";
        description = "Strata model family (`--family qwen`).";
      };
      quant = lib.mkOption {
        type = lib.types.str;
        default = "IQ3_S";
        description = "Quantization size (`--model IQ3_S`); 3.5-bit i-quant, the best quality tier.";
      };
      description = lib.mkOption {
        type = lib.types.str;
        default = "Qwen3.8-Flash-Next IQ3_S (ISTA-DASLab GSQ-RCO), ~54.8 GB weights + 28.8 GB n-gram table";
        description = "Human-readable identity recorded with the deployment evidence.";
      };
      path = lib.mkOption {
        type = lib.types.str;
        default = "${cfg.dataDir}/models/IQ3_S";
        description = "Host-installed directory holding the pinned GGUF files below.";
      };
      upstreamRepo = lib.mkOption {
        type = lib.types.str;
        default = "ISTA-DASLab/Qwen3.8-Flash-Next-GSQ-RCO-GGUF";
        description = "Hugging Face repository of the pinned model files.";
      };
      upstreamRevision = lib.mkOption {
        type = lib.types.str;
        default = "ed59f92082b1e93c0e96d60a8b11aab089b52f09";
        description = "Pinned repository revision (upstream setup.py HF_REVISIONS, 2026-09-29).";
      };
      files = lib.mkOption {
        type = lib.types.listOf fileType;
        default = [
          {
            name = "Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf";
            sizeBytes = 54817524224;
            sha256 = "4c1eb2ceb4915e1192f4f386021897bde56a97f40a0bb78bb86465e0f7d2aca3";
            role = "weights-shard-1";
          }
          {
            name = "Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00002-of-00002.gguf";
            sizeBytes = 28800138432;
            sha256 = "316b46f3a2dbd68c900f43136ab9449f9dcc3725dfd8c794847c204bc161e113";
            role = "ngram-table";
          }
        ];
        description = ''
          Expected files, sizes and SHA-256 from the upstream pin. The host task
          verifies them with sha256sum after install; until then the evidence
          table keeps them marked 未验证. Knoted only uses text, so the optional
          mmproj vision projector is not part of this contract.
        '';
      };
    };

    upstream = {
      productVersion = lib.mkOption {
        type = lib.types.str;
        default = "0.2.13.1";
        description = "Strata PRODUCT_VERSION this contract was read from.";
      };
      inspectedCommit = lib.mkOption {
        type = lib.types.str;
        default = "13eedf552417a6a791a7d13c6597e24300ee3a09";
        description = ''
          Upstream commit of the read-only checkout used for the interface probe.
          The release installer is not commit-pinned; the host records the
          installed build and the evidence compares it with this value.
        '';
      };
    };

    credentialFile = lib.mkOption {
      type = lib.types.str;
      default = cfg.apiKeyFile;
      readOnly = true;
      description = "Path Knoted's API/worker uses as the client credential file.";
    };

    provider = {
      baseUrl = lib.mkOption {
        type = lib.types.str;
        default = "http://${cfg.listen.address}:${toString cfg.listen.port}";
        readOnly = true;
        description = "OpenAI-compatible base URL handed to Knoted (no /v1 suffix in configuration).";
      };
      model = lib.mkOption {
        type = lib.types.str;
        default = cfg.model.alias;
        readOnly = true;
        description = "Model id sent in requests and expected in /v1/models.";
      };
      credentialFile = lib.mkOption {
        type = lib.types.str;
        default = cfg.apiKeyFile;
        readOnly = true;
        description = "Credential file the client reads at runtime.";
      };
    };

    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Additional `strata serve` flags; never put a key here (argv is world-readable).";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.listen.address == "127.0.0.1";
        message = "knoted.strata: the provider must listen on 127.0.0.1 only (Knoted plan fixes 127.0.0.1:15604).";
      }
      {
        assertion = cfg.gpus != [ ];
        message = "knoted.strata: declare at least one GPU UUID; the engine must not fall back to default cards.";
      }
      {
        assertion = builtins.all (uuid: builtins.match "GPU-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}" uuid != null) cfg.gpus;
        message = "knoted.strata.gpus: every entry must be a GPU UUID from nvidia-smi (GPU-…), not a numeric index.";
      }
      {
        assertion = lib.hasPrefix "/" cfg.package && lib.hasPrefix "/" cfg.runConfig
          && lib.hasPrefix "/" cfg.model.path && lib.hasPrefix "/" cfg.apiKeyFile;
        message = "knoted.strata: package, runConfig, model.path and apiKeyFile must be absolute host paths.";
      }
      {
        assertion = cfg.extraArgs == [ ] || !(builtins.any (arg: lib.hasPrefix "--api-key" arg) cfg.extraArgs);
        message = "knoted.strata.extraArgs: never pass an API key on the command line; use apiKeyFile.";
      }
    ];

    systemd.tmpfiles.rules = lib.optionals cfg.manageRuntimeDir [
      "d /run/knoted 0750 root ${cfg.group} -"
    ];

    systemd.services.knoted-strata = {
      description = "Strata local provider for Knoted (loopback OpenAI-compatible endpoint)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      environment = {
        CUDA_VISIBLE_DEVICES = visibleDevices;
        STRATA_HOME = cfg.stateDir;
        HOME = cfg.stateDir;
      };

      serviceConfig = {
        Type = "simple";
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.stateDir;
        ExecStartPre = preflight;
        ExecStart = serve;
        # The key is copied from apiKeyFile into the unit's private credential
        # directory by systemd; the wrapper exports it as STRATA_API_KEY.
        LoadCredential = "${cfg.credentialName}:${cfg.apiKeyFile}";
        Restart = "on-failure";
        RestartSec = 15;
        TimeoutStopSec = 300;
        # `strata serve` owns the engine as a child; stop the whole group so a
        # cancelled/stopped unit cannot leave the engine holding a GPU.
        KillMode = "control-group";
      };
    };
  };
}
