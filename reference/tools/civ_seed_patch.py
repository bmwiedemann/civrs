#!/usr/bin/env python3
"""Build a runnable CIV.EXE whose random seed can be set from the DOS environment.

    python3 -I civ_seed_patch.py ../../../civ/CIV.EXE OUT/CIV.EXE [--trace] [--quiet-ui]

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
- With --trace, random(n) (0x1d43:0x53) jumps from its last shift (0x1d499) to 0:0x8d9, which
  does the shift and appends a record to RANDOM.LOG (in the current directory, C:\\) for every
  draw: the caller's return ip, its code segment minus the load segment (0x1d43-style for the
  main image; overlays share their load area at 0x22f3), n, the result, and the return ip and
  segment of the caller's caller and the caller's first two arguments (eight words). The file must exist (the harness creates
  it empty); each record opens, appends and closes it, so the log survives a killed dosbox.
"""
import struct, sys

TIME, SRAND = 0x25ec, 0x2ade  # time() and srand() in segment 0x1fc3
RT_SEG = 0x1fc3
SEED_RANDOM = 0x1d468          # 0x1d43:0x38
CAVE = 0x940                   # 0:0x940
CAVE_END = 0x9e3               # first used stub after the free entries (90 = 0x76d + 7*90)
RANDOM_TAIL = 0x1d499          # random(n) 0x1d43:0x53 from `mov cl,15` on (shift, pop bp, retf)
LSHR = 0x32c6                  # long shift right in segment 0x1fc3
TRACE = 0x8d9                  # 0:0x8d9, the free stub entries 52..
TRACE_DATA = 0x9c8             # its record and file name, after the seed routine
QUIET = 0x9ac                  # 0:0x9ac, the presentation draws' own generator (--quiet-ui)
OVL_BIN = 0x175                # civ.overlay.bin (flat overlay addresses) starts 0x175 before e_cp * 512
# random(n) calls that only make pictures: the map view's drawing order (draw_map_view
# 0x1a68e), the page border (0xa46a), the leader's animation (o 0x7ea8), the quiz's question
# (o 0x157a1), the spaceship screen's stars (o 0x17575, o 0x181be), the city view's layout
# (o 0x19175, which also re-seeds rand() with the city name, o 0x1920f) and the newspaper
# (o 0x1cb75)
QUIET_MAIN = [0x1a6e8, 0xa476]
QUIET_OVL = [0x7fe4, 0x804b, 0x157b2, 0x1760e, 0x17621, 0x181ec, 0x19281, 0x1933b, 0x1938f,
             0x193d7, 0x19427, 0x1946c, 0x19506, 0x19517, 0x19791, 0x197a2, 0x1ccca, 0x1cd1d,
             0x1cdf1, 0x1cf1a]
CITY_VIEW_SRAND = 0x1920f


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


def trace_code(relocs):
    """random(n)'s tail: the shift of rand() * n, then a record appended to RANDOM.LOG in the
    current directory (which must exist): the caller's ip and cs, n, the result, and through
    the caller's saved bp the ip and cs it returns to and its first two arguments (for
    ai_unit_command the civ and the unit), eight little-endian words, the code segments
    relative to the load segment."""
    JB = 0x72
    buf = lambda off: struct.pack('<H', TRACE_DATA + off)
    return assemble([
        b'\xb1\x0f', ('far', LSHR),     # mov cl,15; call far lshr (dx:ax >>= 15)
        b'\x50\x56\x57\x06\x1e',         # push ax si di es ds
        b'\x16\x1f\x0e\x07',             # ds = ss (DGROUP), es = cs
        b'\x8d\x76\x02\xbf', buf(0),     # lea si,[bp+2]; mov di,buf
        b'\xfc\xa5\xa5\xa5\xab',         # cld; movsw x3 (ip, cs, n); stosw (the result)
        b'\x8b\x76\x00\x8d\x74\x02\xa5\xa5\xa5\xa5',  # si = [bp] + 2: the caller's return, 2 args
        b'\x8c\xcb\x26\x29\x1e', buf(2), # mov bx,cs; sub es:[buf+2],bx
        b'\x26\x29\x1e', buf(10),        # sub es:[buf+10],bx
        b'\x0e\x1f',                     # push cs; pop ds
        b'\xba', buf(16), b'\xb8\x01\x3d\xcd\x21',   # open(name, write)
        ('jcc', JB, 'done'),
        b'\x8b\xd8\x31\xc9\x31\xd2\xb8\x02\x42\xcd\x21',  # bx = handle; seek to the end
        b'\xba', buf(0), b'\xb9\x10\x00\xb4\x40\xcd\x21',  # write 16 bytes
        b'\xb4\x3e\xcd\x21',             # close
        ('label', 'done'),
        b'\x1f\x07\x5f\x5e\x58\x5d\xcb', # pop ds es di si ax bp; retf
    ], TRACE, relocs)


