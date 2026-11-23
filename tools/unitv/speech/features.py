"""Catalyst Coprocessor speech front end: the numpy reference.

fe.c (the K210's streaming C version) computes exactly this; test_fe.py
checks the two against each other. Training (train.py) and evaluation
(evaluate.py) use these functions, so the model sees what the K210 sees.

Input: 16 kHz mono int16 PCM, x = sample / 32768.

Frames: a 512-sample window every 320 samples (20 ms); frame k covers
samples [320k, 320k + 512), so N samples give (N - 512) // 320 + 1 frames.

Per frame:
  - power  p[k] = mean(x^2 over the 512 unwindowed samples) + 1e-10
  - energy E[k] = 10 log10 p[k] dB (for the VAD and the window choice)
  - periodic Hann w[n] = 0.5 - 0.5 cos(2 pi n / 512), real 512-point FFT
    (no zero padding), power spectrum P[b] = re^2 + im^2, b = 0..256
  - 40 triangular mel filters on P. HTK mel, m(f) = 2595 log10(1 + f/700);
    42 edges evenly spaced in mel from 60 Hz to 7600 Hz; filter j rises
    from edge j to a peak of 1.0 at edge j+1 and falls to 0 at edge j+2.
    The triangles are linear in mel (as HTK's own filterbank and
    TensorFlow's linear_to_mel_weight_matrix are), so bin b's weight comes
    from its mel value m(b * 16000 / 512). No area normalisation.
  - log-mel L[k][j] = ln(mel_j + 1e-6)

VAD (streaming, one step per frame, in float32 like fe.c):
  the floor starts at frame 0's own energy, but no higher than -45 dB
  (push-to-talk opens the microphone a moment before the first word, so
  the first frame is the room; capped, so a word said at once still counts).
  Frame k is speech if E > floor + 12 and E > -60; then the floor moves:
  floor = E if E < floor, else floor += 0.05.
  Onset: 3 consecutive speech frames k-2, k-1, k; start = k - 2 - 5
  (not below 0), last = k, speech = 3, state = 1. Afterwards every speech
  frame sets last = k, counts in speech, and clears `ended` (state 1);
  once k - last reaches 30 (30 non-speech frames after the last speech
  frame) state = 2 (ended). Report (state, start, last, speech).

KWS window: 48 consecutive frames (0.96 s), chosen by best_window():
  frames still available are [oldest, count) with oldest = max(0,
  count - 512) (fe.c keeps a ring of 512 frames). The range [frm, to] is
  clamped to them. If it holds at least 48 frames, the window start s
  runs over [frm, to - 47]; if it holds fewer, s runs over the windows
  that contain the whole range, [max(oldest, to - 47), min(frm,
  count - 48)]; if fewer than 48 frames exist at all, s = oldest. The
  chosen s maximises the sum of p over the window (each candidate's 48
  powers added in order in float32, ties to the earliest s). Rows of the
  window past the frames that exist are silence, ln(1e-6) in every band.

The rule the K210 uses (and training simulates) to pick the range:
  frm = VAD start if the VAD has seen speech (state > 0), else 0;
  to = the last frame (count - 1). See choose_window().

Model input: the 48 x 40 block, row = frame, normalised per band,
  (L - mean[j]) * inv_std[j], with mean/inv_std from kws.json.
"""

import numpy as np

SAMPLE_RATE = 16000
WIN = 512
HOP = 320
NBINS = WIN // 2 + 1
BANDS = 40
FMIN = 60.0
FMAX = 7600.0
LOG_EPS = 1e-6
POW_EPS = 1e-10
RING = 512
WINDOW_FRAMES = 48
SILENCE = float(np.float32(np.log(np.float32(LOG_EPS))))

VAD_FLOOR_CAP = -45.0
VAD_RISE = 0.05
VAD_MARGIN = 12.0
VAD_MIN_DB = -60.0
VAD_ONSET = 3
VAD_PREROLL = 5
VAD_HANG = 30


def hz_to_mel(f):
    return 2595.0 * np.log10(1.0 + np.asarray(f, dtype=np.float64) / 700.0)


def mel_matrix():
    """40 x 257 filter weights (float64), triangles linear in HTK mel."""
    m_lo, m_hi = hz_to_mel(FMIN), hz_to_mel(FMAX)
    edges = m_lo + (m_hi - m_lo) * np.arange(BANDS + 2) / (BANDS + 1)
    bin_mel = hz_to_mel(np.arange(NBINS) * SAMPLE_RATE / WIN)
    w = np.zeros((BANDS, NBINS))
    for j in range(BANDS):
        lo, c, hi = edges[j], edges[j + 1], edges[j + 2]
        up = (bin_mel - lo) / (c - lo)
        down = (hi - bin_mel) / (hi - c)
        w[j] = np.maximum(0.0, np.minimum(up, down))
    return w


