# Plan: Rust reimplementation of Civilization (1991, civ/CIV.EXE) for modern Linux

## Context

`civ/` holds MicroProse Civilization for DOS (CIV.EXE, 1996-dated files, MCGA 256-colour
assets: 110 `.PIC` images, 38 `.PAL` palettes, `FONTS.CV`, `MAP.PIC` Earth map, text files
BLURB0-4/KING/PRODUCE/HELP/INTRO/STORY/CREDITS/ERROR.TXT, sound drivers `*.CVL`, graphics
drivers `*GRAPHIC.EXE`). The task (taskciv.txt) is a Rust game with the same functionality
on modern Linux, using a modern toolkit (SDL3 chosen), rendering the original 320x200
graphics upscaled by an integer factor with no interpolation (nearest-neighbour).

Environment facts (verified):
- Host: openSUSE Tumbleweed-Slowroll, Rust 1.98.1 / cargo 1.98.1, cmake, gcc, ninja.
- `libSDL3.so.0` (3.4.x) runtime is installed on the host; no SDL3 dev package/pkg-config.
  The `sdl3` crate 0.20 fetches and compiles from crates.io fine (network OK); final link
  needs an unversioned `libSDL3.so` -> solved by a `build.rs` that creates a symlink in
  `OUT_DIR` pointing at `/usr/lib64/libSDL3.so.0` and emits `cargo:rustc-link-search`
  (fallback: the user can install `SDL3-devel` in the tumbleweed distrobox and build there).
- DOSBox available. Behaviour is reconstructed from the binary's data tables, the
  shipped text files, and documented Civ1 rules (manual + READ.ME corrections).

## Verified asset formats (from read-only analysis of civ/)

### PIC / PAL container (verified on all 110 PICs + 38 PALs)
Sequence of blocks to EOF: `tag[2] len:u16le payload[len]`.
- `E0` (len 258): first,last, one byte per VGA index (EGA dither map) -> ignore.
- `M0` (len 770): first,last, 256*RGB 6-bit (0..63). Scale `(c*255+31)/63`.
- `X0` = **256-colour** picture: `w:u16 h:u16 maxbits:u8(=11)` then LZW to end of block.
- `X1` = **16-colour** picture, same header; decoded rows are `ceil(w/2)` bytes, **low nibble = left pixel**.
- Empty picture block: len==4 (w=h=0, no maxbits) — how `.PAL` files end (`M0` + empty `X0`).
- Block orders seen: `E0,M0,X0` (most); `M0,X0` (CBACKS, KING03, SLAG2, SLAM2, SP257, TER257);
  `M0,X1` (KINK00, KINK03, MAP, TORCH); `X1` only, no palette (SPRITES).
- LZW: LSB-first bit reader, code width starts at 9, dictionary 0..255 + code 256 reserved
  (never emitted, but occupies a slot), next free = 257. Per code: entry = dict[code] or
  prev+prev[0] (KwKwK); output entry; if prev: push prev+entry[0]; prev=entry; then if
  dict.len >= 1<<width: width<maxbits ? width++ : (truncate to 257, width=9, prev=None).
  Stop when fewer than `width` bits remain in the block.
- RLE after LZW: byte 0x90 escape; `90 00` = literal 0x90; `90 N` = repeat last byte N-1 more times.
- Non-320x200 images: CBACKS1-3 320x65, CBRUSH0/2/4 182x95, CBRUSH1/3 136x95, CBRUSH5 135x95,
  SLAG2/SLAM2 197x67, TORCH 13x14 (X1, 7 bytes/row).
- Sprite sheets: TER257 = 16x16 terrain tiles, grid origin (0,0), 16 cols x 12 rows used.
  SP257 = 16x16 grid; units 16x16, 20 per row from y=160 (28 units: row 160 + 8 on row 176);
  terrain overlays/specials/icons rows 0..159. SPRITES.PIC = 16-colour variant (EGA) -> unused.
  Unit sprites use indices 10 (light) and 2 (dark) as civ-colour placeholders (to confirm).
- Palette (SP257.PAL): 0 black; 1..15 EGA-like civ colours (light 15,12,10,9,14,11,13,7 /
  dark 8,4,2,1,6,3,5); 16..31 grey ramp; 32..113 terrain colours; 128..223 magenta filler;
  224..254 ramps. TER257's palette differs at 17..31 (dark greys / browns).
- PAL usage: leader portraits KINGxx use indices 64..143, BACK* backgrounds use 144..255, so a
  diplomacy screen = BACK palette + KING palette merged. Index 0 black, 255 usually white.
- Game version string (INTRO.TXT): "CIVILIZATION (TM) - Version 474.01".

