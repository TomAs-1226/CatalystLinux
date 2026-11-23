/*
 * Catalyst Coprocessor speech front end; see fe.h for the API and
 * features.py for the reference this matches (test_fe.py checks it).
 *
 * The 512-point real FFT is a 256-point complex radix-2 FFT of the
 * even/odd samples packed as re/im, then the usual split step. All tables
 * are built by fe_init with cosf/log10f; everything is float32 (no double
 * anywhere, so it suits MaixPy's -march=rv64imafc -mabi=lp64f build).
 */
#include "fe.h"

#include <math.h>
#include <stddef.h>
#include <string.h>

#define FE_HALF (FE_WIN / 2)
#define STEP (6.28318530717958648f / (float)FE_WIN)   /* 2 pi / 512 */

static float hz_to_mel(float f)
{
    return 2595.0f * log10f(1.0f + f / 700.0f);
}

void fe_init(fe_t *s)
{
    int k, j;
    float mel_lo, mel_hi, edge[FE_BANDS + 2], bin_mel[FE_BINS];
    int off = 0;

    memset(s, 0, sizeof(*s));
    for (k = 0; k < FE_WIN; k++)
        s->hann[k] = 0.5f - 0.5f * cosf(STEP * (float)k);
    for (k = 0; k < FE_HALF; k++)
        s->cos_t[k] = cosf(STEP * (float)k);
    /* sin(2 pi k / 512) = cos(2 pi (128 - k) / 512), and symmetric about 128 */
    for (k = 0; k < FE_HALF; k++)
        s->sin_t[k] = s->cos_t[k < FE_HALF / 2 ? FE_HALF / 2 - k : k - FE_HALF / 2];

    mel_lo = hz_to_mel(60.0f);
    mel_hi = hz_to_mel(7600.0f);
    for (j = 0; j < FE_BANDS + 2; j++)
        edge[j] = mel_lo + (mel_hi - mel_lo) * (float)j / (float)(FE_BANDS + 1);
    for (k = 0; k < FE_BINS; k++)
        bin_mel[k] = hz_to_mel((float)k * ((float)FE_RATE / (float)FE_WIN));
    for (j = 0; j < FE_BANDS; j++) {
        float lo = edge[j], c = edge[j + 1], hi = edge[j + 2];
        int first = -1, n = 0;
        for (k = 0; k < FE_BINS; k++) {
            float up = (bin_mel[k] - lo) / (c - lo);
            float down = (hi - bin_mel[k]) / (hi - c);
            float w = up < down ? up : down;
            if (w > 0.0f && off + n < FE_MELW) {
                if (first < 0)
                    first = k;
                s->melw[off + n] = w;
                n++;
            } else if (first >= 0) {
                break;
            }
        }
        s->mel_first[j] = (int16_t)(first < 0 ? 0 : first);
        s->mel_n[j] = (int16_t)n;
        s->mel_off[j] = (int16_t)off;
        off += n;
    }

    s->vad_floor = -45.0f;   /* lowered to frame 0's energy in vad_step */
    s->vad_last = -1;
}

/* In-place 256-point complex FFT (forward, e^{-i...}). */
static void fft256(const fe_t *s, float *re, float *im)
{
    int i, j, len;
    for (i = 0, j = 0; i < FE_HALF; i++) {
        int bit;
        if (i < j) {
            float t = re[i]; re[i] = re[j]; re[j] = t;
            t = im[i]; im[i] = im[j]; im[j] = t;
        }
        for (bit = FE_HALF >> 1; j & bit; bit >>= 1)
            j ^= bit;
        j |= bit;
    }
    for (len = 2; len <= FE_HALF; len <<= 1) {
        int half = len >> 1, step = FE_WIN / len, m;
        for (m = 0; m < half; m++) {
            float wr = s->cos_t[m * step], wi = -s->sin_t[m * step];
            for (i = m; i < FE_HALF; i += len) {
                int q = i + half;
                float xr = re[q] * wr - im[q] * wi;
                float xi = re[q] * wi + im[q] * wr;
                re[q] = re[i] - xr;
                im[q] = im[i] - xi;
                re[i] += xr;
                im[i] += xi;
            }
        }
    }
}

static void vad_step(fe_t *s, int k, float e)
{
    int sp;
    /* The first frame is the room (push-to-talk opens the microphone a
     * moment before the first word): the floor starts there, capped at
     * -45 dB so a word said at once still counts. */
    if (k == 0 && e < s->vad_floor)
        s->vad_floor = e;
    sp = e > s->vad_floor + 12.0f && e > -60.0f;
    if (s->vad_state == 0) {
        s->vad_run = sp ? s->vad_run + 1 : 0;
        if (s->vad_run >= 3) {
            int st = k - 2 - 5;
            s->vad_state = 1;
            s->vad_start = st < 0 ? 0 : st;
            s->vad_last = k;
            s->vad_speech = 3;
        }
    } else if (sp) {
        s->vad_last = k;
        s->vad_speech++;
        s->vad_state = 1;
    } else if (k - s->vad_last >= 30) {
        s->vad_state = 2;
    }
    if (e < s->vad_floor)
        s->vad_floor = e;
    else
        s->vad_floor += 0.05f;
}

