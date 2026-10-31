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
23 Carrier: -,2,5,0,1,12,16,3,0(code: any number of air units, 0x6a53),seaattack,Advanced Flight
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
23 SS Component 16/0/Plastics; 24 SS Module 32/0/Robotics. (Barracks: table 0, CIV.EXE 0x13084
adds 1 each for Prince+, Emperor, Gunpowder, Combustion; Palace free.)
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
Ocean n/a; River -2/5 n/a. Road turns 2x move cost, railroad 4x move cost (0x5346); pollution clean 4 (0x50db).
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
new game *INIT → long anthem (o 0x48ad), faded at start of play (0x24bf); ZOC → 37 for a human unit, the *ZOC note only with Instant Advice (0x5b1a);
discovery → short anthem (0xbe14); famine 36 (0xd0e3); nuclear test 36 (0xe566); civil
disorder 36 (0x12641); democracy collapse 36 (0x127ce); WLTKD start 34 (0x12a49) / end 35
(0x12b5b); meltdown 36 after the explosion's 42 (0x12fe9; pollution itself is silent); city captured → conqueror's short anthem (0x140d3);
wonder given to a computer at the end of the turn → owner's short anthem (0x3e9e; a wonder from its own production is silent); combat (0x19bc4): winning bomber 43, else human
owns the winner → 38/40, otherwise 39/41, "modern" = winner attack > 4 or defence > 2;
spaceship story 3 (o 0x1a37) then 34 at spacest.pic (o 0x1de4); destroyed/*ARCH 35 then
human's short anthem (o 0x20db/0x2301); rating screen 34 (o 0x2e3d, faded at its key o 0x3102); diplomacy:
war drums 33 for CANCEL/TRIBUTE/TAKECIV/PROVOKE/REJECT/RID/MOBILIZE (o 0x6c68..0x76b1),
emissary prompt short anthem (o 0x7afc), audience long anthem (o 0x7b77), fade at the end
(o 0x86b5); conquest plaque long anthem (o 0xd9c8); civ destroyed slam screen → long anthem
of the victim (o 0xdd5e) then human's over the score wall (o 0xe07d); palace long anthem
(o 0x16006); city view with the new item 44 (o 0x19fa1); nuke animation 42 (o 0x1dfb6);
the year limit's "dynasty ends" newspaper long anthem (0x2b96; a retirement and a conquest's newspaper are silent); exit PlayTune(0) + Close (0x1c11).
Nearly every tune is followed by PlayTune(1) when its screen closes.

### Sprite-sheet map (SP257.PIC, 16x16 cells, origin (cx*16, cy*16); colour 0 = transparent)
Cells with a 1px L-shaped guide line (colour 3 or 9 on row 0/col 0) must mask that line.
| Content | Cells | Confidence |
|---|---|---|
| Units (28) | row y=160 cells 0..19, row y=176 cells 0..7, drawn as the 15x15 inside (cell + 1); idx 10 light / 2 dark, 15 -> 11 for the white civ, 7 -> 3 for the grey one (o 0x5a9c) | verified |
| Roads 8 pieces N,NE,E,SE,S,SW,W,NW | y=48 cells 0..7 | verified |
| Railroads same order | y=96 cells 8..15 | verified |
| Rivers 16 variants mask N=1,E=2,S=4,W=8 | y=64 cells 0..15 (y=80 duplicate) | verified |
| Specials in terrain order Desert..Ocean | y=112 cells 0..10 (Grassland empty) | partly verified |
| Irrigation / Mine / map Pollution / city-screen smog | y=32 cells 4 / 5 / 6 / 3 (map icons 0..6 = y=32 cells, o 0x59d7) | verified |
| Hut / Fortress / fortified, walls / city texture | 15x15 from (16i+1, 113), i = 15 / 14 / 13 / 12 | verified |
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
  `~/.local/share/civrs/` (`--save-dir` override), AutoSave option (CIV.EXE's every 50 turns, here into slots 4..8 (CIV.EXE: 4..9), plus civrs's silent save of every turn to slot 9); hall of fame `fame.dat`.
  The original's CIVILn.SVE + CIVILn.MAP import into a `Game` (`save::sve`, overlay 11 layout;
  the .MAP is map page 2 as a PIC): `civrs --load-sve FILE` (also with `--dump screen:NAME`),
  and the load menu offers CIVILn.SVE (save or game directory) in slots without a civrs save.
  Original `.SVE` compatibility is not attempted.
- Sound: AdLib driver re-implementation (`Audio::play(Sound)`, `national_theme(civ, long)`),
  OPL2 emulation, SDL3 audio stream; MT-32/Tandy/PC-speaker drivers not reproduced.
- Not reproduced: copy-protection QUIZ, DOS launcher, EGA/Tandy assets (SPRITES.PIC, KINK*, SP299).
  The quiz (overlay 16, from game_turn 0x2783): on every 50th turn while the year is BC and
  ds:0x4cd8 bit 0 (quiz passed, saved at SVE 0x9172, cleared by every load o 0xf0ce) is clear,
  "CIVILIZATION QUIZ" asks for the two prerequisites of a random advance (two tries, *QUIZ /
  *QUIZ2); a right answer sets the bit, a second wrong one (*INCORRECT) deletes every unit of
  the human (0x7e6:0xf0c with ds:0x20f4 = 1). civrs plays as if it were always answered right:
  the bit has no other reader, so nothing else changes. The hardware setup menu (overlay 1,
  INTRO.TXT: graphics, sound, mouse or keyboard) is replaced by the command line
  (`--keyboard-only`, sound on).
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
The runner follows game_turn's civ loop (0x2412): per civ in slot order its cities (0xa5e2, 0x250d),
politics (0x252b), then units (0x25af); after civ 7 the barbarian raids and world events, then turn + 1. It starts
after the human's units: civs after the human (turn T), world, new turn, barbarians and civs before the human
(turn T + 1), the human's cities and politics, then `Done` for the human to move. A new game first runs
`TurnRunner::opening` (`Game::opening`, not saved) so the civs before the human move at turn 0 as well.
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
caravans (0x5b66: in a foreign city the route is set up at once; in an own city "Will you?" Keep moving / Establish trade route / Help build WONDER. only when it builds a wonder, is 10+ squares from home or on another continent; trade greyed for a wonder city under 10), revenue (0x146be) `(d+10)(ta+tb)/24`, /2 same continent, /2 own civ, -1/3 each for Railroad, Flight of the target's owner, to gold and beakers; route kept in the home city only (3 slots, weakest replaced if stronger), diplomat menu (embassy / investigate / sabotage / steal tech / incite revolt), unit stack list.
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
- Mapgen [approx, structurally Civ1-like]: 80x50, wrapping east-west (0x1ec91; the map view scrolls across the edge); blobs of random-walk land scaled by land mass; terrain by latitude/temperature, moisture/climate,
  height (hills/mountains); age = smoothing passes; rivers from hills to ocean; special resource pattern `(x%4)*4+(y%4) == ((x/4)*13+(y/4)*11+seed)%16` [c1];
  shield grassland pattern; huts on land away from starts; continents by flood fill; start sites scored by radius yield, distance >= 10, distinct continents.
  Earth: MAP.PIC terrain + fixed historical start table (snap to land).
- Year/turn [spec]; end years 2100/2080/2060/2040/2020 by difficulty [READ.ME].
- Tile yield: TERRAIN/special [spec]; irrigation/mine bonuses [spec]; road +1 trade on Desert/Grassland/Plains; railroad +50% food/shields/trade after Colossus, before govt (0x134fc); city centre auto-irrigated
  (no minimum shield: tile_yield 0x133c2 has no city-square case); Despotism/Anarchy -1 on tiles >2; Republic/Democracy +1 trade where >=1; pollution halves, rounding up (0x1357d); Colossus +1 on squares with trade, before the government rules.
- City turn: food surplus = food - 2*size - settlers upkeep (1; 2 Rep/Dem); growth at (size+1)*10, Granary keeps half, Aqueduct for >10; famine shrinks;
  shields minus unit upkeep (0xfc6e: 1 per unit; under Anarchy/Despotism units up to city size free; Diplomats/Caravans free); Factory +50% / Mfg. Plant +100%, plants/Hoover (same continent) +50% with either, counted in quarters (0xdc5d);
  cost = cost*10 shields; no penalty for changing production (none in CIV.EXE); buy cost units `2r + r^2/20` (x2 if empty), improvements `2r`, wonders `4r`; trade routes `(ta+tb+4)/8`
  (x2 foreign, max 3); corruption by distance to palace * govt factor (4/3/2/2(Communism flat)/1/0), Courthouse halves; rates in 10% steps capped per govt
  (60/70/80/80/100, Anarchy all lux); Marketplace, then Bank +50% tax&lux each (compounding), Library/University +50% base sci (+1/3 each with Newton, all cities), Copernicus x2 city, SETI +50% all (0xf2f2);
  specialists 2 each; upkeep paid by the human only, not under Anarchy, building by building after taxes; one the treasury cannot pay is sold for its cost ("can't maintain", 0x13033). Build menu (0xacd7): no Courthouse with Palace, no Factory with Mfg. Plant,
  Bank needs Marketplace, University needs Library, Power Plant needs Factory, no plant with a Hoover Dam on the continent.
- Happiness: content base 4..0 by difficulty; 2 lux per step; Temple 1 with Ceremonial Burial (2 with Mysticism, Oracle x2), Colosseum 3, Cathedral 4 with Religion (6 with Michelangelo),
  martial law max 3 under Anarchy/Desp/Mon/Comm; Republic 1 / Democracy 2
  unhappy per aggressive unit abroad (Women's Suffrage -1); disorder if unhappy > happy; celebration size>=3, happy > half, no unhappy; Democracy 2 turns disorder -> anarchy;
  then wonders: Hanging Gardens/Cure for Cancer +1 happy, Shakespeare all content, Bach -2 unhappy on its continent.
- Science: cost `(n+1)*(8+2*difficulty)` beakers [approx]; research choice after each discovery; Future Tech after all; Great Library, Darwin's 2 advances.
- Movement in thirds, road 1/3, rail free, no river bonus (CIV.EXE 0x6336); partial moves (0x634a): a land unit with exactly 1 MP always enters, otherwise it fails when random(3*cost) > moves left; sight 1, or 2 for units with table sight flag 2 (ships: the outer ring only over ocean, 0x8061); ZOC rule [ERROR ZOC]; trireme 50% loss away from coast (human only, 0x75da);
  air fuel (Fighter 1 turn, Bomber 2, Nuclear 1); transports/carriers; orders Fortify/Sentry/Goto(A*); Settlers work counted per turn incl. the
  order's turn (unit +5, 0x4492 repeats the key): road 2x/rail 4x move cost, irrigate/mine per table, fortress move cost+4 (0x522a), clean 4;
  pillage at once (0x4c80); irrigation water 0x77fd (city squares never count); settlers join city <10 (no Aqueduct exception); cities may be adjacent; Esc undo.
- Combat: `A = att*8*vet1.5`, `D = def*8*terrain/2*fortified1.5*fortress2*walls: def*8*terrain/2*3 instead of fortification, not vs attack-12 units (Artillery, Bomber) nor for air defenders (0x198ca)*vet1.5`; loop rolling `rng.below(A)` vs `rng.below(D)`;
  winner 50% veteran; stack dies outside city/fortress; city shrinks on lost defence without walls; Fighters only vs air; ground units capture; capture plunders gold
  `gold*size/total` (also from a destroyed city), steals an advance (also from a destroyed one), Palace/Temple/Courthouse/Cathedral lost and by force every second
  one of improvements 1..16 (`& 0xaaaa >> random(2)`) and all of 17.. (0x13ef5); trade routes stay; size-1 city destroyed; civil war only for a computer player's
  Palace city with > 4 cities ranked above the human (0x13db4);
  nukes (Manhattan) 3x3 kill + half size + pollution, SDI blocks.
- Pollution (0x12de9, human cities only: 0x12ddd skips it for the AI): P = shields/f - 20 + size*techs/4 (f 3 Recycling / 2 Hydro,Nuclear,Hoover / 1; Mass Transit: techs 0), polluted when 2P > random(256 - advances*difficulty/2); Nuclear Plant meltdown 1 in 3 per turn of disorder until Fusion Power (0x12f46: plant lost, 3x3 cities halved, land polluted 2/3); global warming when polluted tiles >= 8+4k (terrain shifts).
- Huts: gold 25/50/100, advance, mercenaries, barbarians (not for Settlers / near own city), empty [c1/approx].
- Disasters D1..D6 after turn 10 (1/200 per city size>=3), each cancelled by the matching improvement; off after Medicine/Industrialization [approx].
- Governments table (max rate / free units / unhappy per abroad unit / tile penalty / trade bonus / corruption / settler food / martial law):
  Anarchy lux-only/size/0/yes/no/4/1/3; Despotism 60/size/0/yes/no/3/1/3; Monarchy 70/0/0/no/no/2/1/3; Communism 80/0/0/no/no/2 flat/1/3; Republic 80/0/1/no/yes/1/2/0;
  Democracy 100/0/2/no/yes/0/2/0. Revolution: anarchy until the civ's politics on a turn with turn & 3 == 0, or the next turn with the Pyramids (0x1495a); a Democracy collapsing
  in disorder holds the human one more such turn (0x12820); computer players then take Despotism -> choose govt menu -> GOVT screen; senate auto-accepts peace under Rep/Dem.
- Diplomacy (peace/war): contact on adjacency; AI audience on first contact and periodically by attitude; attitude = traits + treaty + embassies - war - power ratio +
  random; proposals EXCHANGE/CANCEL/TRIBUTE/TAKECIV/PEACE/GIVECIV/GIVECASH/GROVEL/THREAT/TREATY/HELPME/CRUSADE/... from KING.TXT; human audience menu
  (peace / military proposal / demand tribute); embassies reveal intel; Great Wall/UN owner always offered peace (o 0x66d1, 0x151b2); AI-AI relations rolled every 10 turns.
- Barbarians: huts + sea landings from turn 50 (Chieftain 150), `2+difficulty` era-appropriate units + leader (ransom 100 gold), attack nearest city, INVADER screens.
- Wonders: effects as Civ1 (Pyramids no anarchy, Hanging Gardens +1 content, Colossus, Lighthouse +1 sea move (not with Magellan), Great Library, Oracle, Great Wall, Magellan +1 sea move,
  Michelangelo, Copernicus, Shakespeare, Newton, Bach, Darwin, Hoover, Women's Suffrage, Manhattan, UN, Apollo (owner sees all city squares, SS parts for all), SETI, Cure for Cancer);
  obsolete when any civ learns the advance (0xc91:0x6c8d); caravan help = 50 shields.
- Spaceship: after Apollo; structural <=39, component <=16, module <=12; launch when >=1/2/3 and structurals cover; flight turns `max(6, 100/(2+components/2))`;
  success from life support/habitation; capital capture loses it; arrival wins.
- Score (citizens counted at 40% luxuries, o 0x1c0e0): 2*happy + content + 20*wonders + 5*future tech + 3*peace turns (after 1 AD) - 10*polluted tiles + spaceship bonus; rating from RATINGS/ADJECTIVES;
  end on year limit / spaceship / conquest / destruction / retire; hall of fame.
- AI: production priority list (defenders, settlers by expansionist trait, Temple/Granary/Barracks/Library/Marketplace/Walls/Aqueduct/Courthouse by state,
  attackers in groups when at war, caravans/wonders in best city); unit roles (settle via best_city_site, fortify, group attack when A/(A+D) > 0.4, explore);
  rates of update_civ_tax_rates (0xa699: lux +-1 by disorder, sci = civilized + (10 - lux)/2, +1 when rich, 0 with Robotics; set_government tax 4 - gov/2);
  revolution to Monarchy/Republic; difficulty scales AI cost %, content, starting techs.
- [spec] Unit stack ring (unit +0xa, `game/stack.rs`, save v28, imported from SVE): each civ's units on a square form a
  circular list; a unit created on or stepping onto a square goes in right after the square's lowest-record unit
  (`unit_at` 0x1bade, link 0x1a68:0x13d1 / 0x1972:0xb5c, unlink 0x1972:0xbbf). The ring order decides the defender
  of equals (best_defender_stack 0x8f7e from unit_at, first ten units), the unit drawn on top, the stack box rows
  and default (0x9dc5, 0x8ee5), the unit panel's other units, the martial-law units (0xf818), the AI stack sums
  (0x92a7, ten units), the cargo a ship takes along and the stack killed in combat. Not copied: walks that stop
  after ten units also limit which units die with a stack or a sunk ship.
- [spec] Cargo (2026-10-29): no unit-to-ship link, as in CIV.EXE (the old `carried_by` field is
  `Unit::legacy_cargo`, always `None`, kept for the save layout). A land unit on an ocean square is aboard
  (wake 0x92fe, no attack 0x5ec3); boarding needs `stack_free_capacity` > 0 (0x9231: transports' capacity
  minus land units); an aircraft is on a Carrier when its stack has one (0x918d). A moving ship with a
  capacity, or a Carrier, takes units of its domain from its old successor in ring order (`take_cargo`,
  0x68ad..0x6a62): all from the sea, in port the sentried ones and a computer's not fortified ones but the
  last left; up to the capacity (Carrier and barbarians unlimited), at most 20 links. Units over the
  capacity stay, at sea too.

## Input model
Keyboard: numpad 1-9/arrows move (Home/PgUp/End/PgDn diagonals); Space skip; w wait; f fortify; s sentry; g goto; r road/rail; i irrigate; m mine;
f fortify (Settlers: fortress); P pillage; p clean pollution; b build/join city; h home; u unload; D disband; c centre; Enter end turn; Esc cancel/undo; G/O/A/W/C menus;
F1-F12 reports; Alt-R leaders randomized; Alt-V sound; Ctrl-S/Ctrl-L quick save/load; Alt-Q quit prompt (Ctrl-Q too, a civrs extra); Esc on the map: nothing (keyboard-only: look cursor off), but in the goto choice Esc ends it (a civrs extra; CIV.EXE ignores it); g again ends the goto choice (keyboard-only: g puts the look cursor on the unit and Enter picks its square). City screen: I H M V R Esc, Shift-M, Shift-A.
Mouse (320x200 space): LMB menu bar/items; LMB tile: own city -> city screen, own unit -> activate/stack list, else centre; RMB adjacent tile -> move;
goto LMB = destination; LMB minimap centres; LMB unit box cycles; dialogs LMB choose, RMB cancel; city map LMB toggles worker, citizen click cycles specialist.
Software cursor TORCH.PIC (hot spot top-left, corner colour transparent) drawn in 320x200 space; hardware cursor hidden.
Keyboard-only mode: CIV.EXE's setup asks "Mouse and Keyboard / Keyboard only" (o 0x375, ds:0x1a3c);
civrs takes the second answer as `--keyboard-only` (UiContext::keyboard_only). In it Tab (0x72e0) and
Esc (0x72d3 -> 0x4b40) switch a look cursor that starts on the active unit when that is in the map
window (else the view centre, 0x7363), moves with the keypad instead of the unit (0x4f99; off the window
the view jumps to (x - 8, y - 7)), blinks as a 16x16 outline white/black with the unit (0x8a43/0x8aff)
and on Enter acts as a left click on its square (0x4605 -> 0x4909); `MapView::look_key`. 'g' uses the
same cursor (0x4e83); civrs's goto mode keeps its own keypad cursor. With a mouse CIV.EXE ignores Tab
and Esc on the map (checked live in DOSBox), so civrs does too; the only quit question is Alt-Q (civrs also takes Ctrl-Q)
(0x728c, "Are you sure you\nwant to Quit?" / "Keep Playing" / "Yes, Quit" at (100, 80)).

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

### Differential test against CIV.EXE (reference/tools/sve_diff.py)
CIV.EXE and civrs play on from the same original save and their states are diffed field by field.
- `python3 -I reference/tools/sve_diff.py play GAME_DIR SLOT TURNS OUT_DIR` runs CIV.EXE in dosbox-staging
  under a private Xvfb (`reference/tools/dosciv.py`; display `CIVDISPLAY`, default :79; it kills only the
  processes it started). GAME_DIR is a writable copy of the game directory. The script answers the
  start-up questions (VGA, no sound, mouse and keyboard), loads CIVIL<SLOT>.SVE, turns Instant Advice
  off and End of Turn on in the Options (a note would take a key; without the End of Turn prompt the
  turn ends by itself after the last unit's move), and then for each turn presses
  Space (skip the active unit: the human's units keep their orders) and saves to slot 2. It finds the
  GAME menu and the slot cursor from the screen colours, and answers any dialog with Return. Each new
  turn's save goes to OUT_DIR/t<TURN>.SVE/.MAP, and the screenshots go to OUT_DIR/shot*.png.
- `python3 -I reference/tools/sve_diff.py diff target/release/civrs START.SVE OUT_DIR/t*.SVE` runs
  `civrs --load-sve START --dump state:N` (N = turns since START, the human doing nothing, decisions
  answered with their first choice). It diffs the result against `--dump state:0` of each CIV.EXE save,
  and prints `field: CIV.EXE | civrs` (`save::sve::state_text`: turn, year, gold, beakers, rates,
  government and revolution flag and advances per civ, city stores, production, worked squares,
  specialists, AUTO and sold flags, yields, units by owner/type with orders and work counters, tile
  improvements, the terrain of every row).
- The state dump makes the goto use the land block links stored in the save (SVE 0x8aa6 = ds:0xdb44,
  `Game::use_exe_land_links`). Overlay 7 0x9edd builds them from map page 2 while the new world is
  still on page 1, so in CIV.EXE they do not fit the world (all four sample saves); civrs computes
  sane links for play.
  With the saved links the AI's goto steps match CIV.EXE.
- Scripted human actions: `play ... --actions SCRIPT` and `diff ... --actions SCRIPT` run the same
  script in both games (format in `src/save/actions.rs`; `civrs --dump state:N --actions SCRIPT`).
  Each `turn T` block runs at the start of the human's turn T, right after the harness saved
  t<T>.SVE, so its effects show in the later saves. Unit lines name the square of the active unit
  and send map keys (keypad moves k1..k9, r i m f s b w space h, P pillage, D disband, `ret` for a
  message box; `kN:ROW` answers the menu a move opens: a caravan's "Will you?", a treaty
  partner's "Break treaty."); `tax`/`lux`
  move the cursor of the rate menus ('=' / '-', which start on the current rate); `revolution` uses
  the GAME menu; `city NAME X,Y change ROW | buy | sell ROW | spec N | tile DX,DY` opens the city
  screen with FindCity ('/' and the name, which centres the map: the city at column 7, row
  y - clamp(y - 6, 0, 38)) and a mouse click, then uses the city screen keys and Escape (the CHANGE
  menu starts on row 0); `select X,Y CITY CX,CY` centres on the city the same way and clicks the
  unit's square, then Return in the "which unit" box (pick_unit_from_stack 0x9dc5). civrs refuses
  an action it cannot do as written (another active unit, another menu item, an error message that
  would need a key in CIV.EXE), so script and games agree before the states are compared. Put the
  unit lines last in a turn and leave no error: a message box would take the next key. Scripts for
  CIVIL0/3/4 are in reference/tools/actions/.
- Which unit comes next depends on the map view (move_units 0x434f: units in columns vx+1..vx+14
  and rows vy+1..vy+9 first, `Game::next_active_unit`), so the civrs side keeps `Game::view` as
  CIV.EXE's view moves: FindCity centres it, a unit taken from outside it centres it, a moving unit
  scrolls it (scroll_to 0x9505). CIV.EXE compares those columns without the east-west wrap: with
  the view origin past x = 64 it sees no unit on screen and takes the next one anywhere. civrs
  wraps, so near x = 0 a script `select`s the unit when the two would differ. A save loaded from
  slot 4 on gives the human's units their moves back (ds:0xdf60 = 2), the slots 0..3 do not.
- Map drawing and the random stream: every `draw_map_view` (0x1a68e) draws random(256) once
  (0x1a6e8, the seed of its square order); draw_tile, draw_tile_with_units, the unit blink and
  animate_move draw none. It runs on every full map redraw: redraw_main_screen 0x3ec4 (after
  menus, reports, the city screen, game_turn 0x1bea, move_units 0x4188 ...), scroll_to 0x9505 when
  a square leaves the inner view (a step of a unit the human sees, 0x6761: his own, and a foreign
  one with the enemy-moves option ds:0x19c0 bit 0x20), the off-screen unit 0x43c9, a click on an
  empty square 0x4b96, Alt+keypad 0x6eed, 't' 0x71b8, and overlays 4, 6, 15, 20, 22, 23. civrs
  draws nothing for it: its view is drawn by the renderer, and the count depends on the screen,
  so a seeded comparison moves these draws out of CIV.EXE's game stream (civ_seed_patch
  `--quiet-ui`) rather than civrs replaying them.
- Results (2026-10-28, scripts above): CIVIL0 10 turns and CIVIL4 12 turns with every field equal;
  then only the computers' random advances (and in CIVIL3 the AI Settlers' goals and the human's
  research bonus that follows from them) differ. The runs found and fixed: the CHANGE menu's first
  row, the unit records and their order, the view-dependent next unit, scroll_to, the click on a
  lone unit, the moves after a load from slot 4, and the taxes and workers of a city that builds
  Settlers. CIV.EXE accepts 70% tax under Despotism from the rate menu (o 0x6fd9), as civrs does.
- Hand-edited saves (`reference/tools/sve_edit.py IN.SVE OUT.SVE EDIT...`: advances, city
  fields, new units, gold, diplomacy; it re-encodes the .MAP to mark each new unit's square on
  map page 2, (x + 160, y) = civ | 8, which unit_at 0x1a68:0x145e needs to see the unit). A
  city's specialists are the count in bits 26.. of its worked word (0xcce3), not its unworked
  citizens. CIVIL5.txt / CIVIL6.txt (the edits in their headers) cover specialists, caravans
  (help, keep moving, routes), the Settlers' fortress, pillage, home city, disbanding, and
  attacks on a barbarian, a treaty partner's Settlers and a city. They match 4..6 turns but for
  the veteran rolls (random(2), 0x1a20a) and the fortress's time, which CIV.EXE takes from a
  stale local of move_units (0x522a reads [bp-0x3e], set only by the road/irrigation/mine/move
  code; civrs uses the square's terrain).
- The runs found and fixed: the policy a re-choice reads (0x15cd1 stores it first), the
  re-choice of a computer city emptied by an attack (0x625e), 'f' as the only fortress key and
  'D' without a question, the next unit off the screen (from the loop's mark, 0x4150/0x4381),
  and what a load does for the human (politics, ai_civ_turn, goals and his units' looks,
  0x2503..0x25af, 0x41e3). The looks run unit by unit as CIV.EXE's loop reaches them, so a
  computer's emissary can appear in the middle of the human's actions: scripts keep units of
  civilizations at war apart. civrs looks the same way (`Game::next_active_unit`, until the first
  Settlers waits, 0x45c9), and moves goto units and aircraft returning to base when the loop takes
  them, in record order (`Game::run_auto_unit`); a scripted turn ends with a Space for every unit
  still waiting, as the harness presses it, so the loop passes them all. The human's Settlers
  work (the loop presses the job's key, 0x44bd..0x452f) and his units finish fortifying (and
  stash a city's garrison, 0x4239..0x431f) as the loop comes to them too, not at the turn's
  start, so a save made in his turn shows only the ones the loop has reached.
- The computer's and the barbarians' loop (`ai::unit_ai::move_units`) is the same move_units:
  moves restored after ai_civ_turn and ai_reset_goals (0x40a4, so the looks' goals survive),
  then by record each unit looks (every pass: the look latch is never set for a computer),
  is passed over when sentried, fortified or without moves, completes a fortification
  (0x4239), else takes up to five commands from ai_unit_command (the barbarian routine for
  civ 0; working Settlers too, 0x41fb tests only 0x09), and an aircraft counts its fuel
  (0x750f). Audiences from its contacts reach the turn runner at the look that made them.
- dosbox's captured mouse only moves relatively, one 640x400 picture pixel per host pixel, and the
  picture sits letterboxed at y = 40 of the 640x480 window: `dosciv.click` pushes the pointer into
  the top-left corner, where the driver clamps it, and then moves it by (2x + 1, 2y + 1).
- A land unit short of the move cost rolls random(cost) (0x634a), except with exactly 3 points
  left, which always get through; scripts avoid the roll.
- No RNG state is saved: CIV.EXE seeds rand() from the clock when it starts (but see the seeded
  runs below). Unseeded, divergences behind
  random() are expected: hut outcomes (0x978d random(4)), the Chieftain computer's skipped advance
  (0xbb4a), and the computer Settlers' goals. After such a divergence, its effects follow on, for
  example the human's Chieftain research bonus when another civ found the goal first (0x12c93). Two
  CIV.EXE runs from the same save differ in the same way.
- Mid- and late-game scripts (2026-10-28, all from CIVIL4 edited with sve_edit.py, the
  commands in the headers; `build:` buildings, `wonder:`, `govt:`, `rates:`, `tile:` bits,
  `warming:`, `contact:`, `work:` the first squares of the radius): MONARCHY / REPUBLIC /
  DEMOCRACY (corruption, settlers' food, units abroad, disorder, the Senate overruling a
  treaty break, a caravan's route to another continent, a far city, the Democracy falling),
  GROWTH (size 10 with and without an Aqueduct, Granary, famine), RAPTURE (We Love the King
  Day growth), POLLUTION (polluted squares, cleaning, a global warming), WONDERS. They match
  CIV.EXE field for field over 5..8 turns but for combat rolls. Found and fixed: a Democracy
  falls on the second turn of disorder by the status bit (0x12701, also right after a load),
  the caravan revenue's travel distance (0x1d6af), the view of a loaded game (capital x - 7,
  row 19, 0x1bd4) and the moves given back before the first unit is taken (0x40c0), and a
  rapture growth that only raises the size (0x12b12).
- The harness: the palace screen (overlay 17) gets section 4 (or the next one taken) and the
  first style; the copy-protection quiz (overlay 16, every 50th turn in BC) is answered by
  reference/tools/quiz.py, which finds the pictured advance by its icon's colours and rebuilds
  the menu (the candidates, names compared without case, 0x157a1) and clicks the right line
  (menu_select follows the mouse pointer, so keys alone would take the line under it); AutoSave
  is turned off (game_turn 0x2796 saves turn 50, 100, ... to CIVIL4..9 and asks for the drive).
- Effects of the diff method, not differences: a load replays the human's politics (0x252b),
  ai_civ_turn and goals, so the result saves are read as written (`--dump state:0`,
  `sve::import_as_saved`): imported as a load, a save of a turn with turn & 3 == 0 ended the
  Anarchy a second time (DEMOCRACY's t52) and one of a turn with turn & 15 == 0 let the
  contacts kept by the human's politics lapse at the replay, which found their MET bit
  cleared (MONARCHY, REPUBLIC, DEMOCRACY t64, CIVIL3 t80; a probe in the politics loop showed
  CIV.EXE's memory with the contact kept). A run of messages waiting for keys at the human's turn
  start (WONDERS t61: Rome's famine in the newspaper, the Domestic Advisor's population note,
  an advance, the research menu) is answered by the save step itself: each answered dialog
  is a try that does not count against the six failed ones, so the turn's first save still
  comes before a Space reaches a unit (it once took all six attempts, and the save came
  after a Space had skipped the Sail). CIV.EXE advances a Settlers' work when its loop
  reaches the unit (0x50db), after the save if an earlier unit waits, civrs at the turn start.
  Edited saves keep the cities' saved trade (+0xa) of the original until their next turn, which
  the caravans and routes read, so the scripts set up routes from the second turn on.
- Seeded runs (`--seed N` for `play` and `diff`): GAME_DIR's CIV.EXE is built with
  `python3 -I reference/tools/civ_seed_patch.py civ/CIV.EXE GAME_DIR/CIV.EXE --trace
  --quiet-ui` (the file must be named CIV.EXE). `play` sets CIVSEED=N, so seed_random
  (0x1d43:0x38) seeds rand() with N & 0x7fff, and keeps CIV.EXE's draws in OUT_DIR/random.log;
  `diff` runs `civrs --dump state:N:SEED --rng-log F` and prints, besides the fields, the first
  draw where the two streams part (index, CIV.EXE's call address and the address its caller
  returns to, civrs's address, n, result) with the draws around it. `play GAME_DIR new ...` and
  `diff CIVRS new ...` start a new game instead (Chieftain, 7 civilizations, the Romans,
  "Caesar"; civrs `--dump state:N:SEED` without `--load-sve`), the world generator's draws
  compared too.
- civrs draws all its game randomness from CIV.EXE's one stream (`Game::rng`: the MS C rand()
  state, random(n) = rand() * n >> 15), each call named by the CIV.EXE address it stands for
  (`Rng::random(at, n)`, overlay addresses marked by `rng::ovl`), and draws nothing for
  pictures. CIV.EXE also draws for pictures: the map view's tile order (random(256) per
  draw_map_view 0x1a68e: every redraw, scroll and centring), the page border (0xa46a, every
  save screen), the leader's animation (o 0x7ea8, every tick of an audience), the city view's
  layout (o 0x19175, which first re-seeds rand() with the sum of the city name's characters,
  o 0x1920f, on every view: a built improvement with Animations on, a capture), the
  newspaper, the spaceship's stars and the quiz's question. Those depend on the screen and on
  time, so `--quiet-ui` sends them to a 16-bit generator of their own (0:0x9ac) and skips the
  city view's srand. With that a loaded game draws nothing before its first game draw, so
  civrs needs only srand(N) after the import; a new game's stream goes on from the world
  generator into civ_init and start_report and then the turns.
- `--trace` makes random(n) append (caller ip, caller cs - load segment, n, result, the
  caller's return ip and cs and its first two arguments, e.g. civ and unit of ai_unit_command)
  to RANDOM.LOG in the current directory, C:\ (opened, appended and closed per draw, so the
  log survives the harness killing dosbox). Overlay calls log the overlay load area 0x22f30 +
  the offset in the overlay's code, which sve_diff.py matches against the overlay's flat
  address.
- `Game::exe_quirks` (set by the state dump only, like `use_exe_land_links`) plays CIV.EXE
  mistakes that civrs otherwise leaves out where they decide what is drawn: the Chieftain
  wonder purchase tests the prerequisite of the improvement with the wonder's number (0x3c4a);
  the goto's local search picks a step by the neighbours' grid costs alone (0x1eae2), so a
  road beside a city does not count; the production advisor's war test reads ds:0x6ed4, set by
  whichever civilization's ai_civ_turn ran last (0x15ca1, 0xb7c7). A new game's dump also
  takes CIV.EXE's land links from its first save (`--exe-links`, see above).
- Results (2026-10-29, seed 4763, after the fixes in this round): CIVIL4 30 turns, CIVIL0 30,
  CIVIL1 30, CIVIL3 30, GROWTH 30, RAPTURE 30, MONARCHY 30, REPUBLIC 30, DEMOCRACY 30,
  WONDERS 30, POLLUTION 24 (6 saves missing), WAR 30 (Monarchy at war, walls, veterans,
  barbarians, Berlin captured, Leipzig destroyed and the Germans back as another tribe, the
  Aztecs and Babylon at war) and a new game 40 turns: every draw of the two streams agrees,
  and the fields with them but for one-turn effects (and in WAR a Caravan counter that
  CIV.EXE's trading-at-once leaves at -1). Since the result saves are read as written
  (above) and the dump lists a square's units by record, the only one left (WONDERS t61's skipped Sail was the harness,
  above; 13 turns of WONDERS now match) is a computer
  Settlers' heading byte in a new game's t1: the harness saves through the GAME menu, and a
  menu choice while the human has no active unit runs the unit code for record 0x80
  (0x73cc -> 0x4bbe), whose heading reset (0x4bf8) lands on the next civilization's first
  unit (not copied; probes in a patched CIV.EXE logging the unit commands and that byte
  showed it). The first WAR save (its Aztecs at 70,37, 70,38 and 69,38, commit 087ab3e)
  matches now too: a unit created in play looks at once (create_unit 0x8b51 -> unit_look
  0x8cd3), so the barbarians out of a hut drop the goto of the Legion beside them. A
  Trireme of the human's at sea (CIVIL4 with a Trireme at 6,27 and a Legion next to a
  barbarian) rolls its loss when the loop leaves it (0x750f), before the Legion's fight.
  The seeded runs found and fixed: the move roll for any land move off a road with other than
  one move left (0x634a), the veteran rolls drawn for veterans, the respawn's draw order, the
  AI's square value for non-attackers, the extra defender's lower-record count (0x90dc), the
  losses counted only in the human's fights (0x6075), an attack, a failed move roll or the
  zone of control keeping the computer unit's heading (0x6433 comes after them), a captured
  city keeping its worked squares, computer moves and fights scrolling the human's view, the
  barbarians' start
  rates, the new game's opening before the human's first move, and the quirks above.
- Long seeded runs (2026-10-29, later): the new game to t100 with the human founding Rome
  and fortifying every unit it builds (so a turn needs one Space): 100 turns, every draw
  and every field but t1's heading byte. WARX: CIVIL5 with Map Making for the Romans
  (`sve_edit.py adv:1:7`), WAR.txt, then a Legion and a bought Trireme, the Trireme fetching
  the veteran Legion from Berlin to Rome ('u' wakes it), a march on Paris, contact with the
  French (their emissary), a revolution (Monarchy to Despotism), a tax change and attacks
  that destroy Paris (its size falls with each defender lost): t51..t85 match (35 turns);
  then from CIV.EXE's own t86 save t87..t101 (15) and from its t101 save t102..t135 (34),
  each stretch draw for draw and field for field. Both breaks were the script: a move that
  makes contact must be the turn's last action, the audience takes the keys after it; and
  the emissary question moves the human's view (below), so the next unit his loop takes
  is another. The runs found and fixed: a look gives the neighbour's civ a defend goal on
  the looker's square at peace too (0x84ff; only the attack goal waits on the treaty);
  CHANGE values the production menu for the human too, advisors shown or not (0x117e4..
  0x117ef: the Diplomat line's random(5), the city routine in mode -1); a computer's land
  unit walks into an empty city of a computer treaty partner and takes it, the treaty
  standing (0x6523 guards the human's land only; 0x6a65), and the old owner sees the
  conqueror (0x6a83); the emissary question draws the map with the meeting square at
  column 8, row 2 (o 0x7a63), which is the human's view afterwards; a computer city goes
  on alert only when its defender loses (a beaten attacker leaves at 0x60cd); civ_init
  clears the slot's diplomacy both ways before it looks whether the tribe comes back (o5
  0x4a81); every re-choice runs the worker assignment of the mode -1 city routine first
  (0xaa1b: a city that lost population to an attack and re-chooses at 0x625e); create_unit
  never writes the moves byte (delete_unit clears it, 0x8e6c), so a unit made in the middle
  of a turn (a respawned tribe's Settlers) has no moves till its owner's move loop. New
  `exe_quirks`: destroy_city's sweep of its square takes the conqueror's unit by the map
  mark and walks the old owner's record of that number (0xa8bc..0xa8eb; an empty record
  counts as type -1, which is the Caravan counter of the civilization before: WAR's and
  WARX's -1 Caravans), the local search's cached grid is indexed without bounds, so a probe
  beyond the window keeps the last grid of the same waypoint (0x1e765: a one-move unit's
  prices then send a Legion through a swamp), and the first-city test reads the owner byte
  of free city records (o 0x1b5df): a slot that lost more than one city gets no Palace.
  The new game continued from its t100 save matches t101..t107; then a computer city that
  built Settlers re-chose with its food-first placement skipped: process_city reads the
  local [bp-0x108] at 0xd81d before anything sets it, and when the stack word left there is
  0xffff the loop ends at once (another mode -1 run in the same game placed food first), so
  the citizens go by the weighted loop alone. Not reproducible without CIV.EXE's stack;
  like the fortress's stale [bp-0x3e] above, not copied.
- A customized new game (2026-10-30): `civrs --dump state:N:SEED --world L,T,C,A --level D`
  starts the seed's game on a Customize World world (land mass, temperature, climate, age,
  each 0..2 as the menu rows) at difficulty D; a copy of sve_diff.py that answers the title
  menu with Customize World (after the intro's Return, the menus start on row 1) and the
  difficulty menu with D x Down drives CIV.EXE the same way (King has no first-turn help
  overlay). Seed 13, small land mass, King, the human founding Rome at once and fortifying
  what it builds: the computers ship Settlers from t110 on. The run found and fixed: a step
  the zone of control refuses drops the goto (0x5b44); a unit taking the city of its goto
  drops the goto there (0x6848 before 0x6aa9); a look meets the civilization of each
  neighbour square in turn between that square's goals (0x8491: the city goals of the square
  before, its attack goal after, so a treaty signed or ended there counts for the rest of
  the look); the computer's step-back, heading and stack tests and the border rule follow
  the step's cost and move roll (0x63cc..0x64bb), and so does the capture of an empty city
  (its cost is the step's, not a full move: a Chariot from a road goes on); a finished road
  or fortress has the Settlers look around (0x53aa, 0x5268); a barbarian goto is a Goto
  order; a computer unit's five calls count from its last step (0x6c6c); and the harness
  plays with Instant Advice off, which the production menu's valuation now reads (with it
  on, choose_production values the new menu twice, 0xb9ed -> 0xaa23). Probes in patched
  copies of CIV.EXE (logging records in RANDOM.LOG from ai_add_goal 0x187e3, from the goto
  key of move_units 0x4be3 and from its call count 0x4464) showed each. With them the draws
  agree to t81 (was t9). Left: a barbarian Sail's goto to Rome, inland, at t72. Its block
  search finds no sea block for the target (goto_block_entry 0x1e522 fails), and every
  goto_block_cost 0x1ec12 it tried for the target's neighbour blocks has overwritten
  ds:0x6590/0x6592 with the goto square, so 0x1e0dc runs its breadth-first search from
  "block" (42, 14), outside the 20x13 grids, reading the masks and marking the distances
  past their tables (0x7f38 + 13x + y, ds:0xd816 + 13x + y), and finds a waypoint (73, 25)
  that civrs, starting from the unit's own block as when nothing was tried, does not (it
  steps toward the goto). Not reproducible: the search reads the distance of every
  square it reaches (cells 13x + y from 546 down; the queue only takes cells whose distance
  byte is 0), and any way from cell 560 back into the table (cells below 260, steps of at most
  14) passes cells 288..559, which are ds:0xd936.., the 512-byte buffer of the PIC reader and
  writer (pic_file_reader 0x1f83a, lzw_put_code 0x15cd): the end of the last picture read or
  written (in the harness the .MAP of the last save; after a load the loaded one), not game
  state (the masks it reads there, ds:0x803c..0x8178, are the AI statistics, which civrs has).
  With `exe_quirks` civrs starts from the goto square as a block when that stays inside the
  tables (a goto square with 13x + y below 260 and its search staying in them), else from the
  start block. The barbarians' later draws part at t82; from CIV.EXE's t82 save the Sail (goto
  Rome, re-searched every turn) parts at once (t83 at 3,2 in CIV.EXE, 7,7 in civrs). The
  barbarians' raids aim at Rome (inland) again and again, so stretches were played from CIV.EXE's
  saves between raids: t92..t102 (10 turns of draws, parting when the next raid's Sail parts) and
  from t106. They found: the census's land squares and sites are the world generator's tables
  ds:0xdcfe and ds:0xb1ee (overlay 7 writes them, nothing in play), which leave out the two rows
  at either pole (civrs counted the map now, so a continent with polar land looked three times
  its size and lost its offensive policy; imported from SVE 0x0a88/0x0b88, saved in v31); and a
  computer's Phalanx stepping into an empty enemy city whose square the map layer x+160 still
  gave its own civ (a respawned tribe's city) walked in and stayed beside the city (0x6a65 tests
  tile_owner, read at 0x65f0; `exe_quirks`). That city then works two squares at size 1 with
  the enemy unit inside (t107): worker placement 0xd454 checks all 21 squares (ds:0x6540),
  the centre among them, against the owner the map layer x+160 records (0x1a68:0x14e6), so
  the walked-in unit idles the centre and both citizens work outside it, which a kept layout
  carries on until the next fresh placement (`City::worked` bit 0 = `CENTRE_IDLE`, imported
  from the SVE's bit 20). From t106 the stretch now agrees to t115 in draws and fields; the
  next parting is a goto local search that reuses a grid flooded for a one-move unit
  (ds:0x6796 cache): CIV.EXE has searched another waypoint in between (civ3's Settlers and
  civ4's Militia keep their gotos to (29,18) and search, civrs's Settlers drop theirs in
  their look; not resolved). (The K13 run's t6.. "Militia fortify" difference was the
  script: r-K13 was played with K13F.txt, with the turn-5 'f'; with it the run agrees to
  t69 in fields and t81 in draws.)

  LATE (2026-10-31): a late-era start made from CIV.EXE's t100 save of the seed-4763 NEWF run
  by sve_edit.py (scratchpad w90/mklate.py: 39..52 advances per civ up to Space Flight,
  Communism, Democracy and Republic governments, Factories, Power Plants, Apollo Program,
  polluted squares and warming 14, Settlers, Caravans, Diplomats, Mech. Inf., Artillery, a
  war of the Germans with the English and the Babylonians). Played seeded from t100 to t129
  (the harness slows down as the human's Rome fills with Militia it never moves): all 29
  saves agree draw for draw, fields to t125. It found: a Diplomat disbanded in its own city
  runs the rest of the unit routine as the freed record (0x16a95; `exe_quirks`); a rioting
  computer Republic/Democracy sends its farthest land unit home every eighth record turn and
  its disorder counts as new (0xfd3e..0xfedf); the government choice reads ds:0xe3c2/0xdb42
  as all city runs since the civ's cities came up left them (`Game::gov_case`); Chieftain
  science doubles for the research menu's answer (0x12c93); Anarchy splits trade by the
  rates (0xf237); a city's score and return value read the last city run's moods (0x12dbe,
  0x13223); a human city run for attention lets the garrison out and stashes it again by
  record (0xcda0, 0x1320d); the production advice counts units on squares numbered as the
  continent, ocean bodies too (0xb0c1, `exe_quirks`); a move keeps the work counter (+5).
  Left at t126: a Battleship's long sea goto takes another way (block search). A human's step that crosses a treaty partner's border is asked about
  after the move roll now (0x634a before 0x64bb), see "Questions inside a computer's unit loop".
  The t126 Battleship (goto (67,37)) is the PIC-buffer case of the block search: its goto
  square has no block of its own and the search from it as a block leaves the tables.
  LATE2 (2026-10-31, scratchpad w91/L129.txt from CIV.EXE's t129: Rome changes to Women's
  Suffrage, its units sentry, so the harness keeps pace): 30 turns, draws agree t130-t140,
  fields but the Battleship to t140; t141 the Battleship (civrs's way) attacks Babylon. It
  found: the advisor's run that a completed production starts inside a city's turn (0xaa1b)
  leaves the city routine's globals, so the turn's trade split starts from its total with
  the routes (ds:0x70de, 0xf136) and adds them again, and the caravan revenue compares the
  target's continent with the home city's (x, x) (0x14738), both `exe_quirks`.
  Seed 13 from t106 now agrees in draws t106-t122 and fields to t119: the t116 parting was
  civ3's Settlers beside civ4's city Tenochtitlan with a civ3 Phalanx walked in; the
  square's owner for the zone of control and enemy tests is the layer x+160's (civ4,
  0x1bb66), not the first unit's (`Game::unit_owner_at`). (The searches toward (29,18) seen
  in the hook run were the turn before.) t120 is the barbarian Sail's goto Rome, the
  PIC-buffer block search again. Hook probes (scratchpad w91/probe3.py: ai_unit_command
  entry and exit with the goto bytes; probe4.py: the trade store 0xf136 with city and mode)
  fit the 60 free bytes of the cave.

  Round w92 (2026-10-31, scratchpad w92). A city's turn reads its totals as counted before
  its production (`economy::TurnTotals`): the units abroad for its happiness (ds:0x6546,
  0xd26b) and the shields, divisor and advance count of the pollution (0x12de9), so a
  Bomber, Factory or Mass Transit of this turn counts from the next; and it collects the
  taxes and science of its trade split (ds:0xe17a, ds:0x70e6), with the tiles of its last
  turn's celebration, before its moods change the flag. With `exe_quirks` the advisor's
  nested run (0xaa1b, after a computer's improvement, unit or alert, or a human AUTO
  choice) leaves its totals for the rest of the turn: shields for the unit support and the
  government cases (0xfd23, 0x10303), food for a celebration's growth (0x12ad6), units
  abroad, the pollution factors, and after a mood change's run (0x12761, 0x128ff) its split
  and moods for the taxes, science and the celebration test (0x12972); a celebration
  survives disorder (0x12b19 clears it only in order). The advisor itself values from its
  own run, not the turn's carried totals (an advance learned in the turn re-chooses inside
  it). Long runs: LATE2 from CIV.EXE's t141 (w92/r-L141) agrees in draws t142-t171, fields
  to t167 (t168 an Ironclad's sea goto). The manual late-game saves (tests/data/late,
  renamed to slot 1 as the harness loads them, since slots 4..9 give the human his moves
  back): CIVIL4 (t350) agrees t351-t362 in draws, CIVIL3 (t405) t406-t413. They found: a
  differential dump from an SVE starts without the research catch-up ds:0xd2de (set only at
  the 50-turn marks, 0x27e0; not saved); a caravan's home continent at (x, x) reads, from
  x = 50 on, page 2 at (x, x + 50), the improvement nibble of (x, x - 50) (0x1bfc8); a
  barbarian uprising's square needs no unit at all next to it (enemy_adjacent gets
  game_turn's [bp-0x2], 8 after its civ loops, 0x2f00); the harness's keys while the human's
  loop has no unit (the save or options at a turn's start with none waiting, the key that
  ends the turn) run the unit code for record 0x80, which forgets the heading of the next
  civ's record 0 (0x76c3, 0x73cc -> 0x4bbe, 0x4bf8; this was NEW's t1 difference); and the
  end-of-turn garrison stash tests foreign_unit_adjacent(city x, city y, 0x1c * index),
  the saved si of the city loop (0x131fa; a probe showed 336 for Carthage, index 12, and
  "adjacent"): rows below 50 are the unit layer, 50..199 picture data that the .MAP keeps
  (`Game::exe_page_band`), from 200 on memory past the driver's row table (cs:0x17d6),
  taken as adjacent. Probes (w92/probe5..8.py: direction scores at 0x1870a, the unit
  command's key at 0x18766, the garrison check, taxes at 0x1296b) and their parser plog.py,
  cmpscore.py. Left: CIVIL4's t357 human Settlers work a turn apart (the human loop's first
  unit depends on the map view), whose terrain change parts the human's research at t363;
  CIVIL3 t408 the same.
  Seed 13 (w92/r-K122B, from CIV.EXE's t122 with the barbarian Sail stack of (4, 4) removed by
  sve_edit.py `kill:0:4,4`): draws t123-t128, then the next barbarian raid at Rome. An
  unseeded run (w92/r-N100U from NEWF t100, CIV.EXE seeded from the clock; seedof.py finds
  the seed, 3183, from the first draws) agrees t101-t114: the t108 parting is the
  uninitialised [bp-0x108] at 0xd81d in the nested advisor run after Delhi's Settlers (the
  food-first pass skipped when it holds -1; the same in seed 13's t127 Samarkand): with the
  pass skipped civrs's assignment gives exactly CIV.EXE's squares, but the slot holds what the
  last frame at that depth left, so always skipping it breaks other runs. No draws of civrs's
  that CIV.EXE lacks were found before such a parting.
  Later in the round: every quiet run of the city routine places the workers too, so a
  computer city starts over on its (city + turn) & 3 == 0 turns also in the runs between its
  turns (0xd454 in mode -1; it was CIVIL4's research parting at t363), and a city whose mood
  changes re-chooses with its own unit off the building counts (0x12756, 0x128f4). SPACE
  (w92/mkspace.py: LATE2's t170 with Space Flight, Plastics and Superconductor for the
  English, Germans and Romans, structurals and components in production, the Manhattan
  Project, 20 polluted squares and warming 16) agrees in draws t171-t200 (spaceship parts
  built, a global warming, a Nuclear unit in production), fields to t181 (t182 a German
  Submarine's sea goto). CIVIL3 continued from CIV.EXE's t413: parts at once (t414) on the
  Sail of Chapultepec that releases fortified units by the size of city [bp-0xc], an
  uninitialised local of ai_unit_command (0x16e9a; civrs takes the ship's city).
  Round w93 (2026-10-31, scratchpad w93; probe9..12.py log the local search's waypoint and
  cache (0x1dcb4), the garrison checks 0x430d and 0x131fa (the latter without a frame, so
  the record names process_city's caller), unit_look's garrison release 0x828e). LATE2's
  t168 Ironclad and SHIP2's t202 Cruiser/Battleship are the picture-buffer block search: a
  sea goto onto a land square whose block has no sea links, entered from a probed
  neighbour, floods from the goto square taken as a block (13 x + y up to ~600), reading
  link masks past ds:0x7f38 and "visited" bytes past ds:0xd816, i.e. the PIC buffer
  ds:0xd936 (CIV.EXE's waypoint for the Ironclad was (25, 21), 3 blocks away). SPACE's
  t182 Submarine was a rule: a new sea or air goal sends only the first unit of its role in
  reach (0x18901); SPACE's t185 purchase is the exe_quirks [bp-0xe8] (0xec84 reads the
  improvement id 0xe49e left). The human's first unit: messages about a square move the
  map (draw_map_view: pollution, meltdown, uprising, foreign disaster, revolt, bribe), and a
  foreign move scrolls by the seen bits before the step (0x6721); CIVIL4-late (T350,
  t351-t369) and CIVIL3-late (T405, t406-t413) now agree in every field. Every quiet run of
  the city routine ends at the garrison check (0x125d6 -> 0x131d2): civ_score's runs (si
  0x1c * index, o 0x1c1be) stash the garrison an audience's look let out. Staging notes: a
  `city:..:prod=` edit leaves the per-type production counts (SVE 0x318 + 0x38 civ + 2 type,
  ds:0xd308) stale, which the AI's choice reads (w93/mkship.py corrects them). Runs: SHIP2
  (SPACE's t200, English ship complete but one module, Apollo, the Oracle in London, the
  human Democratic and researching Religion, at war with the English: module placed,
  forced-or-chosen launch, Oracle obsolete) agrees t201 in fields, draws to t203; SHIP3 (the
  same with the English a Democracy: the audience's Senate makes them a Monarchy, o 0x6973)
  t201 too; both part at t202 on English ships' block searches toward a German or Aztec
  city (any goto square from x = 20 on lies past the 20x13 tables, so this limits late sea
  wars); ARRIVE
  (CIVIL3-late with an Aztec ship launched, arriving 1859) agrees t406-t408 and both end the
  game in the same turn. Left: the harness key for record 0x80 with a goto on the next
  civ's record 0 (0x4bce: goto_direction for the human's civ moves that unit; not seen in
  the stored runs), the human's Senate (the harness answers yes to peace).

### Map screen pixel comparison (reference/tools/mapshot.py)
- `python3 -I reference/tools/mapshot.py GAME_DIR SLOT OUT [N] [KEYS...]` loads CIVIL<SLOT>.SVE in
  CIV.EXE (dosciv.py, `CIVDISPLAY`), takes N shots 0.25 s apart and writes 320x200 PPMs to diff
  against `civrs --dump screen:map --load-sve CIVILn.SVE --out F.ppm`. The water and rivers turn
  three palette rings every 15 ticks (`mapview::water_cycles`) and the active unit blinks every 10
  frames (blink_wait 0x8932: hidden 10 ticks, shown 10), so compare a capture against the renders at
  `CIVRS_FRAMES` = 15k and 15k + 10 (k = 0..15) and take the best. With that, captures of CIVIL0, 1, 3 and 4 match pixel for pixel, map and
  sidebar. `reference/tools/sve_edit.py` `imp:` and `unit:...:SEEN` stage improvements (real and
  as the human saw them) and foreign units next to the human's.
- Known differences of a staged save: the squares around added human units are explored in civrs
  (the import's vision pass) and not in CIV.EXE until the units look. draw_map_view draws the
  squares in a random order (an LCG seeded with random(256), 0x1a68e), so a Settlers' job frame
  (x - 1, y - 1) or a stack's second copy (+1, +1) may be covered by a neighbour drawn later;
  civrs draws row by row.
- What draw_tile (0x1aa40) shows of the improvements is the human's view of them, map page band
  (x + 80, y + 100/150) (`Map::seen`, save v26): his units' looks, his own work, pollution,
  his cities' captures and Apollo copy the real bits there (update_seen_improvements 0x1bc87).

## Questions inside a computer's unit loop

CIV.EXE asks the human in the middle of a computer's `move_units` (0x403e): an audience that a
unit's look or move opens (civs_meet 0x1508d into overlay 6) is held at once, and the answer holds
for the rest of that civ's moves. The turn runner plays the loop with `unit_ai::start_units`,
which stops after the look or the command that raised a question for the human
(`TurnStep::Decision`); `TurnRunner::units` keeps where it stood (the pass, the record, the unit
and its next command call) and `unit_ai::resume_units` goes on from there once the question is
answered. `unit_ai::move_units` (tests, tools) runs the loop to its end, with
`Game::may_fight` keeping the peace while an audience waits.

A human goto stepping onto a treaty partner's developed land (the territory rule 0x64bb) drops
the goto before "We have signed a peace treaty" (0x656d) and asks; "Cancel action." leaves the
unit waiting for orders (0x58c1 -> 0x6d7f -> 0x442c) with the step's cost spent, "Break treaty."
makes the step and the unit waits for orders after it. The question comes after the step's cost
and move roll (0x62b1..0x63c9, 0x634a before 0x64bb): a failed roll ends the unit's turn with no
question and the goto kept, so the next turn's step rolls (and asks) again. Scripts answer that question with `menu ROW`
(src/save/actions.rs; the harness's `act` needs `menu` as rows + Return).

### City screen pixel comparison (mapshot.py with clicks)
- `mapshot.py GAME_DIR SLOT OUT N @X,Y ...`: an `@X,Y` key token clicks the 320x200 screen. A
  click on a map square away from the active unit centres the map there; one on an own city
  opens its city screen. civrs side: `CIVRS_CITY=NAME CIVRS_MOUSE=X,Y civrs --dump
  screen:city[:0:0:happy|map] --load-sve F --out F.ppm` (the torch where the click left it).
  CIVIL0/1/3/4 (Rome, Caesarea) and staged busy cities (`sve_edit.py` `build:`, `unit:`,
  `city:..:route=C`, `work:`) match pixel for pixel, INFO, HAPPY and MAP tabs, up to the water
  cycles, which go on behind the city screen (compare against several `CIVRS_FRAMES`).
- Load staged saves from a slot below 4: from slot 4 on the load gives the human's units their
  moves and the unit loop completes fortifications before the screen opens (0x40c0).
- Layout facts: the small icons are draw_icon's 7x7 cuts (SP257 (129 + 8 (i & 3), 33 + 2 (i & 4))),
  the units the 15x15 map icons in load_sprite_sheets' civ colours; the city size and the unit
  letters on the city map and in the INFO box are in the screen's current font, the small one;
  `work_tile` (0x13233) redraws each worked square and packs its icons 8/5/3 px apart, rows 8
  px down (a third row runs into the square below, which a later square may cover again).
- Not matched: the martial law units of the HAPPY tab follow CIV.EXE's stack ring (unit +0xa,
  walked from 0x1a68:0x145e), which civrs does not keep.

## The site/owner layer (map page 2, x+80)

`Map::claims` (save v29) is CIV.EXE's band (x+80, y): the generator's site values (its copy at
(x+80, y+50) is `Map::site` and never changes), the founder's number on a new city's radius and
8 for site values beyond (o 0x1b7ae), the owner on each square a city works in its turn (work_tile
0x132e3), the captor's on a captured city's radius (0x13fbc), 0 where another civ's unit steps
(0x6815), the generator's values back where a city is destroyed (0xa8f3; beyond the radius only
without another city within 3). The territory rule, huts, the computer's Settlers and landings,
the BUILDCITY hint and the respawn search read it. `state:N` dumps it as `claims.yN` rows; it
matches CIV.EXE's saves over the CIVIL0 (10 turns) and CIVIL4 (20 turns) scripts.
