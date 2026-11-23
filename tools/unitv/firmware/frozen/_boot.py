# Catalyst Coprocessor: what the firmware runs at every start (frozen).
#
# Always the program: /flash/main.py (the handheld sends newer ones over the
# Grove port; catalyst_boot rolls back one that never reaches the handheld).
# Never MaixPy's /flash/boot.py, nor a demo. If main.py is missing or stops
# with an error, the safe program (catalyst_safe, frozen) answers the
# handheld instead, says why, and takes a new main.py over the link, so a
# bad program never leaves the camera deaf. A Ctrl-C on the USB serial line
# (the handheld's installer, over USB-A) stops either and leaves MaixPy's
# REPL.

import os
import sys

try:
    os.chdir("/flash")
except OSError:
    pass
sys.path[:] = ["", "/flash"]

import catalyst

# Delays sleep from here on: the CPU waits for an interrupt between looks.
catalyst.power(sleep=1)

import catalyst_boot

_boot_said = catalyst_boot.run("/flash")
del catalyst_boot
_why = None
try:
    with open("/flash/main.py") as _f:
        _source = _f.read()
except OSError:
    _source = None
    _why = "no main.py"
if _source is not None:
    try:
        _code = compile(_source, "main.py", "exec")
        del _source
        exec(_code, {"__name__": "__main__", "BOOT_SAID": _boot_said})
        _why = "main.py ended"
    except KeyboardInterrupt:
        raise
    except BaseException as _e:
        _why = "main.py: %s %s" % (type(_e).__name__, _e)
        try:
            sys.print_exception(_e)
        except Exception:
            pass
    _code = None

import catalyst_safe

catalyst_safe.run("/flash", _why, _boot_said)
