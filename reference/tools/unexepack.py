#!/usr/bin/env python3
"""Unpack the EXEPACK-compressed CIV.EXE for disassembly.

    python3 unexepack.py ../../../civ/CIV.EXE civ.bin
    objdump -D -b binary -m i386 -M i8086,intel --start-address=0x8b7 --stop-address=0x9d7 civ.bin

Writes `civ.bin` (the load image, 201,520 bytes; a far address seg:off is at seg*16+off, the
data segment DS is 0x2a80 -> 0x2a800) and `civ.bin.overlay` (the 23 MS C overlays appended
after the EXE image, each an MZ header + code invoked through `int 3Fh`; their far calls to
segment 0 are unrelocated and mean the main image's first segment).

Graphics driver interface: MGRAPHIC.EXE (DOS loads it with int 21h/4B03h) has a header at
code offset 0x24: [0x28] code segment (relocated), [0x2e] first stub index, [0x30] entry
count (52), [0x32..] entry offsets. CIV.EXE patches them into 7-byte far-jump stubs at
0:0x76d + 7*i (jmp at +2), so entry i is called as `call 0:0x76f+7*i`. Entry 10 (driver
0x6b7) is the pixel dissolve: `push duration; push page; call 0:0x7b5`.
"""
import struct, sys
src_path, out_path = sys.argv[1], sys.argv[2]
d = open(src_path, 'rb').read()
e_cblp, e_cp, e_crlc, e_cparhdr = struct.unpack_from('<4H', d, 2)
e_ip, e_cs = struct.unpack_from('<2H', d, 0x14)
hdr = e_cparhdr * 16
exe_len = (e_cp - 1) * 512 + e_cblp if e_cblp else e_cp * 512
img = bytearray(d[hdr:exe_len])
ep = e_cs * 16  # exepack header position in image
real_ip, real_cs, mem_start, exepack_size, real_sp, real_ss, dest_len = struct.unpack_from('<7H', img, ep)
sig = img[ep + 14:ep + 16]
print(f'exe_len={exe_len:#x} image={len(img):#x} exepack hdr @{ep:#x} sig={sig} real cs:ip={real_cs:#x}:{real_ip:#x} ss:sp={real_ss:#x}:{real_sp:#x} dest_len={dest_len * 16:#x} exepack_size={exepack_size:#x}')
packed = bytes(img[:ep])
dst_len = dest_len * 16
out = bytearray(dst_len)
# packed data is decompressed in place from the end; emulate with separate buffers
src = len(packed)
while src > 0 and packed[src - 1] == 0xFF:
    src -= 1
dst = dst_len
# the packed data occupies the low part of the destination (in-place), so copy it first
out[:len(packed)] = packed
n = 0
while True:
    cmd = out[src - 1]; src -= 1
    length = out[src - 2] | (out[src - 1] << 8); src -= 2
    if cmd & 0xFE == 0xB0:
        fill = out[src - 1]; src -= 1
        dst -= length
        out[dst:dst + length] = bytes([fill]) * length
    elif cmd & 0xFE == 0xB2:
        dst -= length; src -= length
        out[dst:dst + length] = out[src:src + length]
    else:
        print(f'bad command {cmd:#x} at src={src:#x} dst={dst:#x} after {n} cmds'); break
    n += 1
    if cmd & 1:
        break
print(f'done: {n} commands, dst={dst:#x} src={src:#x}')
# relocation table (after unpacker code): apply nothing; far-call segments stay load-relative
open(out_path, 'wb').write(out)
# overlay after the exe image
ov = d[exe_len:]
print(f'overlay after image: {len(ov)} bytes at {exe_len:#x}')
open(out_path + '.overlay', 'wb').write(ov)
