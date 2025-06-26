let
  # We specify sources via Niv: use "niv update nixpkgs" to update nixpkgs, for example.
  sources = import ./util/nix/sources.nix { };
  # However, if you want to override Niv's inputs, this will let you do that.
  # NOTE: update with niv hasn't been successful, pkgsCross.avr.libcCross attr missing

  # NOTE: override with nix-shell --arg arm false
in { pkgs ? import sources.nixpkgs { }, avr ? true, arm ? true, teensy ? true
}:
with pkgs;
let
  avrlibc = pkgsCross.avr.libcCross;

  avr_incflags = [
    "-isystem ${avrlibc}/avr/include"
    "-B${avrlibc}/avr/lib/avr5"
    "-L${avrlibc}/avr/lib/avr5"
    "-B${avrlibc}/avr/lib/avr35"
    "-L${avrlibc}/avr/lib/avr35"
    "-B${avrlibc}/avr/lib/avr51"
    "-L${avrlibc}/avr/lib/avr51"
  ];

in mkShell {
  name = "qmk-firmware";

  buildInputs =
    [ clang-tools_11 dfu-programmer dfu-util diffutils git niv bear ]
    ++ lib.optional avr [
      pkgsCross.avr.buildPackages.binutils
      pkgsCross.avr.buildPackages.gcc8
      avrlibc
      avrdude
    ] ++ lib.optional arm [ gcc-arm-embedded ]
    ++ lib.optional teensy [ teensy-loader-cli ];

  AVR_CFLAGS = lib.optional avr avr_incflags;
  AVR_ASFLAGS = lib.optional avr avr_incflags;
  shellHook = ''
    # Prevent the avr-gcc wrapper from picking up host GCC flags
    # like -iframework, which is problematic on Darwin
    unset NIX_CFLAGS_COMPILE_FOR_TARGET
  '';
}
