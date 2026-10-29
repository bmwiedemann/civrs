#!/usr/bin/env python3
"""Build a runnable CIV.EXE whose random seed can be set from the DOS environment.

    python3 -I civ_seed_patch.py ../../../civ/CIV.EXE OUT/CIV.EXE

In DOSBox, `SET CIVSEED=4763` before `civ` makes the game seed rand() (and the map seed
ds:0x6e80) with 4763 & 0x7fff instead of time(0) & 0x7fff. That is the same value a clock seed
would have given, so civrs's MsRand seeds < 0x8000 line up with it. Without CIVSEED the patched
game behaves like the original (clock seed).

The output is CIV.EXE unpacked (EXEPACK removed, load image == civ.bin from unexepack.py) with
the overlays appended at the next 512-byte page, as the overlay manager (0x1fc3:0x2bf1) expects:
it walks the MZ headers by e_cp pages, never by e_cblp. The file must keep the name CIV.EXE
because the overlay manager reopens the program by its own path.

Patch:
- seed_random 0x1d43:0x38 becomes `jmp far 0:0x940`. Its two old far-call relocations
  (0x1d46e, 0x1d47d) are dropped and one for the jump is added.
- 0:0x940 is in the unused far-jump stub entries 52..89 (0:0x8d9..0x9e3, all zero, never
  written by the driver loader). The new routine there scans the DOS environment block
  (PSP:0x2c, PSP from int 21h/62h) for CIVSEED=digits, else takes time(0), then does
      ax &= 0x7fff; ds:0x6e80 = ax; srand(ax)
  and returns to seed_random's caller. (The C getenv found nothing when called with the name
  on the stack.) The two far calls into the C runtime segment 0x1fc3 get new relocations.
"""
import struct, sys

TIME, SRAND = 0x25ec, 0x2ade  # time() and srand() in segment 0x1fc3
RT_SEG = 0x1fc3
SEED_RANDOM = 0x1d468          # 0x1d43:0x38
CAVE = 0x940                   # 0:0x940
CAVE_END = 0x9e3               # first used stub after the free entries (90 = 0x76d + 7*90)


def unpack(d):
    """(EXEPACK header fields, load image, relocations as linear addresses)."""
    e_cblp, e_cp, e_crlc, e_cparhdr, e_minalloc, e_maxalloc = struct.unpack_from('<6H', d, 2)
    e_cs = struct.unpack_from('<H', d, 0x16)[0]
    hdr = e_cparhdr * 16
    exe_len = (e_cp - 1) * 512 + e_cblp if e_cblp else e_cp * 512
    img = d[hdr:exe_len]
    ep = e_cs * 16
    real_ip, real_cs, mem_start, exepack_size, real_sp, real_ss, dest_len = struct.unpack_from('<7H', img, ep)
    assert img[ep + 14:ep + 16] == b'RB', 'not EXEPACK'
    packed = img[:ep]
    out = bytearray(dest_len * 16)
    out[:len(packed)] = packed
    src = len(packed)
    while src > 0 and packed[src - 1] == 0xFF:
        src -= 1
    dst = len(out)
    while True:
        cmd = out[src - 1]; length = out[src - 3] | (out[src - 2] << 8); src -= 3
        if cmd & 0xFE == 0xB0:
            fill = out[src - 1]; src -= 1
            dst -= length
            out[dst:dst + length] = bytes([fill]) * length
        elif cmd & 0xFE == 0xB2:
            dst -= length; src -= length
            out[dst:dst + length] = out[src:src + length]
        else:
            sys.exit(f'bad command {cmd:#x}')
        if cmd & 1:
            break
    stub = img[ep:ep + exepack_size]
    pos = stub.find(b'Packed file is corrupt')
    assert pos > 0, 'relocation table not found'
    pos += len(b'Packed file is corrupt')
    relocs = []
    for i in range(16):
        count = struct.unpack_from('<H', stub, pos)[0]; pos += 2
        for _ in range(count):
            relocs.append(i * 0x10000 + struct.unpack_from('<H', stub, pos)[0]); pos += 2
    hdr_fields = dict(cp=e_cp, minalloc=e_minalloc, maxalloc=e_maxalloc, ip=real_ip, cs=real_cs,
                      sp=real_sp, ss=real_ss, packed_len=len(img))
    return hdr_fields, out, relocs


def assemble(items, base, relocs):
    """Bytes for `items`: raw bytes, ('label', name), ('jcc', opcode, name) short jumps,
    ('far', off) = call far 0x1fc3:off (its segment word gets a relocation), ('addr', name) =
    the 16-bit offset of a label."""
    code, labels, fix = bytearray(), {}, []
    for it in items:
        if isinstance(it, bytes):
            code += it
        elif it[0] == 'label':
            labels[it[1]] = len(code)
        elif it[0] == 'jcc':
            code += bytes([it[1], 0]); fix.append(('rel8', len(code) - 1, it[2]))
        elif it[0] == 'far':
            relocs.append(base + len(code) + 3)
            code += b'\x9a' + struct.pack('<HH', it[1], RT_SEG)
        elif it[0] == 'addr':
            code += b'\0\0'; fix.append(('abs16', len(code) - 2, it[1]))
    for kind, at, name in fix:
        if kind == 'rel8':
            rel = labels[name] - (at + 1)
            assert -128 <= rel < 128
            code[at] = rel & 0xff
        else:
            struct.pack_into('<H', code, at, base + labels[name])
    return code


