# civrs — Civilization (1991) for modern Linux

A Rust reimplementation of MicroProse's *Sid Meier's Civilization* (DOS, version 474.xx)
that reads the **original game files** at runtime and renders the original 320x200,
256-colour graphics through SDL3, upscaled by an integer factor with no interpolation.

Nothing from the original is converted or bundled: the `.PIC` pictures (LZW + RLE),
`.PAL` palettes, `FONTS.CV` bitmap fonts, `MAP.PIC` (Earth), and the `.TXT` dialogue and
Civilopedia texts are decoded by the Rust code when the game starts. The rules tables
(units, advances, improvements, wonders, terrain, civilisations, city names) were extracted
from `CIV.EXE` and transcribed into `src/rules/`.

## Building

Requirements: Rust 1.80+ (`cargo`), the SDL3 runtime library (`libSDL3.so.0`), a C linker.
The build script locates `libSDL3.so.0` in the usual library directories and creates the
unversioned link the linker needs, so the SDL3 development package is optional.

```sh
cd civrs
cargo build --release
```

If your distribution does not ship SDL3, build it from source or install `SDL3-devel`
(e.g. inside the tumbleweed distrobox: `zypper in SDL3-devel`) and set `SDL3_LIB_DIR`.

## Running

```sh
./target/release/civrs                      # looks for the game files in ./civ, ../civ, $CIV_DIR
./target/release/civrs --assets /path/to/civ
./target/release/civrs --scale 4            # fixed integer scale (default: largest that fits)
./target/release/civrs --fullscreen
./target/release/civrs --skip-intro         # straight to a new game on the map
```

Alt+Enter toggles full screen. The window can be resized; the picture always stays an
integer multiple of 320x200 with nearest-neighbour scaling, letterboxed in black.

Saved games (10 slots, plus autosave in slot 9) and the Hall of Fame are written to
`$XDG_DATA_HOME/civrs` (default `~/.local/share/civrs`); `--save-dir DIR` overrides it.

## Controls

Keyboard follows the original: numeric keypad / arrows move the active unit
(Home/PgUp/End/PgDn for diagonals), `Space` no orders, `w` wait, `f` fortify, `s` sentry,
`g` go to, `b` found city / join city, `r` road / railroad, `i` irrigate, `m` mine,
`Shift+F` fortress, `Shift+P` pillage, `p` clean pollution, `h` home city, `u` unload,
`Shift+D` disband, `c` centre, `Return` end of turn, `Esc` cancel, `F1`–`F12` the advisor
and world reports, `Alt+G/O/A/W/C` the menu bar. Mouse: left click selects units, opens
cities and chooses menu items; right click on an adjacent tile moves the active unit.

## Headless tools

```sh
civrs --dump pic:KING03 --out k.ppm           # decode a picture
civrs --dump sheet:SP257 --out s.ppm          # sprite sheet with a labelled 16px grid
civrs --dump fonts --out f.ppm                # font sampler
civrs --dump map:42 --out m.ppm --zoom 2      # overview of a generated world (seed 42)
civrs --dump earth --out e.ppm
civrs --dump screen:map:7:30 --out scr.ppm    # the map screen after 30 AI turns, seed 7
civrs --dump sound:3 --out theme.wav          # render sound ID 3 (main theme) to a 48 kHz WAV
civrs --dump sound:10:3:60 --out r.wav        # ID 10 (Roman theme), arrangement level 3, 60 s
civrs --dump sound-regs:37 --out w.txt        # OPL2 register log (sub-tick, reg, value)
```

## Layout

- `src/assets/`   decoders for PIC/PAL, FONTS.CV, text files, MAP.PIC
- `src/gfx/`      320x200 indexed surface, text rendering
- `src/platform/` SDL3 window/input (and a headless backend)
- `src/rules/`    static game data from CIV.EXE
- `src/game/`     world generation, units, cities, economy, happiness, science, combat,
                  diplomacy, barbarians, pollution, spaceship, turn processing, score
- `src/ai/`       computer players and pathfinding
- `src/ui/`       all screens (title, setup, map, city, reports, Civilopedia, events)
- `src/save/`     saved games and the Hall of Fame
- `src/audio/`    the AdLib sound driver (`ASOUND.CVL`) re-implemented, played through an OPL2 emulator

## Sound

The original has no music files: every tune and effect is bytecode compiled into the
sound-driver overlays. `src/audio/driver.rs` re-implements the AdLib driver `ASOUND.CVL`
(the v474 one, ID string `RLND Cvlzatn12-03-91`): it reads the music streams and FM
instruments from the player's own driver file and reproduces the original's OPL2 register
writes tick for tick (verified byte for byte against an emulation of the real driver for all
45 sound IDs). The register stream feeds `nuked-opl3`, a bit-exact YMF262 emulator in its
OPL2-compatible mode, and SDL3 plays the result; the clock is the game's 300 Hz timer
interrupt (`SoundWorker` every fifth tick, `FastSoundWorker` while noise effects run).

Sound IDs, triggered where CIV.EXE triggers them (its `PlayTune` call sites are listed in
`reference/DESIGN.md`): 3 main theme (intro, spaceship arrival), 4 world creation, 5–18 /
19–32 full and short national anthems (new game, palace, diplomacy audience, conquest and
retirement plaques, score wall / advance learned, another civ's wonder, city captured,
emissary), 33 war drums (demands, threats, war), 34/35 fanfares (space-race victory,
civilization destroyed, We Love the King Day starting/ending), 36 civil disorder, famine,
pollution, Manhattan Project, barbarians, 37 zone-of-control violation, 38–41 battles (the
"won" pair when the human's unit wins, explosion variants when the winner has attack above 4
or defence above 2), 42 nuclear strike, 43 a bomber winning, 44 city finished a build. The
in-game Sound option (menu Game → Options) gates every call, as in CIV.EXE. Other drivers
(Roland MT-32, Tandy, PC speaker) and MicroProse's 1994 patched drivers are not supported;
with no usable `ASOUND.CVL` the game runs silent.

## Reference material

`reference/DESIGN.md` documents the decoded file formats, the rules tables extracted from
`CIV.EXE` and the design of this reimplementation. `reference/original-screens/` holds
320x200 captures of the original game (made with dosbox-staging, see `REPORT.md` there)
that the screens were measured against.