HANN = 0.5 - 0.5 * np.cos(2.0 * np.pi * np.arange(WIN) / WIN)
MEL = mel_matrix()


def n_frames(n_samples):
    return 0 if n_samples < WIN else (n_samples - WIN) // HOP + 1


def analyse(pcm):
    """int16 PCM -> (logmel [K, 40] float32, energy dB [K] float32,
    power [K] float64). Batched: pcm may be [..., N]."""
    pcm = np.asarray(pcm)
    x = pcm.astype(np.float64) / 32768.0
    k = n_frames(x.shape[-1])
    idx = HOP * np.arange(k)[:, None] + np.arange(WIN)[None, :]
    fr = x[..., idx]
    power = np.mean(fr * fr, axis=-1) + POW_EPS
    energy = (10.0 * np.log10(power)).astype(np.float32)
    spec = np.fft.rfft(fr * HANN, n=WIN, axis=-1)
    p = spec.real ** 2 + spec.imag ** 2
    mel = p @ MEL.T
    logmel = np.log(mel + LOG_EPS).astype(np.float32)
    return logmel, energy, power


def vad_trace(energy, history=None):
    """Run the streaming VAD over the energies (float32, as fe.c).
    Returns the final (state, start, last, speech); if `history` is a
    list, the tuple after every frame is appended to it."""
    f32 = np.float32
    floor = f32(VAD_FLOOR_CAP)
    margin, min_db, rise = f32(VAD_MARGIN), f32(VAD_MIN_DB), f32(VAD_RISE)
    state, start, last, speech, run = 0, 0, -1, 0, 0
    for k, e in enumerate(np.asarray(energy, dtype=np.float32)):
        e = f32(e)
        if k == 0 and e < floor:
            floor = e
        sp = bool(e > f32(floor + margin)) and bool(e > min_db)
        if state == 0:
            run = run + 1 if sp else 0
            if run >= VAD_ONSET:
                state = 1
                start = max(0, k - (VAD_ONSET - 1) - VAD_PREROLL)
                last = k
                speech = VAD_ONSET
        else:
            if sp:
                last = k
                speech += 1
                state = 1
            elif k - last >= VAD_HANG:
                state = 2
        floor = e if e < floor else f32(floor + rise)
        if history is not None:
            history.append((state, start, last, speech))
    return state, start, last, speech


def best_window(power, frm, to, count=None):
    """Start frame of the best 48-frame window within [frm, to] (see the
    module docstring). power: per-frame p (index = frame number; frames
    before oldest are never read)."""
    if count is None:
        count = len(power)
    if count <= 0:
        return 0
    oldest = max(0, count - RING)
    frm = min(max(frm, oldest), count - 1)
    to = min(max(to, oldest), count - 1)
    if to < frm:
        to = frm
    n = WINDOW_FRAMES
    if count - oldest < n:
        return oldest
    if to - frm + 1 >= n:
        lo, hi = frm, to - n + 1
    else:
        lo, hi = max(oldest, to - n + 1), min(frm, count - n)
    # each candidate summed in order in float32 (cumsum is sequential), as fe.c
    p = np.asarray(power, dtype=np.float32)
    idx = np.arange(lo, hi + 1)[:, None] + np.arange(n)[None, :]
    sums = np.cumsum(p[idx], axis=1, dtype=np.float32)[:, -1]
    return lo + int(np.argmax(sums))


def window(logmel, start, mean=None, inv_std=None, count=None):
    """The 48 x 40 model input starting at frame `start` (rows past the
    available frames are silence), normalised if mean/inv_std given."""
    if count is None:
        count = len(logmel)
    oldest = max(0, count - RING)
    out = np.full((WINDOW_FRAMES, BANDS), SILENCE, dtype=np.float32)
    lo, hi = max(start, oldest), min(start + WINDOW_FRAMES, count)
    if hi > lo:
        out[lo - start:hi - start] = logmel[lo:hi]
    if mean is not None:
        out = ((out - np.asarray(mean, np.float32)) *
               np.asarray(inv_std, np.float32)).astype(np.float32)
    return out


def choose_window(energy, power):
    """The K210's rule: the range is [VAD start (or 0), last frame]."""
    count = len(energy)
    state, start, _, _ = vad_trace(energy)
    frm = start if state > 0 else 0
    return best_window(power, frm, count - 1, count)


def kws_input(pcm, mean=None, inv_std=None):
    """Whole stream of int16 PCM -> the (normalised) 48 x 40 model input."""
    logmel, energy, power = analyse(pcm)
    s = choose_window(energy, power)
    return window(logmel, s, mean, inv_std)
