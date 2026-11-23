/*
 * Catalyst Coprocessor speech front end (streaming log-mel + VAD + KWS window).
 *
 * Portable C99, float32 only (no double), no malloc, no globals; libm: logf,
 * log10f, cosf only (plus memset/memmove). One fe_t holds all state (94.5 KB
 * on RV64 and x86-64: the 512-frame ring of log-mels is 80 KB), so put it in
 * static storage, not on a small stack. fe_push uses about 3 KB of stack.
 * Cost per 20 ms frame: one 512-point real FFT plus 40 logf, about 20 k
 * float operations; fe_best_window is at most 465 x 48 adds.
 *
 * Input is 16 kHz mono int16 PCM. Frame k covers samples [320k, 320k + 512)
 * (512-sample periodic-Hann window, hop 320 = 20 ms). Per frame:
 *   power  p = mean(x^2) + 1e-10 over the 512 unwindowed samples, x = s/32768
 *   energy E = 10 log10 p dB
 *   log-mel L[j] = ln(mel_j + 1e-6), 40 HTK-mel triangles (60..7600 Hz,
 *          linear in mel, peak 1.0) on the 512-point FFT power spectrum.
 * features.py (numpy) is the reference and documents every detail; test_fe.py
 * checks this file against it.
 *
 * VAD: floor starts at frame 0's energy (at most -45 dB); a frame is speech if E > floor + 12 and
 * E > -60; then floor = E if E < floor, else floor += 0.05. 3 consecutive
 * speech frames start it (start = first of them - 5, not below 0); every
 * later speech frame moves `last`; 30 non-speech frames after `last` set
 * state 2 (ended), and speech after that sets state 1 again.
 *
 * KWS: fe_best_window() picks a 48-frame window (0.96 s) by energy and
 * fe_window() writes it as the model's 48 x 40 float input (row = frame).
 * The range to search, as training used it:
 *     fe_vad(s, &state, &start, &last, &speech);
 *     int w = fe_best_window(s, state ? start : 0, fe_count(s) - 1);
 *     fe_window(s, w, mean, inv_std, input);   (mean/inv_std from kws.json)
 */
#ifndef CATALYST_FE_H
#define CATALYST_FE_H

#include <stdint.h>

#define FE_RATE 16000
#define FE_WIN 512
#define FE_HOP 320
#define FE_BINS 257
#define FE_BANDS 40
#define FE_RING 512          /* frames kept, 10.24 s */
#define FE_WINDOW 48         /* frames in the KWS window, 0.96 s */
#define FE_MELW 520          /* room for the mel weights (513 are used) */

typedef struct {
    /* tables, set by fe_init */
    float hann[FE_WIN];
    float cos_t[FE_WIN / 2];         /* cos(2 pi k / 512), k = 0..255 */
    float sin_t[FE_WIN / 2];         /* sin(2 pi k / 512) */
    float melw[FE_MELW];             /* weights of band j: melw[mel_off[j]..] */
    int16_t mel_first[FE_BANDS];     /* first FFT bin of band j */
    int16_t mel_n[FE_BANDS];         /* bins in band j */
    int16_t mel_off[FE_BANDS];
    /* samples not yet in a frame */
    float pend[FE_WIN];
    int npend;
    /* frames */
    int count;                       /* frames since fe_init */
    float logmel[FE_RING][FE_BANDS];
    float energy[FE_RING];           /* dB */
    float power[FE_RING];            /* linear, = 10^(E/10) */
    /* VAD */
    float vad_floor;
    int vad_state, vad_start, vad_last, vad_speech, vad_run;
} fe_t;

/* Reset everything (frames, VAD) and build the tables. */
void fe_init(fe_t *s);

/* Add n samples (any n >= 0, cut anywhere); returns the frames this added. */
int fe_push(fe_t *s, const int16_t *pcm, int n);

/* Frames produced since fe_init (frame numbers are 0..count-1). */
int fe_count(const fe_t *s);

/* The 40 log-mels of frame k, or NULL if k is not (or no longer) kept:
 * frames [max(0, count - FE_RING), count) are kept. */
const float *fe_logmel(const fe_t *s, int k);

/* Frame k's energy in dB; -100 (silence) if k is not kept. */
float fe_energy(const fe_t *s, int k);

/* Frame k's linear power (what the window choice sums); 0 if not kept. */
float fe_power(const fe_t *s, int k);

/* VAD so far: state 0 none, 1 speech, 2 ended (30 quiet frames after the
 * last speech frame); start = onset frame - 5 (>= 0); last = latest speech
 * frame (-1 before any); speech = speech frames since the onset. */
void fe_vad(const fe_t *s, int *state, int *start, int *last, int *speech);

/* Start frame of the best 48-frame window within [from, to] (clamped to the
 * kept frames): the one with the most energy (ties: earliest). If the range
 * is shorter than 48 frames, the best of the windows that contain all of it;
 * if fewer than 48 frames are kept at all, the oldest kept frame. */
int fe_best_window(const fe_t *s, int from, int to);

/* Write the 48 x 40 model input starting at frame `start`:
 * out[r * 40 + j] = (L[start + r][j] - mean[j]) * inv_std[j], with
 * L = ln(1e-6) (silence) for frames not kept or not yet produced.
 * mean/inv_std may be NULL for the raw log-mels. */
void fe_window(const fe_t *s, int start, const float *mean,
               const float *inv_std, float *out);

#endif
