# Catalyst Coprocessor: finding the handheld on the Grove port (frozen).
#
# The Grove port's two pins are G34 and G35. Which is which depends on the
# cable, so both only listen (UART1 on 35, UART2 on 34) until the handheld's
# hello arrives on one; that one is this end's RX, the other its TX. The USB
# serial line (MaixPy's REPL) listens too, so the handheld can check this
# answers right after installing it over USB-A; a Ctrl-C there still stops
# it. Nothing is driven on a Grove pin before then.

import time

from catalyst_link import HELLO, Decoder

START_BAUD = 115200


def find(poll_ms=20, on_wait=None):
    """Listens until a hello arrives: (uart, via_usb, rx_pin, tx_pin, the
    frames that came with the hello). Sleeps between looks (the CPU waits
    for an interrupt meanwhile, with the Catalyst firmware)."""
    from fpioa_manager import fm
    from machine import UART

    fm.register(35, fm.fpioa.UART1_RX, force=True)
    fm.register(34, fm.fpioa.UART2_RX, force=True)
    listen = [
        (UART(UART.UART1, START_BAUD, 8, None, 1, timeout=0, read_buf_len=8192), 35, 34),
        (UART(UART.UART2, START_BAUD, 8, None, 1, timeout=0, read_buf_len=8192), 34, 35),
    ]
    try:
        usb = UART.repl_uart()
    except Exception:
        usb = None
    heard = [Decoder(), Decoder()]
    usb_heard = Decoder()
    found = None
    while found is None:
        for i, (port, rx, tx) in enumerate(listen):
            if port.any():
                data = port.read()
                frames = heard[i].push(data) if data else []
                if any(f[0] == HELLO for f in frames):
                    found = (port, False, rx, tx, frames)
                    break
        if found is None and usb is not None:
            try:
                data = usb.read() if usb.any() else None
            except Exception:
                data = None
            frames = usb_heard.push(data) if data else []
            if any(f[0] == HELLO for f in frames):
                found = (usb, True, None, None, frames)
        if found is None:
            if on_wait is not None:
                on_wait()
            time.sleep_ms(poll_ms)
    uart, via_usb, rx_pin, tx_pin, frames = found
    for port, rx, tx in listen:
        if port is not uart:
            port.deinit()
    if not via_usb:
        # Older MaixPy wants unregister(pin, function); force=True replaces
        # the listening function either way.
        try:
            fm.unregister(tx_pin)
        except Exception:
            pass
        fm.register(tx_pin, fm.fpioa.UART1_TX if rx_pin == 35 else fm.fpioa.UART2_TX, force=True)
    return uart, via_usb, rx_pin, tx_pin, frames
