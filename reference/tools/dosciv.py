#!/usr/bin/env python3
"""Drive the original CIV.EXE in dosbox-staging under a private Xvfb display.

    python3 -I dosciv.py start GAME_DIR          start Xvfb + dosbox running CIV.EXE in GAME_DIR
    python3 -I dosciv.py keys K1 K2 ...          send xdotool keys (sleepN.N pauses)
    python3 -I dosciv.py shot FILE.png           screenshot of the 640x400 window
    python3 -I dosciv.py stop                    kill the dosbox and Xvfb this tool started

Environment: CIVDISPLAY (default :79). The pids of the started processes are kept in
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
    with open(conf, 'w') as f:
        f.write(f"""[sdl]
fullscreen = false
output = texture
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
[autoexec]
mount c {game}
c:
civ
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


def shot(path):
    subprocess.run(['import', '-display', DISPLAY, '-window', 'root', '-crop', '640x400+0+0', path], env=ENV)


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
    elif cmd == 'shot':
        shot(sys.argv[2])
    elif cmd == 'stop':
        stop()
