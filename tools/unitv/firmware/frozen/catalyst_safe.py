# Catalyst Coprocessor: the safe program (frozen), run when /flash/main.py is
# missing or stopped with an error. It answers the handheld at 115 200 baud,
# says why in its hello ("safe" and "why"), answers pings and health, and
# takes a new program over the link (the same PUT/CHUNK/DONE/COMMIT as
# main.py), then starts again with it. The light is a slow red pulse.

import time

import catalyst
import catalyst_link as L
import catalyst_port

try:
    import ujson as json
except ImportError:
    import json


def _lamp():
    try:
        from modules import ws2812

        light = ws2812(8, 1)
    except Exception:
        light = None

    def show(level):
        if light is None:
            return
        try:
            light.set_led(0, (level, 0, 0))
            light.display()
        except Exception:
            pass

    return show


def run(root, why, boot_said):
    show = _lamp()
    show(6)
    if boot_said.startswith("trial"):
        # A new program on trial that stopped: started again at once, so
        # the boot counts its tries and puts the old program back.
        time.sleep_ms(300)
        import machine

        machine.reset()
    started = time.ticks_ms()
    stamp = None
    try:
        with open(root + "/catalyst.json") as f:
            stamp = json.loads(f.read())
    except Exception:
        stamp = None
    lit = [0]

    def blink():
        lit[0] = (lit[0] + 1) % 50
        show(6 if lit[0] < 25 else 1)

    uart, via_usb, _rx, _tx, first = catalyst_port.find(poll_ms=40, on_wait=blink)
    show(4)
    files = L.Files(root)
    decoder = L.Decoder()
    seq = [0]

    def send(kind, payload):
        seq[0] = (seq[0] + 1) & 0xFF
        uart.write(L.frame(kind, seq[0], payload))

    def hello():
        send(
            L.HELLO_BACK,
            json.dumps(
                {
                    "v": 0,
                    "safe": 1,
                    "why": why,
                    "boot": boot_said,
                    "fw": catalyst.version(),
                    "fwv": catalyst.FIRMWARE,
                    "upd": 1,
                    "full": 1,
                    "usb": via_usb,
                    "cwd": root,
                    "free": L.free_bytes(root),
                    "up_ms": time.ticks_diff(time.ticks_ms(), started),
                    "stamp": stamp,
                }
            ).encode(),
        )

    pending = list(first)
    while True:
        if uart.any():
            data = uart.read()
            if data:
                pending.extend(decoder.push(data))
        for kind, _s, payload in pending:
            if kind == L.HELLO:
                hello()
            elif kind == L.PING:
                send(L.PONG, payload)
            elif kind == L.HEALTH:
                send(L.HEALTH_BACK, json.dumps({"state": "safe", "why": why, "heap": catalyst.heap(), "t": catalyst.time_ms()}).encode())
            elif kind in (L.PUT, L.CHUNK, L.DONE, L.COMMIT):
                back, said, committed = L.handle_file(files, kind, payload, json.loads, json.dumps)
                send(back, said)
                if committed:
                    time.sleep_ms(100)
                    import machine

                    machine.reset()
            else:
                send(L.LOG, b"safe program: " + (why or "?").encode()[:160])
        pending = []
        time.sleep_ms(10)
