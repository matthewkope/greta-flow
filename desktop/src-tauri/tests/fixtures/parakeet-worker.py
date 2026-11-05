"""Protocol-only fixture. Two samples simulate a crash during recognition."""
import json
import struct
import sys

print(json.dumps({"ready": True, "protocol": 1}), flush=True)
first = True
while header := sys.stdin.buffer.read(4):
    count, = struct.unpack("<I", header)
    data = sys.stdin.buffer.read(count * 4)
    if len(data) != count * 4 or count == 2:
        sys.exit(1)
    if first and (count != 16_000 or any(data)):
        sys.exit(1)  # Every new worker must receive a silence warm-up first.
    first = False
    print(json.dumps({"text": "fixture"}), flush=True)
