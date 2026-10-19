# Disassembly of CIV.EXE and MGRAPHIC.EXE (generated, gitignored)

Regenerate with (from `civrs/`):

    python3 -I reference/tools/unexepack.py ../civ/CIV.EXE reference/disasm/civ.bin
    mv reference/disasm/civ.bin.overlay reference/disasm/civ.overlay.bin
    objdump -D -b binary -m i386 -M i8086,intel reference/disasm/civ.bin > reference/disasm/civ.asm
    objdump -D -b binary -m i386 -M i8086,intel reference/disasm/civ.overlay.bin > reference/disasm/civ-overlays.asm
    objdump -D -b binary -m i386 -M i8086,intel --start-address=0x200 ../civ/MGRAPHIC.EXE > reference/disasm/mgraphic.asm

(`civ.asm` / `civ-overlays.asm` here additionally carry `; ds:"..."` comments on `mov ax,imm`
instructions whose immediate points at a data-segment string.)

Files:
- `civ.bin` — the unpacked (EXEPACK) load image of CIV.EXE, 201,520 bytes. Address = seg*16+off;
  far calls `9a off seg` are load-relative; entry point 0x1fc3:0x2e85; data segment 0x2a80 (DS = file 0x2a800).
- `civ.overlay.bin` — the 23 MS C overlays appended to the EXE (each an MZ header, code invoked via
  `int 3Fh`); their far calls to segment 0 mean the main image's first segment (unrelocated).
- `mgraphic.asm` — the MCGA driver; addresses are file offsets, code offset = file - 0x200.
  Header at code 0x24: [0x28] code segment (DOS-relocated), [0x2e] first stub index, [0x30] entry count (52),
  [0x32..] entry offsets. CIV.EXE patches them into far-jump stubs at 0:0x76d + 7*i (jmp at +2), so driver
  entry i is `call 0:0x76f+7*i` (C convention, caller cleans).

Known driver entries / call sites (see DESIGN.md "Graphics driver"):
- entry 10 (driver code 0x6b7 = file 0x8b7, stub 0:0x7b5): pixel dissolve `(page, duration)`, 16-bit LFSR taps 0xB400.
  Call sites in `civ-overlays.asm`: 0x1700 title logo (20), 0x166f5 palace (20), 0xe08d score wall after the
  "... destroy the ... civilization!" plaque line (30), 0x19fb1 city founding (10).
- entry 20 (stub 0:0x7fb): copy rect `(src page, x, y, w, h, dst page, x, y)`; entry 32 (stub 0:0x84d): draw picture.
- game helpers in segment 0: 0xbfa fill rect on a page, 0x4aa fade palette `(ticks, pal)`, 0x382/0x3fa/0x42b palette cycling.
- discovery screen: main image 0xbcc0..0xbf6b (no dissolve: picture copied, icon palette faded with 0:0x4aa over 30 ticks).
