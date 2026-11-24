# NixOS module for lanlord — a hardened LAN device inventory service.
#
# Example:
#
#   nixosConfigurations.mine = lib.nixosSystem {
#     system = "x86_64-linux";
#     modules = [ inputs.lanlord.nixosModules.default ];
#     config.lanlord.enable = true;
#     config.lanlord.interface = "eth0";
#     config.lanlord.package = inputs.lanlord.packages.x86_64-linux.default;
#   };
#
{ config, pkgs, lib, ... }:

let
  cfg = config.lanlord;

  # The TOML config is rendered into the Nix store (read-only, reproducible)
  # and handed to the service via `--config`. The live inventory database
  # lives in the managed state directory instead.
  lanlordConfig = pkgs.writeText "lanlord.toml" (lib.concatStringsSep "\n" [
    "interface = \"${cfg.interface}\""
    (lib.optionalString (cfg.subnet != null) "subnet = \"${cfg.subnet}\"")
    "scan_interval = ${toString cfg.scanInterval}"
    "listen = \"${cfg.listenAddress}\""
    "database = \"${cfg.stateDir}/lanlord.db\""
  ]) + "\n";

  lanlordPort = builtins.stringToNumber (builtins.last (builtins.splitString ":" cfg.listenAddress));
in
{
  options.lanlord = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable the lanlord service.";
    };

    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      description = ''
        The lanlord package to run.

        When consuming this module as a flake input, set this to the flake's
        package output, e.g.

            config.lanlord.package = inputs.lanlord.packages.${pkgs.system}.default;

        This module does not pin a package itself so it stays usable whether
        or not lanlord is packaged in nixpkgs.
      '';
    };

    interface = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = ''
        Network interface to scan for ARP, e.g. "eth0".

        Required when `lanlord.enable` is true. The scanned IPv4 segment is
        derived from this interface's address unless `lanlord.subnet` is set.
      '';
    };

    subnet = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Optional IPv4 segment to scan, e.g. "10.0.0.0/8".

        Defaults to the segment derived from `lanlord.interface`.
      '';
    };

    scanInterval = lib.mkOption {
      type = lib.types.int;
      default = 300;
      description = ''
        Delay in seconds between scans. The first scan runs at startup.
      '';
    };

    listenAddress = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:8080";
      description = ''
        Address the web UI listens on, e.g. "127.0.0.1:8080" or "0.0.0.0:8080".

        Choose a loopback address to keep the UI reachable only from the host
        itself (or a reverse proxy), as lanlord has no authentication.
      '';
    };

    stateDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/lanlord";
      description = ''
        Directory holding the SQLite inventory database.

        The service's `StateDirectory` is set to the base of this path, so
        systemd creates and chowns it for the `lanlord` service user.
      '';
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Open the host firewall for the `lanlord.listenAddress` port.

        Disabled by default: enabling the service does not silently change
        the host's exposure. Set to true to allow LAN access to the web UI.
      '';
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkAssert (cfg.interface != null && cfg.interface != "")
      "lanlord.options.interface is required when lanlord.enable is true"
      {
        # Dedicated system user and group for the service.
        users.groups.lanlord = { };

        users.users.lanlord = {
          isSystemUser = true;
          group = "lanlord";
          createHome = false;
          description = "lanlord service account";
        };

        systemd.services.lanlord = {
          description = "lanlord — LAN device inventory";
          after = [ "network-online.target" ];
          wants = [ "network-online.target" ];

          serviceConfig = {
            Type = "simple";
            ExecStart = "${if cfg.package == null then throw "lanlord.options.package must be set when lanlord.enable is true" else cfg.package}/bin/lanlord --config ${lanlordConfig}";

            # Run as the dedicated service user.
            User = "lanlord";
            Group = "lanlord";

            # Managed state directory: systemd creates it and chowns it to the
            # service user; the SQLite database lives there.
            StateDirectory = "lanlord";

            # `CAP_NET_RAW` is the only added capability: raw sockets for ARP.
            CapabilityBoundingSet = [ "CAP_NET_RAW" ];
            AmbientCapabilities = [ "CAP_NET_RAW" ];

            # Sandbox: no new privileges, private /tmp, read-only system, no /home.
            NoNewPrivileges = true;
            PrivateTmp = true;
            ProtectSystem = "strict";
            ProtectHome = true;

            # AF_PACKET (ARP) must stay available, so do not privatise the
            # network namespace; restrict the address families instead.
            RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_NETLINK" "AF_PACKET" ];
            RestrictNamespaces = true;

            # Only the state directory is writable.
            ReadWritePaths = [ cfg.stateDir ];

            # Log to the journal.
            StandardOutput = "journal";
            StandardError = "journal";
            SyslogIdentifier = "lanlord";

            # Further hardening.
            ProtectClock = true;
            ProtectKernelTunables = true;
            ProtectKernelModules = true;
            ProtectControlGroups = true;
            RestrictRealtime = true;
            RestrictSUIDSGID = true;
            LockPersonality = true;
          };
        };

        # Only open the firewall when the operator asks for it.
        networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ lanlordPort ];
        networking.firewall.allowedUDPPorts = lib.mkIf cfg.openFirewall [ lanlordPort ];
      }
  );
}
