{ pkgs, config, lib, ... }:

with lib;

let
  cfg = config.services.ble-ws-central;
in
{
  options = {
    services.ble-ws-central = {
      enable = mkEnableOption "Enable ble-ws-central";
      user = mkOption {
        type = types.str;
        default = "ble-ws-central";
        description = ''
          User that runs ble-ws-central
        '';
      };
      group = mkOption {
        type = types.str;
        default = "ble-ws-central";
        description = ''
          Group that runs ble-ws-central
        '';
      };
      port = mkOption {
        type = types.port;
        default = 5841;
        description = ''
          Port for grpc
        '';
      };
      dataDir = mkOption {
        type = types.str;
        default = "/var/lib/ble-ws-central";
        description = ''
          Directory where ble-ws-central stores its data
        '';
      };
      # TODO: mqtt settings
    };
  };

  config = mkIf cfg.enable {
    systemd.tmpfiles.rules = [
      "d ${cfg.dataDir} 700 ${cfg.user} ${cfg.group}"
      "Z ${cfg.dataDir} 700 ${cfg.user} ${cfg.group}"
    ];
    environment.systemPackages =
      let
        policyPkg = pkgs.stdenv.mkDerivation {
          pname = "ble-ws-central-policy";
          dontUnpack = true;
          src = ''
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE busconfig PUBLIC
            "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
            "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
            <busconfig>
              <policy user="${cfg.user}">
                <allow own="li._5kw.BleWsCentral"/>
              </policy>
            </busconfig>
          '';
          version = "0.1";
          installPhase = ''
            mkdir -p "$out/share/dbus-1/system.d"
            echo "$src" > "$out/share/dbus-1/system.d/li._5kw.BleWsCentral.conf"
          '';
        };
      in
      [ policyPkg ];
    users.groups.${cfg.group}.name = cfg.group;
    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
    };
    hardware.bluetooth = {
      enable = true;
      powerOnBoot = false;
    };
    systemd.services.bluetooth-mesh.aliases = [ "dbus-org.bluez.mesh.service" ];
    systemd.services.ble-ws-central = {
      enable = true;
      wants = [ "bluetooth-mesh.service" ];
      after = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];
      environment = {
        PORT = toString cfg.port;
        DATA_DIR = cfg.dataDir;
        HOST = "0.0.0.0";
      };
      serviceConfig = {
        User = cfg.user;
        Group = cfg.group;
        ExecStart = "${pkgs.ble-ws-central}/bin/ble-ws-central";
        Type = "simple";
      };
      # TODO: security/confinement
    };
  };
}
