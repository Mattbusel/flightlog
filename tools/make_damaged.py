"""Makes deterministic damaged copies of a DataFlash log for recovery tests.

    python tools/make_damaged.py testdata/external/copter-mission.bin

Writes next to the input:
  <name>-truncated.bin  cut off in the middle of a message
  <name>-zeroblock.bin  two 4 KiB runs of zero bytes (as left by interrupted
                        block-based logging or log download over MAVLink)
  <name>-bitflip.bin    64 bytes inverted at pseudo-random offsets
  <name>-garbage.bin    1000 pseudo-random bytes inserted mid-file
The same input always gives the same outputs (fixed-seed LCG).
"""
import pathlib
import sys


def lcg(seed):
    x = seed
    while True:
        x = (x * 6364136223846793005 + 1442695040888963407) % (1 << 64)
        yield x >> 33


def main():
    src = pathlib.Path(sys.argv[1])
    data = src.read_bytes()
    n = len(data)
    stem = src.with_suffix("")

    def write(tag, b):
        out = pathlib.Path(f"{stem}-{tag}{src.suffix}")
        out.write_bytes(bytes(b))
        print(out, len(b))

    write("truncated", data[: n * 6 // 10 + 3])

    z = bytearray(data)
    for at in (n // 5, n * 3 // 5):
        z[at : at + 4096] = bytes(4096)
    write("zeroblock", z)

    f = bytearray(data)
    r = lcg(1)
    for _ in range(64):
        f[next(r) % n] ^= 0xFF
    write("bitflip", f)

    r = lcg(2)
    junk = bytes(next(r) & 0xFF for _ in range(1000))
    at = n // 2 + 7
    write("garbage", data[:at] + junk + data[at:])


if __name__ == "__main__":
    main()
