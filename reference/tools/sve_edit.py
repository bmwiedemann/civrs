#!/usr/bin/env python3
"""Edit a copy of an original CIVILn.SVE to stage a situation for the differential test.

    python3 -I sve_edit.py IN.SVE OUT.SVE EDIT [EDIT ...]

The .MAP next to IN.SVE is copied next to OUT.SVE. Edits (offsets as src/save/sve.rs):
    adv:CIV:A,B,...            give civ CIV the advances A, B (ids as src/rules/advances.rs) and
                               raise its research count (SVE 0x4d8) by the new ones
    city:ID:FIELD=V[,...]      a city record field: size, food, shields, prod (the signed
                               production byte: unit type, -improvement, -24-wonder), worked
                               (CIV.EXE radius bit mask, bit 20 the centre), codes (the 2-bit
                               specialist codes +0x14: 1 taxman, 2 scientist, 3 entertainer),
                               status, trade (the stored trade +0xa), route (a trade route to
                               that city in the first free slot +0x17..0x19); a size change also changes the owner's citizens (0x708)
    unit:CIV:TYPE:X,Y[:HOME[:FLAGS[:SEEN[:GX,GY]]]]   a new unit in the civ's lowest free record,
                               with no goto (or one to GX,GY), full moves (thirds) and the
                               stack ring of its square; the
                               civ's unit counters (0x158, and 0x6d8 for non-Settlers) go up;
                               SEEN = the civs that have seen it (+9, bit per civ)
    imp:X,Y:REAL[:SEEN]        the square's improvement bits on map page 2 (0x02 irrigation,
                               0x04 mine, 0x08 road, 0x10 railroad, 0x20 fortress, 0x40
                               pollution; low nibble at (x, y+100), high at (x, y+150)) and the
                               human's view of them at (x+80, ...) (default: REAL)
    gold:CIV:N                 the civ's gold
    beakers:CIV:N              the civ's research store (0x148)
    census:CIV:K:DEF,ATT,CITIES[:STRENGTH]   the census row of continent K as the civ's last
                               turn left it (0x788 / 0x888 / 0x988, strength 0x718)
    options:N                  the options byte (0x8aa4: 1 Instant Advice, 2 AutoSave, 4 End
                               of Turn, 8 Animations, 0x10 Sound, 0x20 Enemy Moves, ...)
    diplo:A:B:V                the diplomacy word of A towards B (0x648 + 16 A + 2 B)
    build:CITY:A,B,...         build the improvements A, B (ids as src/rules/improvements.rs:
                               record bit id - 1)
    wonder:W:CITY              wonder W (src/rules/wonders.rs) stands in CITY (0x8672 + 2 W)
    govt:CIV:G                 the government (0x538: 1 Despotism, 2 Monarchy, 3 Communism,
                               4 Republic, 5 Democracy)
    rates:CIV:TAX,SCI          the tax and science rates in percent (0x738, 0x8bb0: tenths)
    tile:X,Y:BITS              OR the improvement bits into the square and the human's view of
                               it (as imp:), pollution (0x40) also counted in 0x8a9e
    warming:N                  the global warming counter (0x8aa0)
    contact:CIV:TURN           the civ's last contact turn (0x758); a computer civ in
                               contact asks for an audience when it sees a human unit 16
                               turns after it (unit_look), so a turn ahead keeps it quiet
    owner:CITY:CIV             the city's owner (+0xb) and its square's mark on map page 2
    work:CITY:N                the city works its first N land or ocean squares in CIV.EXE's
                               radius order (ds:0x2498), skipping squares other cities work or
                               foreign units hold, and has size - N specialists (entertainers
                               unless a later codes= says otherwise); give it after size=
"""
import struct
import sys

CITIES, UNITS, ADV, RCOUNT, COUNTS, NUNITS, CITIZENS, GOLD, DIPLO = 0x1508, 0x26c0, 0x4e8, 0x4d8, 0x158, 0x6d8, 0x708, 0x138, 0x648
WONDERS, GOVT, TAX, SCIENCE, POLLUTED = 0x8672, 0x538, 0x738, 0x8bb0, 0x8a9e
BEAKERS, OPTIONS = 0x148, 0x8aa4
# the city radius in CIV.EXE's order (ds:0x2498 / 0x24c2, src/game/economy.rs EXE_RADIUS)
RADIUS = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1), (0, -2), (2, 0), (0, 2), (-2, 0),
          (-1, -2), (1, -2), (2, -1), (2, 1), (1, 2), (-1, 2), (-2, 1), (-2, -1)]
