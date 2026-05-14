{
  description = "Pluto - Solana bots";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay.url = "github:oxalica/rust-overlay";
    deploy-rs = {
      url = "github:serokell/deploy-rs";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    sops-nix = {
      url = "github:Mic92/sops-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      flake-parts,
      rust-overlay,
      deploy-rs,
      sops-nix,
      ...
    }:
    let
      mkPkgs =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
          config.allowUnfreePredicate =
            pkg: builtins.elem (nixpkgs.lib.getName pkg) [ "terraform" ];
        };
      mkPluto =
        pkgs:
        let
          rust = pkgs.rust-bin.nightly.latest.default;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust;
            rustc = rust;
          };
        in
        rustPlatform.buildRustPackage {
          pname = "pluto";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ openssl ];
          doCheck = false;
        };
    in
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      perSystem =
        { system, ... }:
        let
          pkgs = mkPkgs system;
        in
        {
          _module.args.pkgs = pkgs;

          packages.pluto = mkPluto pkgs;
          packages.default = mkPluto pkgs;

          devShells.default = pkgs.mkShell {
            packages = with pkgs; [
              deploy-rs.packages.${system}.default
              gnumake
              jq
              nodejs_24
              postgresql_18
              ripgrep
              rust-bin.nightly.latest.default
              sops
              terraform
            ];

            env = {
              CHAINSTACK_GRPC_ENDPOINT = "yellowstone-solana-mainnet.core.chainstack.com";
              CHAINSTACK_GRPC_TOKEN = "";
              CHAINSTACK_HTTPS_ENDPOINT = "https://solana-mainnet.core.chainstack.com";
              SOLANA_RPC_URL = "https://api.mainnet-beta.solana.com";
              SOLANA_WALLET_ADDRESS = "BGBSb742YrjGLkDXdBL2TKRmeX3ocvueSAp9XmNjqPMe";
              TARGET_WALLET = "2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb";
              PLUTO_MODE = "dry";
              PGHOST = "/data/pluto";
              PGPORT = "5435";
              PGUSER = "pluto";
              PGDATABASE = "pluto";
              DATABASE_URL = "postgres://pluto@localhost:5435/pluto?host=/data/pluto";
            };

            shellHook = ''
              if [ -f secrets/pluto.yaml ]; then
                if _pluto_secrets=$(sops -d --output-type dotenv secrets/pluto.yaml 2>/dev/null); then
                  set -a
                  eval "$_pluto_secrets"
                  set +a
                  unset _pluto_secrets
                  echo "[sops] secrets/pluto.yaml decrypted"
                else
                  echo "[sops] WARNING: could not decrypt secrets/pluto.yaml (missing age key?)"
                fi
              fi
              echo "[pg] start: pg_ctl -D \"$PGHOST\" -l \"$PGHOST/postgres.log\" -o \"-k $PGHOST -p $PGPORT\" start"
              echo "[pg] psql:  psql"
              echo "🐶"
            '';
          };
        };

      flake = {
        nixosModules.pluto = import ./deploy/nixos/module.nix;

        nixosConfigurations.pluto = nixpkgs.lib.nixosSystem {
          system = "x86_64-linux";
          specialArgs = { inherit inputs; };
          modules = [
            sops-nix.nixosModules.sops
            self.nixosModules.pluto
            ./deploy/nixos/host.nix
          ];
        };

        deploy.nodes.pluto = {
          hostname = "pluto.aws";
          profiles.system = {
            user = "root";
            sshUser = "root";
            path = deploy-rs.lib.x86_64-linux.activate.nixos self.nixosConfigurations.pluto;
          };
        };

        checks = builtins.mapAttrs (_: lib: lib.deployChecks self.deploy) deploy-rs.lib;
      };
    };
}