/* One frame from pend[0..511]. */
static void fe_frame(fe_t *s)
{
    float re[FE_HALF], im[FE_HALF], pw[FE_BINS];
    float acc = 0.0f, p, e;
    int n, k, j, slot = s->count % FE_RING;
    float *out = s->logmel[slot];

    for (n = 0; n < FE_WIN; n++)
        acc += s->pend[n] * s->pend[n];
    for (n = 0; n < FE_HALF; n++) {
        re[n] = s->pend[2 * n] * s->hann[2 * n];
        im[n] = s->pend[2 * n + 1] * s->hann[2 * n + 1];
    }
    fft256(s, re, im);
    /* split: X[k] = E[k] + W^k O[k], E = (Z[k] + Z*[256-k]) / 2,
     * O = -i (Z[k] - Z*[256-k]) / 2, W = e^{-2 pi i / 512} */
    pw[0] = (re[0] + im[0]) * (re[0] + im[0]);
    pw[FE_HALF] = (re[0] - im[0]) * (re[0] - im[0]);
    for (k = 1; k < FE_HALF; k++) {
        float ar = re[k], ai = im[k];
        float br = re[FE_HALF - k], bi = -im[FE_HALF - k];
        float er = 0.5f * (ar + br), ei = 0.5f * (ai + bi);
        float or_ = 0.5f * (ai - bi), oi = -0.5f * (ar - br);
        float wr = s->cos_t[k], wi = -s->sin_t[k];
        float xr = er + (or_ * wr - oi * wi);
        float xi = ei + (or_ * wi + oi * wr);
        pw[k] = xr * xr + xi * xi;
    }
    for (j = 0; j < FE_BANDS; j++) {
        const float *w = s->melw + s->mel_off[j];
        const float *b = pw + s->mel_first[j];
        float m = 0.0f;
        for (k = 0; k < s->mel_n[j]; k++)
            m += w[k] * b[k];
        out[j] = logf(m + 1e-6f);
    }
    p = acc / (float)FE_WIN + 1e-10f;
    e = 10.0f * log10f(p);
    s->power[slot] = p;
    s->energy[slot] = e;
    vad_step(s, s->count, e);
    s->count++;
}

int fe_push(fe_t *s, const int16_t *pcm, int n)
{
    int i, added = 0;
    for (i = 0; i < n; i++) {
        s->pend[s->npend++] = (float)pcm[i] * (1.0f / 32768.0f);
        if (s->npend == FE_WIN) {
            fe_frame(s);
            memmove(s->pend, s->pend + FE_HOP, (FE_WIN - FE_HOP) * sizeof(float));
            s->npend = FE_WIN - FE_HOP;
            added++;
        }
    }
    return added;
}

int fe_count(const fe_t *s)
{
    return s->count;
}

static int kept(const fe_t *s, int k)
{
    return k >= 0 && k < s->count && k >= s->count - FE_RING;
}

const float *fe_logmel(const fe_t *s, int k)
{
    return kept(s, k) ? s->logmel[k % FE_RING] : NULL;
}

float fe_energy(const fe_t *s, int k)
{
    return kept(s, k) ? s->energy[k % FE_RING] : -100.0f;
}

float fe_power(const fe_t *s, int k)
{
    return kept(s, k) ? s->power[k % FE_RING] : 0.0f;
}

void fe_vad(const fe_t *s, int *state, int *start, int *last, int *speech)
{
    if (state) *state = s->vad_state;
    if (start) *start = s->vad_start;
    if (last) *last = s->vad_last;
    if (speech) *speech = s->vad_speech;
}

/* Each candidate's sum is added up afresh, in order, in float (at most
 * 465 x 48 adds): no drift, and features.py reproduces it bit for bit. */
int fe_best_window(const fe_t *s, int from, int to)
{
    int count = s->count, oldest, lo, hi, k, w, best_s;
    float acc, best = 0.0f;
    if (count <= 0)
        return 0;
    oldest = count > FE_RING ? count - FE_RING : 0;
    if (from < oldest) from = oldest;
    if (from > count - 1) from = count - 1;
    if (to < oldest) to = oldest;
    if (to > count - 1) to = count - 1;
    if (to < from) to = from;
    if (count - oldest < FE_WINDOW)
        return oldest;
    if (to - from + 1 >= FE_WINDOW) {
        lo = from;
        hi = to - FE_WINDOW + 1;
    } else {
        lo = to - FE_WINDOW + 1 > oldest ? to - FE_WINDOW + 1 : oldest;
        hi = from < count - FE_WINDOW ? from : count - FE_WINDOW;
    }
    best_s = lo;
    for (w = lo; w <= hi; w++) {
        acc = 0.0f;
        for (k = w; k < w + FE_WINDOW; k++)
            acc += s->power[k % FE_RING];
        if (w == lo || acc > best) {
            best = acc;
            best_s = w;
        }
    }
    return best_s;
}

void fe_window(const fe_t *s, int start, const float *mean,
               const float *inv_std, float *out)
{
    const float silence = logf(1e-6f);
    int r, j;
    for (r = 0; r < FE_WINDOW; r++) {
        const float *l = fe_logmel(s, start + r);
        float *o = out + r * FE_BANDS;
        for (j = 0; j < FE_BANDS; j++) {
            float v = l ? l[j] : silence;
            o[j] = mean ? (v - mean[j]) * inv_std[j] : v;
        }
    }
}
