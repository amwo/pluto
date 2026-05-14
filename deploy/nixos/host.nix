{
  modulesPath,
  ...
}:

{
  imports = [
    "${modulesPath}/virtualisation/amazon-image.nix"
  ];

  ec2.hvm = true;

  networking.hostName = "pluto";

  time.timeZone = "UTC";

  nix.settings.experimental-features = [
    "nix-command"
    "flakes"
  ];

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      PermitRootLogin = "prohibit-password";
    };
  };

  users.users.root.openssh.authorizedKeys.keys = [
    # 運用者の ssh 公開鍵をここに追加 (deploy-rs が SSH で入る)
  ];

  sops = {
    defaultSopsFile = ../../secrets/pluto.yaml;
    age.sshKeyPaths = [ "/etc/ssh/ssh_host_ed25519_key" ];
  };

  services.pluto = {
    enable = true;
    secretsFile = ../../secrets/pluto.yaml;
  };

  system.stateVersion = "24.11";
}
