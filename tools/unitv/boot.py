# Catalyst Linux: what runs on the smart camera before main.py (MaixPy runs
# /flash/boot.py, then /flash/main.py, at every start; a firmware's own demo
# boot.py, which can loop for ever, is replaced by this one).
#
# It keeps a new program from leaving the camera dead. When the handheld
# sends a newer main.py (and face model) over the Grove port, main.py moves
# the files into place with the old ones kept as name.old and begins a trial
# (/flash/catalyst.trial: a count, then each file and whether it was there
# before). Each start counts here; main.py ends the trial once the handheld
# has heard it. A new program that starts twice without that gets the old
# files back on the third start (and catalyst.rolledback says so in its
# hello). An update cut off while its files were being moved is finished
# first, from its journal (catalyst.commit).
#
# Only `os`, so it runs on any MaixPy, and on a computer for the tests
# (tools/unitv/test_protocol.py).

import os

ROOT = "/flash"
TRIES = 2


def _exists(path):
    try:
        os.stat(path)
        return True
    except OSError:
        return False


def _remove(path):
    try:
        os.remove(path)
    except OSError:
        pass


def _finish_commit(root):
    try:
        with open(root + "/catalyst.commit") as f:
            lines = f.read().split("\n")
    except OSError:
        return
    kept = []
    for line in lines:
        parts = line.split()
        if len(parts) != 2:
            continue
        kept.append(line.strip())
        cur = root + "/" + parts[0]
        if _exists(cur + ".new"):
            if parts[1] == "1":
                if _exists(cur):
                    _remove(cur + ".old")
                    os.rename(cur, cur + ".old")
            else:
                _remove(cur + ".old")
                _remove(cur)
            os.rename(cur + ".new", cur)
        _remove(cur + ".part")
    with open(root + "/catalyst.trial", "w") as f:
        f.write("0\n" + "\n".join(kept) + "\n")
    _remove(root + "/catalyst.rolledback")
    _remove(root + "/catalyst.commit")


def _roll_back(root, lines):
    for line in lines:
        parts = line.split()
        if len(parts) != 2:
            continue
        cur = root + "/" + parts[0]
        if parts[1] == "1":
            if _exists(cur + ".old"):
                _remove(cur)
                os.rename(cur + ".old", cur)
        else:
            _remove(cur)
    with open(root + "/catalyst.rolledback", "w") as f:
        f.write("rolled back\n")
    _remove(root + "/catalyst.trial")


def catalyst_boot(root):
    try:
        _finish_commit(root)
        try:
            with open(root + "/catalyst.trial") as f:
                lines = f.read().split("\n")
        except OSError:
            return
        try:
            count = int(lines[0])
        except ValueError:
            count = TRIES
        if count >= TRIES:
            _roll_back(root, lines[1:])
        else:
            with open(root + "/catalyst.trial", "w") as f:
                f.write("%d\n" % (count + 1) + "\n".join(lines[1:]))
    except Exception as e:
        print("catalyst boot.py:", e)


catalyst_boot(ROOT)
del catalyst_boot

# MaixPy as M5Stack ships it for the UnitV runs boot.py and not main.py: the
# camera sat at its prompt and never answered the Grove port (found on a
# UnitV M12, 23 November 2026). So boot.py starts main.py. main.py runs for
# ever, so a MaixPy that would have started it after boot.py never does it
# twice; Ctrl-C (the installer's raw REPL) still stops it.
try:
    import sensor  # only on the camera, not on a computer running the tests
except ImportError:
    sensor = None
if sensor is not None:
    del sensor
    try:
        exec(open(ROOT + "/main.py").read(), {"__name__": "__main__"})
    except Exception as e:
        print("catalyst boot.py: main.py stopped:", e)
