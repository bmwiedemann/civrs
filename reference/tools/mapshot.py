#!/usr/bin/env python3
"""Capture CIV.EXE's map screen right after loading an original save.

    python3 -I mapshot.py GAME_DIR SLOT OUT_PREFIX [N] [KEYS...]

Starts CIV.EXE (reference/tools/dosciv.py, display CIVDISPLAY), loads CIVIL<SLOT>.SVE, sends the
optional extra KEYS (xdotool names, sleepN.N pauses, @X,Y mouse clicks on the 320x200
screen, e.g. @200,112 on a city square opens its city screen) and takes N screenshots (default 4, 0.25 s
apart, so both phases of the active unit's blink show) as OUT_PREFIX<i>.png, then 320x200 PPMs
OUT_PREFIX<i>.ppm for a pixel diff against `civrs --dump screen:map --load-sve ...`.
GAME_DIR must be a private, writable copy of the game directory.
"""
import os
import subprocess
import sys
import time

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import dosciv  # noqa: E402


def main():
    game, slot, out = sys.argv[1:4]
    n = int(sys.argv[4]) if len(sys.argv) > 4 else 4
    extra = sys.argv[5:]
    dosciv.start(game)
    k = dosciv.keys
    try:
        time.sleep(6)
        k(['1', 'sleep2', '1', 'sleep8', '1', 'sleep10', 'Return', 'sleep5', 'Down', 'Return', 'sleep3', 'Return', 'sleep2'])
        k(['Down'] * int(slot) + ['Return', 'sleep6'])
        for e in extra:
            if e.startswith('@'):
                x, y = e[1:].split(',')
                dosciv.click(int(x), int(y))
            else:
                k([e])
        for i in range(n):
            png = '%s%d.png' % (out, i)
            dosciv.shot(png)
            subprocess.run(['magick', png, '-sample', '320x200', '-depth', '8', '%s%d.ppm' % (out, i)])
            time.sleep(0.25)
    finally:
        dosciv.stop()


if __name__ == '__main__':
    main()