def cave_code(relocs):
    JE, JNE, JB, JA, JMP = 0x74, 0x75, 0x72, 0x77, 0xeb
    return assemble([
        b'\x56\x57\x1e',                 # push si; push di; push ds
        b'\xb4\x62\xcd\x21',             # mov ah,0x62; int 21h -> bx = PSP
        b'\x8e\xdb\x8e\x1e\x2c\x00',     # mov ds,bx; mov ds,[0x2c] (environment block)
        b'\x31\xf6\xfc',                 # xor si,si; cld
        ('label', 'var'),
        b'\x80\x3c\x00', ('jcc', JE, 'clock'),   # cmp byte [si],0; je clock (end of block)
        b'\x8b\xde\x0e\x07',             # mov bx,si; push cs; pop es
        b'\xbf', ('addr', 'name'),        # mov di,name
        b'\xb9\x08\x00\xf3\xa6',         # mov cx,8; repe cmpsb
        ('jcc', JE, 'found'),
        b'\x8b\xf3',                     # mov si,bx
        ('label', 'skip'),
        b'\xac\x0a\xc0', ('jcc', JNE, 'skip'),   # lodsb; or al,al; jnz skip
        ('jcc', JMP, 'var'),
        ('label', 'found'),
        b'\x31\xdb',                     # xor bx,bx
        ('label', 'digit'),
        b'\xac\x2c\x30', ('jcc', JB, 'number'),  # lodsb; sub al,'0'; jb number
        b'\x3c\x09', ('jcc', JA, 'number'),      # cmp al,9; ja number
        b'\x98\x6b\xdb\x0a\x03\xd8',     # cbw; imul bx,bx,10; add bx,ax
        ('jcc', JMP, 'digit'),
        ('label', 'number'),
        b'\x8b\xc3\x1f\x5f\x5e',         # mov ax,bx; pop ds; pop di; pop si
        ('jcc', JMP, 'seeded'),
        ('label', 'clock'),
        b'\x1f\x5f\x5e',                 # pop ds; pop di; pop si
        b'\x31\xc0\x50', ('far', TIME),   # time(0)
        b'\x83\xc4\x02',                 # add sp,2
        ('label', 'seeded'),
        b'\x80\xe4\x7f',                 # and ah,0x7f
        b'\xa3\x80\x6e\x50', ('far', SRAND),    # mov ds:0x6e80,ax; srand(ax)
        b'\x83\xc4\x02\xcb',             # add sp,2; retf
        ('label', 'name'), b'CIVSEED=',
    ], CAVE, relocs)


def main(src_path, out_path):
    d = open(src_path, 'rb').read()
    h, img, relocs = unpack(d)
    # the code being replaced, so a different CIV.EXE version is refused
    assert img[SEED_RANDOM:SEED_RANDOM + 27] == bytes.fromhex(
        '2bc0509aec25c31f83c40280e47fa3806e509ade2ac31f83c402cb'), 'unexpected seed_random'
    assert not any(img[0x8d9:CAVE_END]), 'stub entries 52..89 not free'
    for r in (SEED_RANDOM + 6, SEED_RANDOM + 21):
        relocs.remove(r)
    code = cave_code(relocs)
    assert CAVE + len(code) <= CAVE_END
    img[CAVE:CAVE + len(code)] = code
    jmp = b'\xea' + struct.pack('<HH', CAVE, 0)       # jmp far 0:CAVE (segment relocated)
    img[SEED_RANDOM:SEED_RANDOM + 27] = jmp + b'\x90' * (27 - len(jmp))
    relocs.append(SEED_RANDOM + 3)

    # image padded to whole pages so the overlays start at e_cp * 512
    img += bytes(-len(img) % 512)
    hdr_len = (0x1c + 4 * len(relocs) + 511) // 512 * 512
    total = hdr_len + len(img)
    # memory: the original's (packed image + minalloc), and at least up to the initial stack
    need = max((h['packed_len'] + 15) // 16 + h['minalloc'], h['ss'] + (h['sp'] + 15) // 16)
    minalloc = max(need - len(img) // 16, 0)
    hd = bytearray(hdr_len)
    struct.pack_into('<2s13H', hd, 0, b'MZ', 0, total // 512, len(relocs), hdr_len // 16,
                     minalloc, h['maxalloc'], h['ss'], h['sp'], 0, h['ip'], h['cs'], 0x1c, 0)
    for i, r in enumerate(relocs):
        struct.pack_into('<2H', hd, 0x1c + 4 * i, r & 0xffff, (r >> 16) * 0x1000)
    overlays = d[h['cp'] * 512:]
    assert overlays[:2] == b'MZ', 'no overlay at the page after the packed image'
    open(out_path, 'wb').write(bytes(hd) + bytes(img) + overlays)
    print(f'{out_path}: image {len(img):#x} bytes, {len(relocs)} relocations, '
          f'seed routine {len(code)} bytes at 0:{CAVE:#x}')


if __name__ == '__main__':
    main(*sys.argv[1:3])
