{
  description = "Apple Calendar and Reminders CLI backed by EventKit";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "aarch64-darwin"
      ];
      mkPackage =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          pname = "icalctl";
          inherit ((lib.importTOML ./Cargo.toml).package) version;

          src = ./.;
          # Dependencies come from Cargo.lock, so there is no vendor hash to maintain.
          cargoLock.lockFile = ./Cargo.lock;
          # The commit this flake was built from, embedded only for the version report.
          ICALCTL_GIT_COMMIT = self.rev or self.dirtyRev or "unknown";

          meta = {
            description = "Apple Calendar and Reminders CLI backed by EventKit";
            homepage = "https://github.com/zydtiger/icalctl";
            license = lib.licenses.mit;
            platforms = systems;
            mainProgram = "icalctl";
          };
        };
    in
    {
      packages = lib.genAttrs systems (
        system:
        let
          icalctl = mkPackage nixpkgs.legacyPackages.${system};
        in
        {
          inherit icalctl;
          default = icalctl;
        }
      );
    };
}
