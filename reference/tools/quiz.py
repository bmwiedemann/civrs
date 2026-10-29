#!/usr/bin/env python3
"""Answer CIV.EXE's copy-protection quiz (overlay 16, civilization_quiz o 0x157a1).

    python3 -I quiz.py GAME_DIR SHOT.png      print the advance, the y of the right line, the menu

On every 50th turn while the year is BC the quiz shows the icon of a random advance with two
real prerequisites and asks for them; a second wrong answer deletes every unit of the human.
The advance is found by matching the icon's colours against the advance icons of
ICONPG1..ICONPG8.PIC (far table 0x269d:0x0000, src/ui/pedia_data.rs ADVANCE_ICON_ORDER), and
the menu is rebuilt as the quiz builds it: advances whose index plus the target's is a
multiple of 16 or that share a prerequisite with it or need it, those with two prerequisites,
in the order of their names, at most 12 lines with the target's always among them.
"""
import os
import struct
import subprocess
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sve_edit import lzw_decode, rle_decode  # noqa: E402

# the advance table ds:0x4da (22 bytes: name, +0x14 / +0x15 the prerequisites; 127 = never)
ADVANCES = [('Alphabet', -1, -1), ('Code of Laws', 0, -1), ('Currency', 17, -1), ('Atomic Theory', 51, 11), ('Democracy', 28, 30),
            ('Monarchy', 25, 1), ('Astronomy', 26, 9), ('MapMaking', 0, -1), ('Navigation', 7, 6), ('Mathematics', 0, 16), ('Medicine', 28, 24),
            ('Physics', 9, 8), ('Engineering', 33, 45), ('University', 9, 28), ('Magnetism', 8, 11), ('Electronics', 12, 54), ('Masonry', -1, -1),
            ('Bronze Working', -1, -1), ('Iron Working', 17, -1), ('Bridge Building', 18, 45), ('Invention', 12, 30), ('Computers', 9, 15),
            ('Writing', 0, -1), ('Steam Engine', 11, 20), ('Trade', 2, 1), ('Ceremonial Burial', -1, -1), ('Mysticism', 25, -1),
            ('Nuclear Fission', 41, 3), ('Philosophy', 26, 30), ('Religion', 28, 22), ('Literacy', 22, 1), ('Horseback Riding', -1, -1),
            ('Feudalism', 16, 5), ('The Wheel', -1, -1), ('Gunpowder', 20, 18), ('Industrialization', 49, 53), ('Chemistry', 13, 10),
            ('Combustion', 55, 56), ('Flight', 37, 11), ('Advanced Flight', 38, 54), ('Space Flight', 21, 46), ('Mass Production', 58, 47),
            ('Pottery', -1, -1), ('Communism', 28, 35), ('The Republic', 1, 30), ('Construction', 16, 2), ('Rocketry', 39, 15),
            ('The Corporation', 53, 35), ('Metallurgy', 34, 13), ('RailRoad', 23, 19), ('Nuclear Power', 27, 15), ('Theory of Gravity', 6, 13),
            ('Steel', 48, 35), ('Banking', 24, 44), ('Electricity', 48, 14), ('Refining', 36, 47), ('Explosives', 34, 36),
            ('SuperConductor', 60, 41), ('Automobile', 37, 52), ('Genetic Engineering', 10, 47), ('Plastics', 55, 40), ('Recycling', 41, 4),
            ('Chivalry', 32, 31), ('Robotics', 60, 21), ('Conscription', 44, 56), ('Labor Union', 41, 43), ('Fusion Power', 50, 57),
            ('1', 127, 127), ('2', 127, 127), ('3', 127, 127), ('4', 127, 127), ('Future Tech.', 127, 127)]
# the advances in the order of their icons, nine to a page row by row
ICON_ORDER = [24, 32, 11, 65, 26, 36, 13, 59, 53, 44, 39, 5, 1, 51, 4, 35, 16, 50, 52, 38, 29, 49, 22, 0, 21, 10, 33, 60, 19, 37, 45,
              12, 15, 43, 30, 40, 2, 41, 17, 27, 18, 3, 61, 56, 57, 6, 34, 55, 14, 62, 20, 58, 23, 8, 64, 66, 63, 31, 9, 48, 42, 7, 47, 54,
              28, 25, 46]
BACKGROUND = 235


def menu(t):
    """The quiz's answer lines for target advance t (o 0x157a1 .. the menu) and the row of the
    right one."""
    p1, p2 = ADVANCES[t][1:]
    # the names compare without case (stricmp 0x1fc3:0x28f6), from "a" to "zzz"
    prev, lines, right = 'a', [], None
    while True:
        best, low = None, 'zzz'
        for i, (name, a, b) in enumerate(ADVANCES):
            if (i + t) & 15 == 0 or a in (p1, p2, t) or b in (p1, p2, t):
                if a != 127 and b != 127 and prev < name.lower() < low:
                    best, low = i, name.lower()
        if best is not None and (best == t or len(lines) < 11):
            if best == t:
                right = len(lines)
            a, b = ADVANCES[best][1:]
            if a != -1 and b != -1:
                lines.append('%s and %s' % (ADVANCES[a][0], ADVANCES[b][0]))
        prev = low
        if best is None or len(lines) > 11:
            return lines, right


