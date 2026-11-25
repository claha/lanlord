{
  description = "lanlord — self-hosted LAN device inventory for homelabs";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    rec {
      lanlordPackages = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        pkgs.rustPlatform.buildRustPackage {
          pname = "lanlord";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ];
          meta = {
            description = "Self-hosted LAN device inventory for homelabs";
            license = pkgs.lib.licenses.mit;
            mainProgram = "lanlord";
          };
        });

      packages = forAllSystems (system: {
        default = lanlordPackages.${system};
        lanlord = lanlordPackages.${system};
      });

      nixosModules.default = import ./modules/lanlord.nix;

      nixosConfigurations.lanlord = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = { lanlordPackage = lanlordPackages.x86_64-linux; };
        modules = [
          ./modules/lanlord.nix
          ({ _, lanlordPackage, ... }: {
            config = {
              lanlord.enable = true;
              lanlord.interface = "lo";
              lanlord.package = lanlordPackage;
              lanlord.listenAddress = "127.0.0.1:8080";
              lanlord.openFirewall = false;

              # Dummy values so the configuration is evaluable as a
              # standalone system (not meant to actually boot).
              fileSystems."/" = { device = "/dev/sda1"; fsType = "ext4"; };
              boot.loader.grub.devices = [ "/dev/sda" ];
            };
          })
        ];
      };

      devShells = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              rust-analyzer
              cargo-watch
              pkg-config
              sqlite
              gh
              just
            ];

            env.RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";

            shellHook = ''
              echo "lanlord dev shell — $(rustc --version)"
            '';
          };
        });
    };
}
