{
  description = "Pluto - Solana bots";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    inputs@{ flake-parts, rust-overlay, ... }:
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
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
        in
        {
          _module.args.pkgs = pkgs;

          devShells.default = pkgs.mkShell {
            packages = with pkgs; [
              gnumake
              jq
              nodejs_24
              postgresql_18
              ripgrep
              rust-bin.nightly.latest.default
              sops
            ];

            env = {
              CHAINSTACK_GRPC_ENDPOINT = "yellowstone-solana-mainnet.core.chainstack.com";
              CHAINSTACK_GRPC_TOKEN = "";
              CHAINSTACK_HTTPS_ENDPOINT = "https://solana-mainnet.core.chainstack.com";
              SOLANA_RPC_URL = "https://api.mainnet-beta.solana.com";
              SOLANA_WALLET_ADDRESS = "BGBSb742YrjGLkDXdBL2TKRmeX3ocvueSAp9XmNjqPMe";
              TARGET_WALLET = "2FJiJVTHrLjqnReWVhNmSv5mPwd5a9bUTkJXcpaE7xrb";
              PLUTO_MODE = "observe";
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
    };
}
