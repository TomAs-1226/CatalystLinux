# Catalyst Coprocessor: keeping a new program from leaving the camera dead,
# at every start, before the program runs (frozen; _boot.py calls run()).
#
# When the handheld sends a newer main.py (and models) over the Grove port,
# the program moves the files into place with the old ones kept as name.old
# and begins a trial (/flash/catalyst.trial: a count, then each file and
# whether it was there before). Each start counts here; the program ends the
# trial once the handheld has heard it. A new program that starts twice
# without that gets the old files back on the third start (and
# catalyst.rolledback says so in its hello). An update cut off while its
# files were being moved is finished first, from its journal
# (catalyst.commit).
#
# Only `os` and catalyst_link, so it runs on a computer for the tests.

import os

from catalyst_link import exists, remove, settle

TRIES = 2


def _roll_back(root, lines):
    for line in lines:
        parts = line.split()
        if len(parts) != 2:
            continue
        cur = root + "/" + parts[0]
        if parts[1] == "1":
            if exists(cur + ".old"):
                remove(cur)
                os.rename(cur + ".old", cur)
        else:
            remove(cur)
    with open(root + "/catalyst.rolledback", "w") as f:
        f.write("rolled back\n")
    remove(root + "/catalyst.trial")


def run(root):
    """The journal finished, this start counted, a failed program rolled
    back. What happened, in a word (for the program's hello)."""
    said = "ok"
    try:
        if settle(root):
            said = "settled"
        try:
            with open(root + "/catalyst.trial") as f:
                lines = f.read().split("\n")
        except OSError:
            return said
        try:
            count = int(lines[0])
        except ValueError:
            count = TRIES
        if count >= TRIES:
            _roll_back(root, lines[1:])
            return "rolled back"
        with open(root + "/catalyst.trial", "w") as f:
            f.write("%d\n" % (count + 1) + "\n".join(lines[1:]))
        return "trial %d" % (count + 1)
    except Exception as e:
        return "boot: %s" % e
