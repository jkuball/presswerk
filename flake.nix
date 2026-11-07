{
  description = "Presswerk release tooling";

  nixConfig = {
    extra-substituters = [ "https://jkuball.cachix.org" ];
    extra-trusted-public-keys = [ "jkuball.cachix.org-1:+9CHwNss+Andi72M3ZruI40RLYvYkyQAcT0ou4P50m0=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    presswerk-release.url = "git+ssh://git@github.com/jkuball/presswerk?ref=refs/tags/v0.1.0";

    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";

    import-tree.url = "github:denful/import-tree";

    nix-unit.url = "github:nix-community/nix-unit";
    nix-unit.inputs.nixpkgs.follows = "nixpkgs";

    rust-flake.url = "github:juspay/rust-flake";
    rust-flake.inputs.nixpkgs.follows = "nixpkgs";

    devshell.url = "github:numtide/devshell";
    devshell.inputs.nixpkgs.follows = "nixpkgs";

    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";

    git-hooks.url = "github:cachix/git-hooks.nix";
    git-hooks.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs@{ flake-parts, import-tree, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } ({ config, ... }: {
      imports = [
        inputs.flake-parts.flakeModules.modules
        inputs.nix-unit.modules.flake.default
        inputs.devshell.flakeModule
        inputs.git-hooks.flakeModule
        inputs.rust-flake.flakeModules.default
        inputs.rust-flake.flakeModules.nixpkgs
        inputs.treefmt-nix.flakeModule
        (import-tree ./modules)
      ];

      flake.presswerkModules = config.flake.modules.presswerk;

      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];
    });
}
