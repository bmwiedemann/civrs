#!/usr/bin/env python3
"""Drive the original CIV.EXE in dosbox-staging under a private Xvfb display.

    python3 -I dosciv.py start GAME_DIR          start Xvfb + dosbox running CIV.EXE in GAME_DIR
    python3 -I dosciv.py keys K1 K2 ...          send xdotool keys (sleepN.N pauses)
    python3 -I dosciv.py shot FILE.png           screenshot of the 640x400 picture
    python3 -I dosciv.py click X Y               mouse click at (X, Y) of the 320x200 screen
    python3 -I dosciv.py stop                    kill the dosbox and Xvfb this tool started

Environment: CIVDISPLAY (default :79). CIVSEED=N is passed on to the DOS environment; a CIV.EXE
built by civ_seed_patch.py then seeds its RNG with N & 0x7fff instead of the clock. The pids of the started processes are kept in
$CIVSTATE (default: a file per display in the temp directory) so `stop` kills only those. GAME_DIR must be a private, writable copy of
the original game directory (CIV.EXE and its files); dosbox mounts it as C:.
"""
import os
import subprocess
import sys
import tempfile
import time

DISPLAY = os.environ.get('CIVDISPLAY', ':79')
ENV = dict(os.environ, DISPLAY=DISPLAY, SDL_VIDEODRIVER='x11')
STATE = os.path.join(tempfile.gettempdir(), 'dosciv%s.pids' % DISPLAY.replace(':', '-'))


def _state_file():
    return os.environ.get('CIVSTATE', STATE)


def win():
    out = subprocess.run(['xdotool', 'search', '--class', 'dosbox'], env=ENV, capture_output=True, text=True).stdout.split()
    return out[-1] if out else None


def start(game, cycles='max'):
    game = os.path.abspath(game)
    conf = os.path.join(os.path.dirname(game), 'dosciv.conf')
    seed = os.environ.get('CIVSEED')
    seed = f'set CIVSEED={int(seed)}\n' if seed else ''
    with open(conf, 'w') as f:
        f.write(f"""[sdl]
fullscreen = false
output = surface
texture_renderer = software
window_size = 640x400
[dosbox]
machine = vgaonly
memsize = 16
[render]
glshader = none
aspect = false
[cpu]
core = auto
cycles = {cycles}
[mixer]
nosound = true
[sblaster]
sbtype = none
[gus]
gus = false
[speaker]
pcspeaker = false
tandy = off
[mouse]
mouse_capture = onstart
mouse_raw_input = false
[autoexec]
mount c {game}
c:
{seed}civ
""")
    x = subprocess.Popen(['Xvfb', DISPLAY, '-screen', '0', '1280x800x24', '-nolisten', 'tcp'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2)
    log = open(os.path.join(os.path.dirname(game), 'dosbox.log'), 'w')
    d = subprocess.Popen(['dosbox', '-conf', conf], env=ENV, stdout=log, stderr=subprocess.STDOUT)
    with open(_state_file(), 'w') as f:
        f.write(f'{x.pid} {d.pid}\n')
    for _ in range(30):
        time.sleep(1)
        if win():
            break
    w = win()
    subprocess.run(['xdotool', 'windowsize', w, '640', '400'], env=ENV)
    subprocess.run(['xdotool', 'windowmove', w, '0', '0'], env=ENV)


def keys(ks, delay=0.15):
    w = win()
    for k in ks:
        if k.startswith('sleep'):
            time.sleep(float(k[5:]))
            continue
        subprocess.run(['xdotool', 'key', '--window', w, k], env=ENV)
        time.sleep(delay)


def top():
    """The y of the 640x400 picture in the window (dosbox letterboxes it in 640x480)."""
    out = subprocess.run(['xdotool', 'getwindowgeometry', win()], env=ENV, capture_output=True, text=True).stdout
    h = int(out.split('Geometry:')[1].split('x')[1].split()[0])
    return max(0, (h - 400) // 2)


def shot(path):
    subprocess.run(['import', '-display', DISPLAY, '-window', 'root', '-crop', '640x400+0+%d' % top(), path], env=ENV)


def park():
    """Push the captured mouse pointer into the top-left corner (where the driver clamps
    it), out of the way of a menu that follows the pointer."""
    for _ in range(6):
        subprocess.run(['xdotool', 'mousemove_relative', '--', '-150', '-150'], env=ENV)
        time.sleep(0.1)
    time.sleep(0.3)


def click(x, y, button=1):
    """A mouse click at (x, y) of CIV.EXE's 320x200 screen. The captured mouse only moves
    relatively (one picture pixel per host pixel), so the pointer is first pushed into the
    top-left corner, where the driver clamps it, and then moved by (2x, 2y)."""
    park()
    subprocess.run(['xdotool', 'mousemove_relative', '--', str(2 * x + 1), str(2 * y + 1)], env=ENV)
    time.sleep(0.5)
    subprocess.run(['xdotool', 'mousedown', str(button), 'sleep', '0.2', 'mouseup', str(button)], env=ENV)
    time.sleep(0.5)


def stop():
    try:
        pids = open(_state_file()).read().split()
    except OSError:
        return
    for p in reversed(pids):
        subprocess.run(['kill', '-9', p], stderr=subprocess.DEVNULL)
    os.remove(_state_file())


if __name__ == '__main__':
    cmd = sys.argv[1]
    if cmd == 'start':
        start(*sys.argv[2:4])
    elif cmd == 'keys':
        keys(sys.argv[2:])
    elif cmd == 'click':
        click(int(sys.argv[2]), int(sys.argv[3]))
    elif cmd == 'shot':
        shot(sys.argv[2])
    elif cmd == 'stop':
        stop()