def read_pic(path):
    d = open(path, 'rb').read()
    pos = 0
    while pos + 4 <= len(d):
        tag, n = d[pos:pos + 2], struct.unpack_from('<H', d, pos + 2)[0]
        body = d[pos + 4:pos + 4 + n]
        pos += 4 + n
        if tag == b'X0' and len(body) > 5:
            w, h, bits = struct.unpack_from('<HHB', body, 0)
            if w and h:
                return rle_decode(lzw_decode(body[5:], bits)), w
    raise ValueError(path)


def pages(game):
    """The 256 RGB colours (M0, 6-bit levels as dosbox shows them) and pixels of each
    ICONPGn.PIC."""
    out = {}
    for n in range(1, 9):
        d = open(os.path.join(game, 'ICONPG%d.PIC' % n), 'rb').read()
        pos, cols = 0, {}
        while pos + 4 <= len(d):
            tag, ln = d[pos:pos + 2], struct.unpack_from('<H', d, pos + 2)[0]
            body = d[pos + 4:pos + 4 + ln]
            pos += 4 + ln
            if tag == b'M0':
                for i in range(body[0], body[1] + 1):
                    j = 2 + 3 * (i - body[0])
                    cols[i] = tuple((v << 2) | (v >> 4) for v in body[j:j + 3])
        px, w = read_pic(os.path.join(game, 'ICONPG%d.PIC' % n))
        out[n] = (cols, px, w)
    return out


def screen(shot):
    d = subprocess.run(['magick', shot, '-depth', '8', 'ppm:-'], capture_output=True).stdout
    h = d.split(b'\n', 3)
    w = int(h[1].split()[0])
    return lambda x, y: tuple(h[3][3 * (2 * y * w + 2 * x):3 * (2 * y * w + 2 * x) + 3])


def match(game, spx):
    """(error, advance): the icon cell (110x68 from (1 + 111 col, 1 + 69 row)) is copied to
    the screen left of the menu, below the tutor text (0x15919..0x15943), so each icon is
    tried at every place of that range, on a grid of every fifth pixel."""
    pg = pages(game)
    best = None
    for k, adv in enumerate(ICON_ORDER):
        cols, px, w = pg[k // 9 + 1]
        cx, cy = 1 + 111 * (k % 3), 1 + 69 * (k % 9 // 3)
        pts = [(x, y, cols.get(px[(cy + y) * w + cx + x], (0, 0, 0))) for y in range(0, min(68, 200 - cy), 5) for x in range(0, 110, 5)
               if cx + x < 320 and px[(cy + y) * w + cx + x] != 253]
        for ox in range(0, 21):
            for oy in range(50, 126):
                err = sum(abs(a - b) for x, y, c in pts if oy + y < 200 for a, b in zip(c, spx(ox + x, oy + y))) / max(len(pts), 1)
                if best is None or err < best[0]:
                    best = (err, adv)
    return best


def menu_rows(spx):
    """The centre y of each menu line (320x200): runs of rows with black in the first letters
    (x 110..120) below the question."""
    ys = [y for y in range(85, 200) if any(spx(x, y) == (0, 0, 0) for x in range(110, 121))]
    out, start = [], None
    for y in ys + [999]:
        if start is None:
            start = last = y
        elif y > last + 1:
            out.append((start + last) // 2)
            start = last = y
        else:
            last = y
    return out


def is_question(px):
    """The quiz page asks (black "Which civilization advances are required ..." below the
    tutor text); the verdict page has no such line."""
    return any(px(x, y) in (b'\0\0\0', (0, 0, 0)) for y in range(112, 126, 2) for x in range(48, 560, 2))


def is_quiz(px):
    """The quiz page: grey (235) all round, the black title "CIVILIZATION QUIZ" at the top
    and a cyan menu bar."""
    grey = bytes([BACKGROUND] * 3)
    return px(5, 5) == grey and px(600, 390) == grey and px(100, 60) == grey and px(300, 20) == b'\0\0\0' and px(214, 20) == b'\0\0\0'


def solve(game, shot):
    """The advance pictured, the menu lines and the y of the right answer's line."""
    spx = screen(shot)
    err, adv = match(game, spx)
    if err > 30:
        return None, [], None
    lines, right = menu(adv)
    rows = menu_rows(spx)
    return adv, lines, rows[right] if len(rows) == len(lines) else None


if __name__ == '__main__':
    t, lines, right = solve(sys.argv[1], sys.argv[2])
    print(t is not None and ADVANCES[t][0], right, lines)