### FONTS.CV (verified, 9 fonts)
`u16 count(=9)`, `u16 off[count]` (offset of each font's BITMAP). For font at `o`:
`n = last-first+1`; `width[n]` at `o-8-n`; `first` at `o-8` (0x20); `last` at `o-7`;
`bytes_per_row` at `o-6` (1 or 2); `height` at `o-4`; bitmap at `o`, stored row-major across
the whole strip: `rows = height+1`, `stride = n*bpr`; glyph c row r = bytes
`[o + r*stride + (c-first)*bpr .. +bpr]`, big-endian, MSB = leftmost pixel. Glyph advance =
`width[c]` (can exceed cell). Fonts: 0 small bold 7px (default UI font), 1 tiny 5px,
2 serif 10px, 3 title 16px, 4 thin serif 16px, 5 bold serif 14px, 6 bold 8px, 7 sans 9px,
8 script 9px. `width[c]` is the ink width: the game draws one blank column after every glyph
(`GLYPH_SPACING` in src/assets/fonts.rs; e.g. menu-bar "GAME" in font 0 is 6+6+7+6+3 = 28 px in the captures).

### MAP.PIC (Earth)
`M0` (EGA palette) + `X1` 320x200; the map is the top-left 80x50 pixels, value = EGA colour =
terrain: 1 Ocean, 10 Grassland, 6 Plains, 2 Forest, 14 Desert, 15 Arctic, 9 River, 12 Hills,
7 Tundra, 13 Mountains, 11 Jungle, 3 Swamp. Rows 0-2 and 47-49 are ocean.
Other small PICs: TORCH 13x14 icon; SETTLERS.PIC = 4 frames 49x16 of a settlers wagon
(top-left 148x81); LOGO.PIC = "CIVILIZATION" lettering at x10-305,y64-137.

### Text files
BLURB0-4, KING, PRODUCE, HELP, ERROR: 528-byte hash index (8 lines of 64 chars + CRLF; ignore)
then sections `*NAME` ... until next `*`, `*END` terminator. Trailing `^` = hard line break;
`_text` lines = menu choices; tokens `$RPLC1 $RPLC2 $US $THEM $BUCKS`. CRLF, Latin-1.
BLURB0 = 69 advance descriptions (Civilopedia); BLURB1 = 45 improvements/wonders x2 (`X` long,
`X2` short rule note); BLURB2 = 28 units x2; BLURB4 = 21 terrain/govt/concept topics x2;
KING = 61 sections (NEWSA-N headlines, diplomacy EXCHANGE/CANCEL/TRIBUTE/TAKECIV/PEACE...,
QUIZ copy-protection, D1-D6 disasters, REV, UMAX, PMAX); PRODUCE = 51 build-advisor blurbs;
HELP = 18 sections; ERROR = 12 (ZOC, TRIREME, FUEL...). STORY.TXT = intro narration, `_`
separates screens. CREDITS.TXT = 24 lines role/name alternating, `%` = `.`.
INTRO.TXT = DOS launcher banner (graphics/sound mode selection) -> not needed.

### Rules data decoded from CIV.EXE (v474.01, EXEPACK-packed; offsets = unpacked image, DS base 0x2A800)
Transcribe these verbatim into `src/rules/*.rs` (static tables). Year from turn t:
t<=250: 20t-4000; <=300: 1000+10(t-250); <=350: 1500+5(t-300); <=400: 1750+2(t-350); else 1850+(t-400).

**Civs** (index, leader, adjective, plural if special; traits aggressive/expansionist/civilized):
1 Caesar/Roman (0,1,1); 2 Hammurabi/Babylonian (-1,-1,1); 3 Frederick/German/Germans (1,-1,1);
4 Ramesses/Egyptian (0,0,1); 5 Abe Lincoln/American (-1,0,1); 6 Alexander/Greek (0,1,-1);
7 M.Gandhi/Indian (-1,-1,0); 9 Stalin/Russian (1,0,-1); 10 Shaka/Zulu (1,0,0);
11 Napoleon/French/French (1,1,1); 12 Montezuma/Aztec (0,-1,1); 13 Mao Tse Tung/Chinese/Chinese (0,0,1);
14 Elizabeth I/English/English (0,1,0); 15 Genghis Khan/Mongol (1,1,-1). Index 0 = Barbarians
(Attila). Slots: player slot s (0..7) gets civ s or s+8 (two civs share a slot/colour).
Slot colours (main/secondary palette idx): 12/4 red (Barbarians), 15/7 white, 10/2 green, 9/1 blue,
14/10 yellow, 11/3 cyan, 13/4 magenta, 7/8 grey. Plural = adjective+"s" unless given.
Govt titles: Mr., Emperor, King, Comrade, President, President (female: Empress, Queen).
Govt names: Anarchy, Despotism, Monarchy, Communism, Republic, Democracy.
Commodities (trade): Silk, Silver, Wine, Copper, Gems, Dye, Salt, Spice.

**City names** (16 per civ, in civ order 1..7, 9..15; then a spare pool of 32):
Romans: Rome, Caesarea, Carthage, Nicopolis, Byzantium, Brundisium, Syracuse, Antioch, Palmyra, Cyrene, Gordion, Tyrus, Jerusalem, Seleucia, Ravenna, Artaxata
Babylonians: Babylon, Sumer, Uruk, Nineveh, Ashur, Ellipi, Akkad, Eridu, Kish, Nippur, Shuruppak, Zariqum, Izibia, Nimrud, Arbela, Zamua
Germans: Berlin, Leipzig, Hamburg, Bremen, Frankfurt, Bonn, Nuremberg, Cologne, Hannover, Munich, Stuttgart, Heidelburg, Salzburg, Konigsberg, Dortmund, Brandenburg
Egyptians: Thebes, Memphis, Oryx, Heliopolis, Gaza, Alexandria, Byblos, Cairo, Coptos, Edfu, Pithom, Busiris, Athribis, Mendes, Tanis, Abydos
Americans: Washington, New York, Boston, Philadelphia, Atlanta, Chicago, Buffalo, St.Louis, Detroit, New Orleans, Baltimore, Denver, Cincinnati, Dallas, Los Angeles, Las Vegas
Greeks: Athens, Sparta, Corinth, Delphi, Eretria, Pharsalos, Argos, Mycenae, Herakleia, Antioch, Ephesos, Rhodes, Knossos, Troy, Pergamon, Miletos
Indians: Delhi, Bombay, Madras, Bangalore, Calcutta, Lahore, Karachi, Kolhapur, Jaipur, Hyderabad, Bengal, Chittagong, Punjab, Dacca, Indus, Ganges
Russians: Moscow, Leningrad, Kiev, Minsk, Smolensk, Odessa, Sevastopol, Tblisi, Sverdlovsk, Yakutsk, Vladivostok, Novograd, Krasnoyarsk, Riga, Rostov, Astrakhan
Zulus: Zimbabwe, Ulundi, Bapedi, Hlobane, Isandhlwana, Intombe, Mpondo, Ngome, Swazi, Tugela, Umtata, Umfolozi, Ibabanago, Isipezi, Amatikulu, Zunguin
French: Paris, Orleans, Lyons, Tours, Chartres, Bordeaux, Rouen, Avignon, Marseilles, Grenoble, Dijon, Amiens, Cherbourg, Poitiers, Toulouse, Bayonne
Aztecs: Tenochtitlan, Chiauhtia, Chapultepec, Coatepec, Ayotzinco, Itzapalapa, Iztapam, Mitxcoac, Tacubaya, Tecamac, Tepezinco, Ticoman, Tlaxcala, Xaltocan, Xicalango, Zumpanco
Chinese: Peking, Shanghai, Canton, Nanking, Tsingtao, Hangchow, Tientsin, Tatung, Macao, Anyang, Shantung, Chinan, Kaifeng, Ningpo, Paoting, Yangchow
English: London, Coventry, Birmingham, Dover, Nottingham, York, Liverpool, Brighton, Oxford, Reading, Exeter, Cambridge, Hastings, Canterbury, Banbury, Newcastle
Mongols: Samarkand, Bokhara, Nishapur, Karakorum, Kashgar, Tabriz, Aleppo, Kabul, Ormuz, Basra, Khanbalyk, Khorasan, Shangtu, Kazan, Quinsay, Kerman
Spare: Mecca, Naples, Sidon, Tyre, Tarsus, Issus, Cunaxa, Cremona, Cannae, Capua, Turin, Genoa, Utica, Crete, Damascus, Verona, Salamis, Lisbon, Hamburg, Prague, Salzburg, Bergen, Venice, Milan, Ghent, Pisa, Cordoba, Seville, Dublin, Toronto, Melbourne, Sydney
Rating names (score screen "exceeds even that of"): Solomon the Wise, Emperor Augustus, King Charlemagne, Thomas Jefferson, Shogun Tokugawa, Franklin Roosevelt, Sulayman the Magnificent, Winston Churchill, Cleopatra, Charles De Gaulle, Vladimir Lenin, Simon Bolivar, Otto von Bismarck, Eric the Red, Kaiser Wilhelm, Louis the XVI, Neville Chamberlain, Ferdinand Marcos, Emperor Nero, Dan Quayle.
Historians: Herodotus, Pliny, Toynbee, Gibbon. Adjectives: Glorious, Great, Fine, Mediocre, Puny, Pathetic, Hopeless. Demographic categories: WEALTHIEST, MOST POWERFUL, MOST ADVANCED, HAPPIEST, LARGEST. Newspapers: Weekly, Today, Times, Tribune, News.

**Units** (idx name: obsolete_by, domain 0 land/1 air/2 sea, moves, range, att, def, cost(x10), sight, capacity, ai_role, prereq):
0 Settlers: -,0,1,0,0,1,4,0,0,settle,-
1 Militia: Gunpowder,0,1,0,1,1,1,0,0,defend,-
2 Phalanx: Gunpowder,0,1,0,1,2,2,0,0,defend,Bronze Working
3 Legion: Conscription,0,1,0,3,1,2,0,0,attack,Iron Working
4 Musketeers: Conscription,0,1,0,2,3,3,0,0,defend,Gunpowder
5 Riflemen: -,0,1,0,3,5,3,0,0,defend,Conscription
6 Cavalry: Conscription,0,2,0,2,1,2,0,0,attack,Horseback Riding
7 Knights: Automobile,0,2,0,4,2,4,0,0,attack,Chivalry
8 Catapult: Metallurgy,0,1,0,6,1,4,0,0,attack,Mathematics
9 Cannon: Robotics,0,1,0,8,1,4,0,0,attack,Metallurgy
10 Chariot: Chivalry,0,2,0,4,1,4,0,0,attack,The Wheel
11 Armor: -,0,3,0,10,5,8,0,0,attack,Automobile
12 Mech. Inf.: -,0,3,0,6,6,5,0,0,defend,Labor Union
13 Artillery: -,0,2,0,12,2,6,0,0,attack,Robotics
14 Fighter: -,1,10,1,4,2,6,2,0,air,Flight
15 Bomber: -,1,8,2,12,1,12,2,0,attack,Advanced Flight
16 Trireme: Navigation,2,3,0,1,0,4,0,2,transport,MapMaking
17 Sail: Magnetism,2,3,0,1,1,4,0,3,transport,Navigation
18 Frigate: Industrialization,2,3,0,2,2,4,0,4,transport,Magnetism
19 Ironclad: Combustion,2,4,0,4,4,6,0,0,seaattack,Steam Engine
20 Cruiser: -,2,6,0,6,6,8,3,0,seaattack,Combustion
21 Battleship: -,2,4,0,18,12,16,3,0,seaattack,Steel
22 Submarine: -,2,3,0,8,2,5,3,0,seaattack,Mass Production
23 Carrier: -,2,5,0,1,12,16,3,0(code: carries 8 air),seaattack,Advanced Flight
24 Transport: -,2,4,0,0,3,5,0,8,transport,Industrialization
25 Nuclear: -,1,16,1,99,0,16,0,0,attack,Rocketry
26 Diplomat: -,0,2,0,0,0,3,0,0,civilian,Writing
27 Caravan: -,0,1,0,0,1,5,0,0,civilian,Trade
Sprite index in SP257: unit i at x=(i%20)*16, y=160+(i/20)*16 (same order as above).

**Advances** (idx name: prereq1, prereq2; -1 none) — 72 entries, indices 67-70 placeholders, 71 Future Tech.:
0 Alphabet -,-; 1 Code of Laws 0; 2 Currency 17; 3 Atomic Theory 51,11; 4 Democracy 28,30;
5 Monarchy 25,1; 6 Astronomy 26,9; 7 MapMaking 0; 8 Navigation 7,6; 9 Mathematics 0,16;
10 Medicine 28,24; 11 Physics 9,8; 12 Engineering 33,45; 13 University 9,28; 14 Magnetism 8,11;
15 Electronics 12,54; 16 Masonry; 17 Bronze Working; 18 Iron Working 17; 19 Bridge Building 18,45;
20 Invention 12,30; 21 Computers 9,15; 22 Writing 0; 23 Steam Engine 11,20; 24 Trade 2,1;
25 Ceremonial Burial; 26 Mysticism 25; 27 Nuclear Fission 41,3; 28 Philosophy 26,30;
29 Religion 28,22; 30 Literacy 22,1; 31 Horseback Riding; 32 Feudalism 16,5; 33 The Wheel;
34 Gunpowder 20,18; 35 Industrialization 49,53; 36 Chemistry 13,10; 37 Combustion 55,56;
38 Flight 37,11; 39 Advanced Flight 38,54; 40 Space Flight 21,46; 41 Mass Production 58,47;
42 Pottery; 43 Communism 28,35; 44 The Republic 1,30; 45 Construction 16,2; 46 Rocketry 39,15;
47 The Corporation 53,35; 48 Metallurgy 34,13; 49 RailRoad 23,19; 50 Nuclear Power 27,15;
51 Theory of Gravity 6,13; 52 Steel 48,35; 53 Banking 24,44; 54 Electricity 48,14;
55 Refining 36,47; 56 Explosives 34,36; 57 SuperConductor 60,41; 58 Automobile 37,52;
59 Genetic Engineering 10,47; 60 Plastics 55,40; 61 Recycling 41,4; 62 Chivalry 32,31;
63 Robotics 60,21; 64 Conscription 44,56; 65 Labor Union 41,43; 66 Fusion Power 50,57;
71 Future Tech. AI weights (interest 2..8, civilized lean -2..2) per tech also exist (see agent
report; optional). Civilopedia descriptions: BLURB0 `*UPPERCASE NAME`.

**Improvements** (idx name: cost x10, upkeep, prereq): 1 Palace 20/5/Masonry(upkeep in table 5;
game treats palace upkeep 0); 2 Barracks 4/0/-; 3 Granary 6/1/Pottery; 4 Temple 4/1/Ceremonial Burial;
5 MarketPlace 8/1/Currency; 6 Library 8/1/Writing; 7 Courthouse 8/1/Code of Laws;
8 City Walls 12/2/Masonry; 9 Aqueduct 12/2/Construction; 10 Bank 12/3/Banking;
11 Cathedral 16/3/Religion; 12 University 16/3/University; 13 Mass Transit 16/4/Mass Production;
14 Colosseum 10/4/Construction; 15 Factory 20/4/Industrialization; 16 Mfg. Plant 32/6/Robotics;
17 SDI Defense 20/4/SuperConductor; 18 Recycling Cntr. 20/2/Recycling; 19 Power Plant 16/4/Refining;
20 Hydro Plant 24/4/Electronics; 21 Nuclear Plant 16/2/Nuclear Power; 22 SS Structural 8/0/Space Flight;
23 SS Component 16/0/Plastics; 24 SS Module 32/0/Robotics. (Barracks upkeep is 1 in-game after
Gunpowder/... per manual: use 0 as table says but manual says 1 -> use 1; Palace upkeep 0.)
**Wonders** (W# name: cost x10, prereq, obsolete by): 1 Pyramids 30 Masonry/Communism;
2 Hanging Gardens 30 Pottery/Invention; 3 Colossus 20 Bronze Working/Electricity;
4 Lighthouse 20 MapMaking/Magnetism; 5 Great Library 30 Literacy/University;
6 Oracle 30 Mysticism/Religion; 7 Great Wall 30 Masonry/Gunpowder; 8 Magellan's Expedition 40 Navigation;
9 Michelangelo's Chapel 30 Religion/Communism; 10 Copernicus' Observatory 30 Astronomy;
11 Shakespeare's Theatre 40 Medicine/Electronics; 12 Isaac Newton's College 40 Theory of Gravity/Nuclear Fission;
13 J.S.Bach's Cathedral 40 Religion; 14 Darwin's Voyage 30 RailRoad; 15 Hoover Dam 60 Electronics;
16 Women's Suffrage 60 Industrialization; 17 Manhattan Project 60 Nuclear Fission;
18 United Nations 60 Communism; 19 Apollo Program 60 Space Flight; 20 SETI Program 60 Computers;
21 Cure for Cancer 60 Genetic Engineering. BLURB1 has `*NAME` (history) and `*NAME2` (effect).

**Terrain** (name: move, defense(halves), food, shields, trade, earthcode | special: F,S,T):
Desert 1,2,0,1,0,14 | Oasis 3,1,0; Plains 1,2,1,1,0,6 | Horses 1,3,0; Grassland 1,2,2,1(shield tiles only),0,10;
Forest 2,3,1,2,0,2 | Game 3,2,0; Hills 2,4,1,0,0,12 | Coal 1,2,0; Mountains 3,6,0,1,0,13 | Gold 0,1,6;
Tundra 1,2,1,0,0,7 | Game 3,0,0; Arctic 2,2,0,0,0,15 | Seals 2,0,0; Swamp 2,3,1,0,0,3 | Oil 1,4,0;
Jungle 2,3,1,0,0,11 | Gems 1,0,4; Ocean 1,2,1,0,2,1 | Fish 3,0,2; River 1,3,2,1,1,9.
Improvement table (irrigate result/turns, mine result/turns; -2=+1 bonus, -4=+3, -1 n/a, >=0 = becomes terrain code):
Desert -2/5 -2/5; Plains -2/5 Forest/15; Grassland -2/5 Forest/10; Forest Plains/5 n/a; Hills -2/10 -4/10;
Mountains n/a -2/10; Tundra n/a n/a; Arctic n/a n/a; Swamp Grassland/15 Forest/15; Jungle Grassland/15 Forest/15;
Ocean n/a; River -2/5 n/a. Road turns 2 (manual), railroad 6 (manual); pollution clean 3? (manual).
City radius offsets (21 squares, "fat cross"): dx = 0,0,1,1,1,0,-1,-1,-1, 0,1,2,2,2,1,0,-1,-2,-2,-2,-1
dy = 0,-1,-1,0,1,1,1,0,-1, -2,-2,-1,0,1,2,2,2,1,0,-1,-2.
Difficulty names: Chieftain, Warlord, Prince, King, Emperor (short: Chief, Lord, Prince, King, Emperor).
Main menu: "Start a New Game / Load a Saved Game / EARTH / Customize World / View Hall of Fame".
Custom world: Land mass (Small/Normal/Large), Temperature (Cool/Temperate/Warm), Climate (Arid/Normal/Wet), Age (3 billion/4 billion/5 billion years).
Menu bar: GAME, ORDERS, ADVISORS, WORLD, CIVILOPEDIA (hotkey = first letter).
Spaceship parts: Structure(x3 sprites), Propulsion, Fuel, Habitation, Life Support, Solar Panel.

### Graphics driver (MGRAPHIC.EXE) and the pixel dissolve (verified in the disassembly)
CIV.EXE is EXEPACK-packed and uses MS C overlays (`int 3Fh`); `reference/tools/unexepack.py` produces the
load image + overlay file for `objdump -b binary -m i386 -M i8086,intel`. DOS loads MGRAPHIC.EXE as an
overlay; its header (code offset 0x24) lists 52 entry offsets which CIV.EXE patches into 7-byte far-jump
stubs at 0:0x76d+7*i, so driver entry i is called as `call 0:0x76f+7*i` with C arguments (page struct
pointers: ds:0xaa = screen, ds:0x19d4 = hidden page 1).
Entry 10 (driver 0x6b7, stub 0:0x7b5) = **dissolve(page, duration)**: a 16-bit Galois LFSR `s>>=1; if carry
s^=0xB400` starting at 1 visits 1..65535; values <= 64000 give pixel offset value-1 (first pixel 46079,
last 0), each copied from the hidden page to the screen. The delay loop is calibrated so 8800/duration
pixels are copied per 70 Hz retrace: duration 20 = 2.1 s, 10 = 1.0 s; 0 = full speed. Call sites:
title logo (galaxy -> black + logo, 20), palace (20), "destroyed by" screen (30), city-founding view
(10). The discovery screen is NOT dissolved by the original: picture copied, then its icon palette is
faded in over 30 ticks (`0:0x4aa`). civrs: `gfx::dissolve` (order + rate: 514 px per 60 Hz frame for
duration 20), `ui::reveal::Reveal` (old frame remapped to the new palette, then replaced in order,
content may animate meanwhile); used by the title (20), palace (20), city founding (10), score wall
(30), and, beyond the original, the discovery screen (20) and the SAD destroyed picture (30). Headless
dumps finish it instantly (`UiContext::instant`) unless `CIVRS_TRANSITIONS=1`. The full disassembly
can be regenerated into `reference/disasm/` (gitignored, see its README).

### Sound (*.CVL)
DOS overlay drivers (AdLib `ASOUND`, MT-32 `RSOUND`, Tandy `TSOUND`, PC speaker `ISOUND`,
none `NSOUND`): MZ executables with 11 far functions listed at image `+0x32` (1 Init,
2 PlayTune(id, level), 3 Close, 4 SoundWorker @60 Hz, 5 FastSoundWorker @300 Hz while
SoundWorker returned 1, 6 SoundTimer), data segment at image `+0x1E20`. The music is bytecode
in the data segment: 231 per-voice streams at DS `0x0050..0x6959` (two-byte `note, duration`
pairs, note 0 = rest, duration 0 = end; commands `FF n` loop, `FE n` outer loop, `FD` restart,
`FC n` instrument, `FB n` release, `FA n` pitch slide, `F9 n` volume, `F8 p d` fade,
`F7 n` detune, `F6` random poke, `F5 n` volume offset, `F4 n` pan, `F3 p d` pan slide),
57 OPL2 patches of 44 bytes at DS `0x695A`, F-number table at `0x73A8`, voice/operator maps
at `0x73C6`/`0x73D8`, RNG seed at `0x73FD` (`state = ror16(state + 0x9248, 3)`).
`PlayTune` dispatches through the jump table at cs:`0x1D0C`; handlers 3-44 just assign streams
to voices (`lea cx, stream; call start_voice_N`), IDs 5-18 index a 4-entry level table, 33
pauses the music for one of seven drum patterns on voice 8, 38/39/44 patch random bytes into
their streams. Full format write-up with the same findings: civ-clone/web-renderer PR #131
(`docs/asound-cvl-format.md`). Re-implemented in `src/audio/driver.rs`; the voice→stream
table there was read from the handler disassembly (`objdump -b binary -m i386 -M i8086,intel`
over the image after the 512-byte MZ header) and the register stream verified against the
PR's simulator of the real driver for all 45 IDs. Synthesis: `nuked-opl3` crate (LGPL-2.1,
pure Rust, bit-exact YMF262 in OPL2 mode), stereo right channel lags one sample as on the chip.

How CIV.EXE drives it (from `reference/disasm/`): the loader (overlay 0xcaf) opens `nsound.cvl`
with the first letter patched (a/i/t/r/n), installs the 11 entries as far-jump stubs at
`0:0xa2b + 7*k` (k = 0 Init, 1 PlayTune, 2 Close, 3 SoundWorker, 4 FastSoundWorker,
5 SoundTimer). SoundWorker is called from the 60 Hz tick routine `0:0x345` (every 7th tick
skipped on 70 Hz displays), FastSoundWorker from the INT 8 handler `0:0x1a6` while the PIT
divisor word `ds:0x4c != 1`. Every PlayTune goes through the wrapper at file `0x1eec`
(`0x1a8:0x46c`), which tests bit 4 of the options byte `ds:0x19c0` (the Sound option) and
skips the call when clear. 64 call sites; the anthem sites push level 3, all others one
argument. Civ → tune: `ds:0x1518 + 0x3a*civ` short, `+2` long (Barbarians and slot 8: 36).
Call sites (file offsets; o = `civ-overlays.asm`): intro title theme 3 (o 0xeb2, the intro
animation paces frames on `SoundTimer() >= 21*f/4`), menu choice fades at a 288-tick phrase
boundary after tick 5269 (0x1f61); story: fade then 4 (o 0xa701), lines paced by SoundTimer;
new game *INIT → long anthem (o 0x48ad), faded at start of play (0x24bf); ZOC → 37 (0x5b1a);
discovery → short anthem (0xbe14); famine 36 (0xd0e3); nuclear test 36 (0xe566); civil
disorder 36 (0x12641); democracy collapse 36 (0x127ce); WLTKD start 34 (0x12a49) / end 35
(0x12b5b); pollution 36 (0x12fe9); city captured → conqueror's short anthem (0x140d3);
AI wonder → owner's short anthem (0x3e9e); combat (0x19bc4): winning bomber 43, else human
owns the winner → 38/40, otherwise 39/41, "modern" = winner attack > 4 or defence > 2;
spaceship story 3 (o 0x1a37) then 34 at spacest.pic (o 0x1de4); destroyed/*ARCH 35 then
human's short anthem (o 0x20db/0x2301); Hall of Fame new entry 34 (o 0x2e3d); diplomacy:
war drums 33 for CANCEL/TRIBUTE/TAKECIV/PROVOKE/REJECT/RID/MOBILIZE (o 0x6c68..0x76b1),
emissary prompt short anthem (o 0x7afc), audience long anthem (o 0x7b77), fade at the end
(o 0x86b5); conquest plaque long anthem (o 0xd9c8); civ destroyed slam screen → long anthem
of the victim (o 0xdd5e) then human's over the score wall (o 0xe07d); palace long anthem
(o 0x16006); city view with the new item 44 (o 0x19fa1); nuke animation 42 (o 0x1dfb6);
retirement at the year limit long anthem (0x2b96); exit PlayTune(0) + Close (0x1c11).
Nearly every tune is followed by PlayTune(1) when its screen closes.

### Sprite-sheet map (SP257.PIC, 16x16 cells, origin (cx*16, cy*16); colour 0 = transparent)
Cells with a 1px L-shaped guide line (colour 3 or 9 on row 0/col 0) must mask that line.
| Content | Cells | Confidence |
|---|---|---|
| Units (28) | row y=160 cells 0..19, row y=176 cells 0..7; civ colour placeholders idx 10 light / 2 dark | verified |
| Roads 8 pieces N,NE,E,SE,S,SW,W,NW | y=48 cells 0..7 | verified |
| Railroads same order | y=96 cells 8..15 | verified |
| Rivers 16 variants mask N=1,E=2,S=4,W=8 | y=64 cells 0..15 (y=80 duplicate) | verified |
| Specials in terrain order Desert..Ocean | y=112 cells 0..10 (Grassland empty) | partly verified |
| Irrigation / Hut / Pollution / Mine / Fortress | y=32 cells 4 / 5 / 6 / 7 / 2 | visual, confirm at M1 |
| City sprites 9 sizes, no walls / walls | y=0 cells 10..18 / y=16 cells 10..18 (alt rows y=32, y=144) | probable |
| Citizens (city screen figures) | y=128 cells 0..4 | probable |
| Explosion 8 frames | y=96 cells 0..7 | probable |
| Fog edges N,E,S,W | y=128 cells 5..8 (index 5 black, 8 dark grey) | verified against captures |
| Fill patterns checker / stone | y=176 cell 18 / 19 | verified |
TER257.PIC: rows 0..9 (y=0..159) = 16 variants each of Desert, Plains, Grassland, Forest,
Hills, Mountains, Tundra, Arctic, Swamp, Jungle (variant = mask of orthogonal neighbours of the
SAME terrain, N=1 E=2 S=4 W=8); y=160..191 = ocean/coast 8x8 quadrant pieces (index to confirm
at M1 with the labelled sheet dump). Icon pages: ICONPG1..8 = 3x3 grid (~70x60 cells at
x=16/129/233, y=4/75/142) = 72 advance icons; ICONPGA..E = 2x3 (~150x55 at x=7/166,
y=3/65/124; E has 2 rows) = improvement/wonder/unit icons; ICONPGT1/2 = terrain icons.
**All sprite coordinates marked probable are confirmed at M1 by rendering labelled sheet dumps
to PNG and viewing them (Read tool on image) before the UI tracks depend on them.**

---

# Implementation plan

## Decisions
- Toolkit: **SDL3** via `sdl3` crate 0.20 (window + streaming RGBA texture, nearest-neighbour,
  integer scale auto-picked from display size, `--scale N` override, letterboxed on resize).
- Project: `/home/tester/git/reverse/civrs/` (single binary crate). Deps: `sdl3`, `serde`,
  `bincode 1.3`. `build.rs` symlinks `$OUT_DIR/libSDL3.so -> /usr/lib64/libSDL3.so.0` and emits
  `cargo:rustc-link-search=native=$OUT_DIR` (skipped if pkg-config finds sdl3).
- Assets read at runtime from the original dir: `--assets DIR`, else `$CIV_DIR`, else `./civ`,
  `../civ`, `/home/tester/git/reverse/civ`. No conversion step.
- Rendering: 320x200 8-bit indexed `Surface` + active `Palette` -> RGBA -> texture each frame, 60 Hz.
- Save games: own format (`CIVR` magic + version + bincode of `Game`), 10 slots in
  `~/.local/share/civrs/` (`--save-dir` override), autosave option; hall of fame `fame.dat`.
  Original `.SVE` compatibility is not attempted.
- Sound: AdLib driver re-implementation (`Audio::play(Sound)`, `national_theme(civ, long)`),
  OPL2 emulation, SDL3 audio stream; MT-32/Tandy/PC-speaker drivers not reproduced.
- Not reproduced: copy-protection QUIZ, DOS launcher, EGA/Tandy assets (SPRITES.PIC, KINK*, SP299).
- Headless/CI: `HeadlessBackend` + `--dump TARGET --out FILE.ppm` and `--ai-only --turns N`.
  Visual checks are done by converting PPM to PNG (python3 PIL) and viewing with the Read tool,
  or by running under Xvfb (`Xvfb :99` exists on host).

## Crate layout
```
civrs/
  Cargo.toml  build.rs
  src/main.rs                CLI parsing, App::run / headless dumps
  src/app.rs                 frame loop, ScreenStack driver, palette fades
  src/assets/{mod,pic,pal,fonts,text,earth}.rs
  src/gfx/{mod,surface,text,dissolve,panel,sprites}.rs   (dissolve = MGRAPHIC pixel dissolve order/rate)
  src/platform/{mod,sdl,input,headless}.rs
  src/rules/{mod,units,advances,improvements,wonders,terrain,civs,city_names,governments,difficulty,misc}.rs
  src/game/{mod,ids,rng,map,mapgen,unit,city,player,economy,happiness,science,movement,combat,
            orders,turn,diplomacy,barbarians,pollution,spaceship,score,events}.rs
  src/ai/{mod,city_ai,unit_ai,diplo_ai,tech_ai,path}.rs
  src/ui/{mod,screen,widgets,theme,intro,title,setup,mapview,sidebar,menubar,tilerender,cityscreen,reveal,
          dialogs,diplomacy_ui,advisors,civilopedia,palace,spaceship_ui,endgame,halloffame}.rs
  src/save/{mod,hall}.rs     src/audio/{mod,cvl,driver,engine}.rs (ASOUND.CVL driver + OPL2)     tests/
```
Conventions: i32 screen coords, u8 palette indices, all game structs `Clone+Serialize+Deserialize`,
`Game` owns only plain data (no asset refs), `unsafe` only in platform/sdl.rs, deterministic
xorshift `Rng` seeded from setup so AI-only runs are reproducible.

## Module interfaces (tracks program against these)
assets: `AssetDir::open(root)`, `read(name)` case-insensitive, `load_pic -> Pic{w,h,depth,pixels(8bpp expanded),palette:Option<Palette>}`,
`load_pal`, `load_fonts -> FontSet(Vec<Font>)` (`Font{first,last,height,glyphs:Vec<Glyph{advance,rows:Vec<u16>}>}`),
`load_text -> TextFile` (`section(name) -> Option<&[String]>`, `substitute(lines,&Vars)`, `split_menu` for `_` choices,
`parse_story`, `parse_credits`), `load_earth -> EarthMap{terrain: Vec<Terrain>}`.
`pic::lzw_decode(data,max_bits)`, `pic::rle_decode(data)`, `Palette::from_m0`, `merged(other, range)`, `lerp(to,t)`.
gfx: `Surface{w,h,px,clip}` with `fill_rect/hline/vline/line/rect_outline/blit/blit_part(key)/blit_remap(map)/fill_pattern/to_rgba/write_ppm`;
`text::draw_text(dst,fonts,&TextStyle{font,colour,shadow},x,y,s) -> end_x`, `_centered`, `_right`, `wrap`, `draw_paragraph`;
`panel::draw_panel(dst,sheet,rect,PanelStyle)`, `draw_bevel`, `draw_button`, `draw_progress_bar`;
`sprites::SpriteSheet{surf,pal}`, `sp257::{unit(t),road(d),rail(d),river(mask),special(t),IRRIGATION,MINE,POLLUTION,HUT,FORTRESS,city(size_class,walls),citizen(kind),explosion(f),fog_edge(d),PATTERN_*}`,
`ter257::{land(t,mask),coast_quadrant(q,mask),OCEAN}`, `civ_remap(slot) -> [u8;256]`, `icon_page_rect(idx) -> (&str, Rect)`.
platform: `trait Backend{poll()->Vec<Event>, present(&Surface,&Palette), wait_frame(), now_ms()}`; `SdlBackend::init(&PlatformOptions{scale,fullscreen,title})`,
`pick_scale(dw,dh)`; `Event::{Key{key,mods,down,repeat},Text(char),MouseMove{x,y},MouseButton{button,x,y,down},Quit}` with mouse already in 320x200 space;
`Key::{Up,Down,Left,Right,Home,End,PageUp,PageDown,Kp(u8),Enter,Esc,Space,Backspace,Tab,Delete,F(u8),Char(char),Plus,Minus}`; `Fader`.
rules: `UNITS:[UnitType;28]`, `ADVANCES:[Advance;72]`, `IMPROVEMENTS:[Improvement;25]`, `WONDERS:[Wonder;22]`, `TERRAIN:[TerrainInfo;12]`,
`CIVS:[Civ;16]`, `SPARE_CITY_NAMES`, `SLOT_COLOURS:[SlotColour;8]`, `GOVERNMENTS:[Government;6]`, `DIFFICULTY:[DifficultyInfo;5]`,
`CITY_RADIUS:[(i8,i8);21]`, `year_for_turn`, `year_text`, name lists (RATINGS, HISTORIANS, ADJECTIVES, COMMODITIES, NEWSPAPERS).
game: `Game{map,units,cities,players,turn,current,active_unit,rng,setup,options,wonders_built,...}` with
`new(GameSetup)`, queries (`units_at, city_at, unit, city, visible_to, next_active_unit`), orders
(`try_move(id,Dir8) -> Result<MoveOutcome,RuleError>`, `give_order`, `found_city`, `disband`, `set_goto`, `set_home`, `unload`,
`caravan_action`, `diplomat_action`), city mgmt (`set_production, buy, buy_cost, toggle_tile, set_specialist, auto_assign, sell_improvement, rename_city,
city_yield, city_happiness, available_production`), empire (`set_rates, set_research, researchable, revolution, choose_government, score, demographics`),
turn: `end_turn() -> TurnRunner` whose `step(&mut Game, &mut dyn AiDriver) -> TurnStep::{Event(GameEvent),Decision(Decision),AiMove{..},Combat{..},Done}`
is a resumable state machine so the UI can show dialogs/animations between steps; `RuleError` variants map 1:1 to ERROR.TXT sections.
ai: `trait AiDriver{play_turn(&mut Game, slot, sink)}`, `city_ai::choose_production`, `unit_ai::act/best_city_site`, `diplo_ai::{attitude,on_contact,respond}`,
`tech_ai::choose`, `barbarians::{spawn,play}`, `path::astar` (shared with human goto).
ui: `trait Screen{on_enter,handle(&Event)->Transition,update()->Transition,resume(DialogResult)->Transition,draw(&mut Surface),palette()}`;
`Transition::{None,Push,Pop(DialogResult),Replace,PopAll,Quit}`; widgets `MenuDialog`, `MessageBox`, `TextInput`, `ListBox`;
`UiContext{assets,fonts,sp,ter,texts,game:Option<Game>,ai,palette,fader,frame,mouse,pic_cache,hof,audio}`.
save: `save_game(path,&Game)`, `load_game(path)`, `list_slots()`, `HallOfFame{entries}`.

## Screen inventory (asset mapping)
Splash ADSCREEN -> Title (CBACK? bg + LOGO lettering keyed + SETTLERS.PIC wagon animation, "Press any key", N/L/E/C shortcuts)
-> Credits (CREDITS.TXT) -> Main menu (Start a New Game / Load a Saved Game / EARTH / Customize World / View Hall of Fame; TORCH cursor)
-> Difficulty (DIFFS.PIC, 5 levels) -> number of civs (3..7) -> pick tribe (14) -> leader name -> [Customize: CUSTOM.PIC + Land mass/Temperature/Climate/Age menus with HELP.TXT text]
-> Evolution story (BIRTH0..8 + STORY.TXT 8 screens, fades; world generated meanwhile) -> KING.TXT INIT message -> Map screen.
Map screen (320x200): menu bar (0,0,320,8) GAME/ORDERS/ADVISORS/WORLD/CIVILOPEDIA; minimap (0,8,80,50); civ status (0,58,80,36) population/year/gold+rates;
unit status (0,94,80,106); viewport (80,8,240,192) = 15x12 tiles. Tile draw order: terrain variant, river, special, irrigation/mine, roads, rails,
fortress, pollution, hut, city (civ-colour remap, name label font 1 shadowed), units (stack offset), fortify/sentry markers; unexplored black + fog edges.
City screen: title, citizens row, resource rows (food/shield/trade icons), 5x5 city map (click toggles worker), food storage box, improvements list (click = sell),
production box with Change/Buy, buttons Info/Happy/Map/View/Rename/Exit, supported/present units. City View = CBACKS1..3 skyline + CITYPIX1..3 + DOCKER.
Dialogs/pictures: production change (PRODUCE.TXT advisor), buy confirm, RIOT/RIOT2 disorder, LOVE1/2 celebration, DISCOVR1/2 + advance icon + research menu,
WONDERS/WONDERS2 crops + BLURB1 NAME2, GOVT{0..3}{A|M} on government change (A ancient / M modern after Invention), diplomacy (BACKgX + GOVTgX advisors by
size rank + KINGnn portrait with palette merge 64..143 + KING.TXT sections), palace (CBACK + CBACKS1..3 road + CASTLE0..4 sections + CBRUSH0..5 gardens, overlay 0x15f76), spaceship SPACEST (+PLANET1/2 arrival),
NUKE1 flash, INVADERS/2/3 barbarians, disasters D1..D6, advisors F1-F6 (City Status, Military, Intelligence, Attitude, Trade, Science), world F7-F12
(Wonders, Top 5 cities, Civ score, World map, Demographics, Spaceships), Civilopedia (icons + BLURB0/1/2/4), save/load slots, tax/lux rates, options,
end game (SAD destroyed; SLAM1/SLAM2/SLAG2 + ARCH score/retire; PLANET spaceship win), Hall of Fame (KING.TXT FAME), ERROR.TXT message boxes, HELP.TXT hints,
caravan menu (trade route / help wonder / keep moving), diplomat menu (embassy / investigate / sabotage / steal tech / incite revolt), unit stack list.
Food storage: the stippled box at the left of the dark FOOD STORAGE panel is the granary itself,
(size+1) icons wide on an 8-px pitch (squeezed when wider than the panel) and 10 rows high; a size-1
city's box is (3,115)-(19,196).
CIVILIZATION NOTE hints (Instant Advice option, src/game/hints.rs): HELP.TXT FIRSTMOVE (turn 0),
BUILDCITY (Settlers on a good site before the first city), FIRSTPRODUCT (after naming the first city,
before the founding picture), FIRSTCIV (before the first research menu), FIRSTUNIT1+2 (first military
unit built), MINING/IRRIGATE/ROAD (Settlers in a city radius, $US = city), MONARCHY, DISORDER; each
once per game (`Game::hints_shown`, appended to v2 saves). Green and red notes share one layout:
box (56,16)-(264,6n+26), black centred title, HELP/ERROR.TXT line breaks kept, text from x 64.

## Rules summary (formulas; [spec]=from EXE, [c1]=known Civ1 rule, [approx]=reconstruction kept in one tunable function)
- Mapgen [approx, structurally Civ1-like]: 80x50 no wrap; blobs of random-walk land scaled by land mass; terrain by latitude/temperature, moisture/climate,
  height (hills/mountains); age = smoothing passes; rivers from hills to ocean; special resource pattern `(x%4)*4+(y%4) == ((x/4)*13+(y/4)*11+seed)%16` [c1];
  shield grassland pattern; huts on land away from starts; continents by flood fill; start sites scored by radius yield, distance >= 10, distinct continents.
  Earth: MAP.PIC terrain + fixed historical start table (snap to land).
- Year/turn [spec]; end years 2100/2080/2060/2040/2020 by difficulty [READ.ME].
- Tile yield: TERRAIN/special [spec]; irrigation/mine bonuses [spec]; road +1 trade on Desert/Grassland/Plains; railroad +50% shields; city centre auto-irrigated
  + min 1 shield; Despotism/Anarchy -1 on tiles >2; Republic/Democracy +1 trade where >=1; pollution halves; Colossus +1 ocean trade.
- City turn: food surplus = food - 2*size - settlers upkeep (1; 2 Rep/Dem); growth at (size+1)*10, Granary keeps half, Aqueduct for >10; famine shrinks;
  shields minus unit upkeep beyond free units (3 under Anarchy/Despotism/Monarchy/Communism, 0 Rep/Dem); Factory/Mfg +50% each, plants/Hoover +50% with Factory;
  cost = cost*10 shields; change-class penalty 50%; buy cost units `2r + r^2/20` (x2 if empty), improvements `2r`, wonders `4r`; trade routes `(ta+tb+4)/8`
  (x2 foreign, max 3); corruption by distance to palace * govt factor (4/3/2/2(Communism flat)/1/0), Courthouse halves; rates in 10% steps capped per govt
  (60/70/80/80/100, Anarchy all lux); Marketplace/Bank +50% tax&lux, Library/University +50% sci, Copernicus/Newton +100% city, SETI +50% all;
  specialists 2 each; upkeep paid, sell improvement if broke.
- Happiness: content base 4..0 by difficulty; 2 lux per step; Temple 1 (2 with Mysticism, Oracle x2), Colosseum 3, Cathedral 3 (Michelangelo everywhere),
  Hanging Gardens/Cure for Cancer +1 content, Bach -2 unhappy, Shakespeare all content; martial law max 3 under Anarchy/Desp/Mon/Comm; Republic 1 / Democracy 2
  unhappy per aggressive unit abroad (Women's Suffrage -1); disorder if unhappy > happy; celebration size>=3, happy > half, no unhappy; Democracy 2 turns disorder -> anarchy.
- Science: cost `(n+1)*(8+2*difficulty)` beakers [approx]; research choice after each discovery; Future Tech after all; Great Library, Darwin's 2 advances.
- Movement in thirds, road 1/3, rail free, no river bonus (CIV.EXE 0x6336); partial moves (0x634a): a land unit with exactly 1 MP always enters, otherwise it fails when random(3*cost) > moves left; sight 1/2/3; ZOC rule [ERROR ZOC]; trireme 50% loss away from coast
  (Lighthouse safe); air fuel (Fighter 1 turn, Bomber 2, Nuclear 1); transports/carriers; orders Fortify/Sentry/Goto(A*)/Road 2/Rail 6/Irrigate/Mine per table/
  Fortress 3/Clean 3/Pillage order per READ.ME §3; irrigation needs water; settlers join city <10; found city not adjacent to city; Esc undo.
- Combat: `A = att*8*vet1.5`, `D = def*8*terrain/2*fortified1.5*fortress2*walls3(land attackers, Great Wall)*vet1.5`; loop rolling `rng.below(A)` vs `rng.below(D)`;
  winner 50% veteran; stack dies outside city/fortress; city shrinks on lost defence without walls; Fighters only vs air; ground units capture; capture plunders gold
  `gold*size/total`, steals an advance, 1/3 improvements destroyed, Palace destroyed; size-1 city destroyed; capital loss with >=6 cities -> civil war (ESCHISM);
  nukes (Manhattan) 3x3 kill + half size + pollution, SDI blocks.
- Pollution chance = f(shields beyond 20, size * tech factor) with Mass Transit/Recycling/plant modifiers; global warming when polluted tiles >= 8+4k (terrain shifts).
- Huts: gold 25/50/100, advance, mercenaries, barbarians (not for Settlers / near own city), empty [c1/approx].
- Disasters D1..D6 after turn 10 (1/200 per city size>=3), each cancelled by the matching improvement; off after Medicine/Industrialization [approx].
- Governments table (max rate / free units / unhappy per abroad unit / tile penalty / trade bonus / corruption / settler food / martial law):
  Anarchy lux-only/3/0/yes/no/4/1/3; Despotism 60/3/0/yes/no/3/1/3; Monarchy 70/3/0/no/no/2/1/3; Communism 80/3/0/no/no/2 flat/1/3; Republic 80/0/1/no/yes/1/2/0;
  Democracy 100/0/2/no/yes/0/2/0. Revolution: 1-4 turns anarchy (0 with Pyramids) -> choose govt menu -> GOVT screen; senate auto-accepts peace under Rep/Dem.
- Diplomacy (peace/war): contact on adjacency; AI audience on first contact and periodically by attitude; attitude = traits + treaty + embassies - war - power ratio +
  random; proposals EXCHANGE/CANCEL/TRIBUTE/TAKECIV/PEACE/GIVECIV/GIVECASH/GROVEL/THREAT/TREATY/HELPME/CRUSADE/... from KING.TXT; human audience menu
  (peace / military proposal / demand tribute); embassies reveal intel; UN forces peace acceptance; AI-AI relations rolled every 10 turns.
- Barbarians: huts + sea landings from turn 50 (Chieftain 150), `2+difficulty` era-appropriate units + leader (ransom 100 gold), attack nearest city, INVADER screens.
- Wonders: effects as Civ1 (Pyramids no anarchy, Hanging Gardens +1 content, Colossus, Lighthouse, Great Library, Oracle, Great Wall, Magellan +1 sea move,
  Michelangelo, Copernicus, Shakespeare, Newton, Bach, Darwin, Hoover, Women's Suffrage, Manhattan, UN, Apollo (reveal map, SS parts for all), SETI, Cure for Cancer);
  obsolete when owner learns advance; caravan help = 50 shields.
- Spaceship: after Apollo; structural <=39, component <=16, module <=12; launch when >=1/2/3 and structurals cover; flight turns `max(6, 100/(2+components/2))`;
  success from life support/habitation; capital capture loses it; arrival wins.
- Score: 2*happy + content + 20*wonders + 5*future tech + 3*peace turns (after 1 AD) - 10*polluted tiles + spaceship bonus; rating from RATINGS/ADJECTIVES;
  end on year limit / spaceship / conquest / destruction / retire; hall of fame.
- AI: production priority list (defenders, settlers by expansionist trait, Temple/Granary/Barracks/Library/Marketplace/Walls/Aqueduct/Courthouse by state,
  attackers in groups when at war, caravans/wonders in best city); unit roles (settle via best_city_site, fortify, group attack when A/(A+D) > 0.4, explore);
  tax 50/sci 50 with lux bumps; revolution to Monarchy/Republic; difficulty scales AI cost %, content, starting techs.

## Input model
Keyboard: numpad 1-9/arrows move (Home/PgUp/End/PgDn diagonals); Space skip; w wait; f fortify; s sentry; g goto; r road/rail; i irrigate; m mine;
F fortress; P pillage; p clean pollution; b build/join city; h home; u unload; D disband; c centre; Enter end turn; Esc cancel/undo; G/O/A/W/C menus;
F1-F12 reports; Tab next unit; Ctrl-S/Ctrl-L quick save/load; Ctrl-Q quit prompt. City screen: I H M V R Esc, Shift-M, Shift-A.
Mouse (320x200 space): LMB menu bar/items; LMB tile: own city -> city screen, own unit -> activate/stack list, else centre; RMB adjacent tile -> move;
goto LMB = destination; LMB minimap centres; LMB unit box cycles; dialogs LMB choose, RMB cancel; city map LMB toggles worker, citizen click cycles specialist.
Software cursor TORCH.PIC (hot spot top-left, corner colour transparent) drawn in 320x200 space; hardware cursor hidden.

## Build order, tracks and milestones
Tracks (parallel subagents after M1/M2 interfaces exist):
- T1 Core (assets, gfx, platform, app loop, headless dump) -> needed by T4, T5.
- T2 Rules data (static tables + tests) -> needed by all.
- T3 Engine (game/*, save/*) against T2 -> needed by T4, T5, T6.
- T4 Map-view UI (intro/title/setup, mapview, sidebar, menubar, tilerender, widgets, advisors, civilopedia, world reports, save/load UI).
- T5 City screen + dialogs (cityscreen, discovery/wonder/govt/riot/love/disasters/huts/errors, diplomacy UI, palace, spaceship, endgame, hall of fame).
- T6 AI + diplomacy + barbarians (after T3's order/combat API, testable headless).
Milestones: M0 skeleton builds/links, window at integer scale; M1 decoders + labelled sheet dumps (confirm sprite map by viewing PNGs) + decoder tests;
M2 rules tables + `Game::new` random/Earth worlds + `--dump map:SEED`; M3 playable map view (scroll, units, terraforming, cities, end turn, sidebar, menus);
M4 city screen, growth, production, happiness, discovery/research, governments; M5 combat/ZOC/huts/barbarians/naval/air/diplomats/caravans/wonders;
M6 AI civs + diplomacy screens + reports; M7 save/load, options, pollution, spaceship, score/end screens, hall of fame, civilopedia, intro/title/credits, palace;
M8 polish: fades, animations, tuning, release build.

## Verification
- `cargo test` (no SDL): all 110 PICs decode with expected dimensions and pixel counts (+ recorded checksums for SP257/TER257/LOGO/MAP), LZW/RLE unit vectors,
  38 PALs, 9 fonts with expected heights, text sections counts (BLURB0 69, ERROR 12, KING EXCHANGE has choices), Earth 80x50 with ocean rows 0-2/47-49,
  rules counts and acyclic tech tree, year_for_turn anchors, deterministic mapgen for a seed, movement/ZOC/combat-probability/yield/happiness fixtures,
  save/load round-trip equality, 200-turn AI-only games at every difficulty finish without panic.
- Headless: `civrs --dump pic:NAME|sheet:NAME|fonts|map:SEED|earth|screen:<name> --out f.ppm`, `--ai-only --turns N --seed S`; PPM -> PNG via python3 PIL,
  inspected with the Read tool. Interactive runs under `Xvfb :99` with `DISPLAY=:99` when needed.
- Manual checklist (M8): title/menus/shortcuts; new game flow with story + fades; Earth map recognisable; terrain joins/rivers/roads/cities/units/fog;
  every order key + ERROR messages; city screen fully interactive; techs/governments/revolution; combat incl. walls/fortress/veterans/capture/civil war;
  diplomacy audiences with correct backgrounds/portraits/advisor counts; barbarians; pollution/global warming; spaceship to arrival; save/load/autosave/hall
  of fame; civilopedia complete; `--scale 1..6`, resize letterboxing, fullscreen.