TRACE_RECORD = bytes(16) + b'RANDOM.LOG\0'   # the record; the log's name


def quiet_code():
    """random(n) for pictures from a 16-bit Weyl sequence of its own: (s += 0x4e6d) / 2 * n
    >> 15, so they draw nothing from rand()."""
    st = struct.pack('<H', QUIET + 26)
    return (b'\x55\x8b\xec'                        # push bp; mov bp,sp
            + b'\x2e\x81\x06' + st + b'\x6d\x4e'      # add cs:[s],0x4e6d
            + b'\x2e\xa1' + st                       # mov ax,cs:[s]
            + b'\xd1\xe8\xf7\x66\x06'                # shr ax,1; mul word [bp+6]
            + b'\xd1\xe0\xd1\xd2\x92'                # shl ax,1; rcl dx,1; xchg ax,dx
            + b'\x5d\xcb' + b'\0\0')                  # pop bp; retf; s


def main(src_path, out_path, trace=False, quiet=False):
    d = open(src_path, 'rb').read()
    h, img, relocs = unpack(d)
    # the code being replaced, so a different CIV.EXE version is refused
    assert img[SEED_RANDOM:SEED_RANDOM + 27] == bytes.fromhex(
        '2bc0509aec25c31f83c40280e47fa3806e509ade2ac31f83c402cb'), 'unexpected seed_random'
    assert not any(img[0x8d9:CAVE_END]), 'stub entries 52..89 not free'
    for r in (SEED_RANDOM + 6, SEED_RANDOM + 21):
        relocs.remove(r)
    code = cave_code(relocs)
    assert CAVE + len(code) <= (TRACE_DATA if trace else CAVE_END)
    img[CAVE:CAVE + len(code)] = code
    jmp = b'\xea' + struct.pack('<HH', CAVE, 0)       # jmp far 0:CAVE (segment relocated)
    img[SEED_RANDOM:SEED_RANDOM + 27] = jmp + b'\x90' * (27 - len(jmp))
    relocs.append(SEED_RANDOM + 3)
    if trace:
        assert img[RANDOM_TAIL:RANDOM_TAIL + 9] == bytes.fromhex('b10f9ac632c31f5dcb'), 'unexpected random'
        relocs.remove(RANDOM_TAIL + 5)
        tcode = trace_code(relocs)
        assert TRACE + len(tcode) <= CAVE and TRACE_DATA + len(TRACE_RECORD) <= CAVE_END
        img[TRACE:TRACE + len(tcode)] = tcode
        img[TRACE_DATA:TRACE_DATA + len(TRACE_RECORD)] = TRACE_RECORD
        img[RANDOM_TAIL:RANDOM_TAIL + 9] = b'\xea' + struct.pack('<HH', TRACE, 0) + b'\x90' * 4
        relocs.append(RANDOM_TAIL + 3)

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
    overlays = bytearray(d[h['cp'] * 512:])
    assert overlays[:2] == b'MZ', 'no overlay at the page after the packed image'
    if quiet:
        q = quiet_code()
        assert QUIET + len(q) <= TRACE_DATA and not any(img[QUIET:QUIET + len(q)])
        img[QUIET:QUIET + len(q)] = q
        call = b'\x9a' + struct.pack('<HH', 0x53, 0x1d43)
        far = b'\x9a' + struct.pack('<HH', QUIET, 0)   # the segment word keeps its relocation
        for a in QUIET_MAIN:
            assert img[a:a + 5] == call, hex(a)
            img[a:a + 5] = far
        for a in QUIET_OVL:
            o = a - OVL_BIN
            assert overlays[o:o + 5] == call, hex(a)
            overlays[o:o + 5] = far
        # the city view's srand(name): jump over the call (its relocated segment word stays)
        o = CITY_VIEW_SRAND - OVL_BIN
        assert overlays[o:o + 5] == b'\x9a' + struct.pack('<HH', SRAND, RT_SEG)
        overlays[o:o + 2] = b'\xeb\x03'
    open(out_path, 'wb').write(bytes(hd) + bytes(img) + bytes(overlays))
    print(f'{out_path}: image {len(img):#x} bytes, {len(relocs)} relocations, '
          f'seed routine {len(code)} bytes at 0:{CAVE:#x}')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], '--trace' in sys.argv[3:], '--quiet-ui' in sys.argv[3:])
