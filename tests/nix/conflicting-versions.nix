let
  presswerk = builtins.getFlake (toString ../..);
in
(presswerk.lib.mkRelease {
  sourceRoot = ../fixtures/release;
  versionFiles.cargo.first.cargoToml = "nested/Cargo.toml";
  versionFiles.cargo.second.cargoToml = "nested/Other.toml";
}).version
