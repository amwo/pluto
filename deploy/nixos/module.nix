{
  config,
  lib,
  pkgs,
  inputs,
  ...
}:

let
  cfg = config.services.pluto;
in
{
  options.services.pluto = {
    enable = lib.mkEnableOption "pluto Solana copy bot";

    package = lib.mkOption {
      type = lib.types.package;
      default = inputs.self.packages.${pkgs.system}.pluto;
      description = "pluto binary package";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "pluto";
    };

    secretsFile = lib.mkOption {
      type = lib.types.path;
      description = "path to sops-managed env file decrypted into /run/secrets/pluto.env";
    };
  };

  config = lib.mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.user;
      home = "/var/lib/pluto";
      createHome = true;
    };
    users.groups.${cfg.user} = { };

    services.postgresql = {
      enable = true;
      package = pkgs.postgresql_18;
      ensureDatabases = [ "pluto" ];
      ensureUsers = [
        {
          name = cfg.user;
          ensureDBOwnership = true;
        }
      ];
    };

    sops.secrets."pluto.env" = {
      sopsFile = cfg.secretsFile;
      format = "dotenv";
      owner = cfg.user;
      mode = "0400";
    };

    systemd.services.pluto = {
      description = "Pluto Solana copy-trading bot";
      after = [
        "network-online.target"
        "postgresql.service"
        "sops-nix.service"
      ];
      wants = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "exec";
        User = cfg.user;
        Group = cfg.user;
        EnvironmentFile = config.sops.secrets."pluto.env".path;
        ExecStart = "${cfg.package}/bin/pluto run";
        Restart = "on-failure";
        RestartSec = "5s";
        TimeoutStopSec = "30s";
        KillSignal = "SIGINT";
        StateDirectory = "pluto";
        WorkingDirectory = "/var/lib/pluto";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        LimitNOFILE = 65536;
      };
    };
  };
}