# moves (whole squares) by unit type, as src/rules/units.rs
MOVES = [1, 1, 1, 1, 1, 1, 2, 2, 1, 1, 2, 3, 3, 2, 10, 8, 3, 3, 3, 4, 6, 4, 3, 5, 4, 16, 2, 1]


def w(d, o):
    return struct.unpack_from('<h', d, o)[0]


def put(d, o, v):
    struct.pack_into('<h', d, o, v)


def edit(d, e, page):
    kind, *a = e.split(':')
    if kind == 'adv':
        civ = int(a[0])
        for n in map(int, a[1].split(',')):
            o = ADV + 10 * civ + n // 8
            if not d[o] >> (n % 8) & 1:
                d[o] |= 1 << (n % 8)
                put(d, RCOUNT + 2 * civ, w(d, RCOUNT + 2 * civ) + 1)
    elif kind == 'city':
        r = CITIES + 0x1c * int(a[0])
        for f in a[1].split(','):
            k, v = f.split('=')
            v = int(v, 0)
            if k == 'size':
                owner = d[r + 0xb] & 7
                put(d, CITIZENS + 2 * owner, w(d, CITIZENS + 2 * owner) + v - d[r + 7])
                d[r + 7] = v
            elif k == 'food':
                put(d, r + 0xc, v)
            elif k == 'shields':
                put(d, r + 0xe, v)
            elif k == 'prod':
                d[r + 9] = v & 0xff
            elif k == 'worked':
                struct.pack_into('<I', d, r + 0x10, v)
            elif k == 'codes':
                struct.pack_into('<H', d, r + 0x14, v)
            elif k == 'status':
                d[r + 6] = v
            elif k == 'trade':
                d[r + 0xa] = v
            elif k == 'route':
                # route=C adds a trade route to city C in the first free slot +0x17..0x19
                s = next(s for s in range(0x17, 0x1a) if d[r + s] == 0xff)
                d[r + s] = v
            else:
                raise ValueError(k)
    elif kind == 'unit':
        civ, typ = int(a[0]), int(a[1])
        x, y = map(int, a[2].split(','))
        home = int(a[3]) if len(a) > 3 else 0xff
        flags = int(a[4], 0) if len(a) > 4 else 0
        seen = int(a[5], 0) if len(a) > 5 else 0
        base = UNITS + 0x600 * civ
        i = next(i for i in range(128) if d[base + 12 * i + 3] == 0xff)
        here = [j for j in range(128) if d[base + 12 * j + 3] != 0xff and d[base + 12 * j + 1] == x and d[base + 12 * j + 2] == y]
        r = base + 12 * i
        gx, gy = map(int, a[6].split(',')) if len(a) > 6 else (0xff, 0)
        d[r:r + 12] = bytes([flags, x, y, typ, 3 * MOVES[typ], 0, gx, gy, 0xff, seen, 0xff, home])
        if here:
            # the stack ring +0xa: put the new unit after the first one of the square
            first = base + 12 * here[0]
            nxt = d[first + 0xa]
            d[first + 0xa] = i
            d[r + 0xa] = here[0] if nxt == 0xff else nxt
        put(d, COUNTS + 0x38 * civ + 2 * typ, w(d, COUNTS + 0x38 * civ + 2 * typ) + 1)
        if typ != 0:
            put(d, NUNITS + 2 * civ, w(d, NUNITS + 2 * civ) + 1)
    elif kind == 'gold':
        put(d, GOLD + 2 * int(a[0]), int(a[1]))
    elif kind == 'census':
        # the last AI turn's census (0x788 defence, 0x888 attack, 0x988 cities by continent;
        # 0x718 strength) that an audience at the load reads
        civ, k = int(a[0]), int(a[1])
        for base, v in zip((0x788, 0x888, 0x988), a[2].split(',')):
            put(d, base + 32 * civ + 2 * k, int(v))
        if len(a) > 3:
            put(d, 0x718 + 2 * civ, int(a[3]))
    elif kind == 'beakers':
        put(d, BEAKERS + 2 * int(a[0]), int(a[1]))
    elif kind == 'options':
        d[OPTIONS] = int(a[0], 0)
    elif kind == 'diplo':
        put(d, DIPLO + 16 * int(a[0]) + 2 * int(a[1]), int(a[2], 0))
    elif kind == 'build':
        r = CITIES + 0x1c * int(a[0])
        bits = struct.unpack_from('<I', d, r)[0]
        for n in map(int, a[1].split(',')):
            bits |= 1 << (n - 1)
        struct.pack_into('<I', d, r, bits)
    elif kind == 'wonder':
        put(d, WONDERS + 2 * int(a[0]), int(a[1]))
    elif kind == 'govt':
        put(d, GOVT + 2 * int(a[0]), int(a[1]))
    elif kind == 'rates':
        tax, sci = map(int, a[1].split(','))
        put(d, TAX + 2 * int(a[0]), tax // 10)
        put(d, SCIENCE + 2 * int(a[0]), sci // 10)
    elif kind == 'tile':
        x, y = map(int, a[0].split(','))
        bits = int(a[1], 0)
        lo, hi = 320 * (y + 100) + x, 320 * (y + 150) + x
        old = page[lo] & 15 | (page[hi] & 15) << 4
        if bits & 0x40 and not old & 0x40:
            put(d, POLLUTED, w(d, POLLUTED) + 1)
        for dx in (0, 80):
            # the square and the human's view of it (x + 80); bit 0x80 stays
            v = (page[lo + dx] & 15 | (page[hi + dx] & 7) << 4) | bits & 0x7f
            page[lo + dx] = page[lo + dx] & 0xf0 | v & 15
            page[hi + dx] = page[hi + dx] & 0xf8 | v >> 4 & 7
    elif kind == 'contact':
        put(d, 0x758 + 2 * int(a[0]), int(a[1]))
    elif kind == 'warming':
        put(d, 0x8aa0, int(a[0]))
    elif kind == 'owner':
        r = CITIES + 0x1c * int(a[0])
        civ = int(a[1])
        old = d[r + 0xb] & 7
        size = d[r + 7]
        put(d, CITIZENS + 2 * old, w(d, CITIZENS + 2 * old) - size)
        put(d, CITIZENS + 2 * civ, w(d, CITIZENS + 2 * civ) + size)
        # the city counts (0x6c8, ds:0x6b66)
        put(d, 0x6c8 + 2 * old, w(d, 0x6c8 + 2 * old) - 1)
        put(d, 0x6c8 + 2 * civ, w(d, 0x6c8 + 2 * civ) + 1)
        d[r + 0xb] = civ
        at = 320 * d[r + 5] + d[r + 4] + 160
        page[at] = page[at] & 8 | civ
    elif kind == 'work':
        c = int(a[0])
        r = CITIES + 0x1c * c
        n, cx, cy = int(a[1]), d[r + 4], d[r + 5]
        taken = set()
        for o in range(128):
            q = CITIES + 0x1c * o
            if o != c and d[q + 6] != 0xff:
                m = struct.unpack_from('<I', d, q + 0x10)[0]
                taken |= {((d[q + 4] + dx) % 80, d[q + 5] + dy) for b, (dx, dy) in enumerate(RADIUS) if m >> b & 1}
        owner = d[r + 0xb] & 7
        mask = 0
        for b, (dx, dy) in enumerate(RADIUS):
            if bin(mask).count('1') == n:
                break
            sq = ((cx + dx) % 80, cy + dy)
            held = page[320 * sq[1] + sq[0] + 160]
            if sq in taken or (held & 8 and held & 7 != owner):
                continue
            mask |= 1 << b
        assert bin(mask).count('1') == n, 'only %d free squares' % bin(mask).count('1')
        spec = d[r + 7] - n
        assert spec >= 0
        struct.pack_into('<I', d, r + 0x10, mask | 1 << 20 | spec << 26)
        struct.pack_into('<H', d, r + 0x14, 0xffff)
    else:
        raise ValueError(e)


def lzw_decode(src, max_bits):
    """The PIC LZW (src/assets/pic.rs lzw_decode): LSB-first codes from 9 bits, code 256
    reserved, new codes from 257, a reset when the dictionary fills at max_bits."""
    def fresh():
        return [bytes([i]) for i in range(256)] + [b'']
    dic, out, buf, nb, pos, width, prev = fresh(), bytearray(), 0, 0, 0, 9, None
    while True:
        while nb < width and pos < len(src):
            buf |= src[pos] << nb
            pos += 1
            nb += 8
        if nb < width:
            break
        code = buf & ((1 << width) - 1)
        buf >>= width
        nb -= width
        if code < len(dic) and code != 256:
            e = dic[code]
        elif code == len(dic) and prev is not None:
            e = dic[prev] + dic[prev][:1]
        else:
            break
        out += e
        if prev is not None:
            dic.append(dic[prev] + e[:1])
        prev = code
        if len(dic) >= 1 << width:
            if width < max_bits:
                width += 1
            else:
                dic, width, prev = fresh(), 9, None
    return bytes(out)


def lzw_encode(data, max_bits):
    """The inverse of lzw_decode: the decoder adds the entry of code i when it reads code
    i + 1, so the encoder's widths and resets follow the decoder's dictionary length."""
    out, buf, nb = bytearray(), 0, 0
    dic, n_read, width = {}, 0, 9

    def emit(code):
        nonlocal buf, nb, n_read, width, dic
        buf |= code << nb
        nb += width
        while nb >= 8:
            out.append(buf & 0xff)
            buf >>= 8
            nb -= 8
        # the decoder's dictionary length after reading this code
        n_read += 1
        size = 257 + n_read - 1
        if size >= 1 << width:
            if width < max_bits:
                width += 1
            else:
                dic, n_read, width = {}, 0, 9
                return True
        return False

    def code_of(w):
        return w[0] if len(w) == 1 else dic[w]

    w = b''
    for c in data:
        wc = w + bytes([c])
        if len(wc) == 1 or wc in dic:
            w = wc
            continue
        nxt = 257 + len(dic)
        if not emit(code_of(w)):
            dic[wc] = nxt
        w = bytes([c])
    if w:
        emit(code_of(w))
    if nb:
        out.append(buf & 0xff)
    return bytes(out)


def rle_encode(data):
    out, i = bytearray(), 0
    while i < len(data):
        b = data[i]
        n = 1
        while i + n < len(data) and data[i + n] == b and n < 255:
            n += 1
        out += b'\x90\x00' if b == 0x90 else bytes([b])
        if n >= 3:
            out += bytes([0x90, n])
        else:
            n = 1
        i += n
    return bytes(out)


def rle_decode(src):
    out, last, i = bytearray(), 0, 0
    while i < len(src):
        if src[i] == 0x90 and i + 1 < len(src):
            n = src[i + 1]
            i += 2
            if n == 0:
                out.append(0x90)
                last = 0x90
            else:
                out += bytes([last]) * (n - 1)
        else:
            last = src[i]
            out.append(last)
            i += 1
    return bytes(out)


def read_map(path):
    d = open(path, 'rb').read()
    assert d[:2] == b'X0'
    n = struct.unpack_from('<H', d, 2)[0]
    w, h, bits = struct.unpack_from('<HHB', d, 4)
    return bytearray(rle_decode(lzw_decode(d[9:4 + n], bits))[:w * h]), (w, h, bits)


def write_map(path, pixels, head):
    w, h, bits = head
    body = struct.pack('<HHB', w, h, bits) + lzw_encode(rle_encode(bytes(pixels)), bits)
    assert len(body) < 0x10000
    open(path, 'wb').write(b'X0' + struct.pack('<H', len(body)) + body)


def main(src, dst, *edits):
    d = bytearray(open(src, 'rb').read())
    page, head = read_map(src[:-3] + 'MAP')
    for e in edits:
        if e.startswith('imp:'):
            _, xy, *bits = e.split(':')
            x, y = map(int, xy.split(','))
            real = int(bits[0], 0)
            for dx, v in ((0, real), (80, int(bits[1], 0) if len(bits) > 1 else real)):
                # (bit 0x80, the second river sprites, stays)
                lo, hi = 320 * (y + 100) + x + dx, 320 * (y + 150) + x + dx
                page[lo] = page[lo] & 0xf0 | v & 15
                page[hi] = page[hi] & 0xf8 | v >> 4 & 7
            continue
        edit(d, e, page)
        # a unit marks its square on map page 2: (x + 160, y) = civ | 8 (0x1a68:0x13d1)
        if e.startswith('unit:'):
            civ, x, y = int(e.split(':')[1]), *map(int, e.split(':')[3].split(','))
            page[320 * y + x + 160] = civ | 8
    open(dst, 'wb').write(d)
    write_map(dst[:-3] + 'MAP', page, head)
    again, _ = read_map(dst[:-3] + 'MAP')
    assert again == page


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print(__doc__)
    else:
        main(*sys.argv[1:])
