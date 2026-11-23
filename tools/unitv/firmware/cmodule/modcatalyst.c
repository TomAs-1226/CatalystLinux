/*
 * Catalyst Coprocessor: the firmware's own module, `catalyst`.
 *
 *  - Power: delays that sleep (the CPU waits for an interrupt meanwhile)
 *    instead of MaixPy's busy loop; the CPU clock lowered and raised at run
 *    time with the time base kept continuous and every UART's divisor set
 *    again; the KPU's clock and PLL1 off while it has nothing to run; the
 *    image sensor powered down (PWDN) with its clock stopped.
 *  - Speech: the streaming front end (tools/unitv/speech/fe.c: log-mels,
 *    voice activity, the keyword spotter's input window).
 *  - Health: the time since start in microseconds, free memory, why it
 *    last started.
 *
 * Built into MaixPy by tools/unitv/firmware/build.sh (patches/ wires it in).
 */

#include <string.h>

#include "py/obj.h"
#include "py/objstr.h"
#include "py/runtime.h"
#include "py/mphal.h"

#include "clint.h"
#include "sysctl.h"
#include "uart.h"
#include "uarths.h"

#include "machine_uart.h"
#include "modmachine.h"
#include "mpconfigboard.h"
#include "sensor.h"
#include "omv_boardconfig.h"

#include "fe.h"

#ifndef CATALYST_FIRMWARE
#define CATALYST_FIRMWARE 0
#endif
#ifndef CATALYST_FIRMWARE_ID
#define CATALYST_FIRMWARE_ID "catalyst-coprocessor"
#endif

extern size_t get_free_heap_size2(void);

/* ------------------------------------------------------------ time base --- */

/* The CLINT's timer counts at the CPU clock / 50 and runs on while the CPU
 * waits for an interrupt (it is what wakes it). Microseconds come from it
 * against the clock it ran at since the last change, so a change of clock
 * neither jumps nor stretches time. (MaixPy counted CPU cycles divided by
 * the clock of the moment, which jumps when the clock changes.) */
static uint64_t tb_us;
static uint64_t tb_mtime;
static uint32_t tb_hz;

uint64_t catalyst_time_us(void)
{
    uint64_t m = clint->mtime;
    if (tb_hz == 0) {
        tb_hz = sysctl_clock_get_freq(SYSCTL_CLOCK_CPU);
        tb_mtime = m;
        tb_us = 0;
    }
    uint64_t d = m - tb_mtime;
    /* Kept well inside 64 bits: d * 50e6 stays below 2^63 for d < 2^37. */
    if (d >= (1ULL << 36)) {
        tb_us += (d * 50ULL) / (tb_hz / 1000000U);
        tb_mtime = m;
        d = 0;
    }
    return tb_us + (d * 50000000ULL) / tb_hz;
}

/* Sleeping delays (patches/0001: mp_hal_delay_ms reads it): 1, the task
 * blocks and the idle task waits for an interrupt; 0, MaixPy's busy loop. */
volatile int catalyst_sleeping = 1;

/* ---------------------------------------------------------------- power --- */

static void uart_again(mp_obj_t o)
{
    if (!MP_OBJ_IS_TYPE(o, &machine_uart_type)) {
        return;
    }
    machine_uart_obj_t *u = MP_OBJ_TO_PTR(o);
    if (u->uart_num == MICROPY_UARTHS_DEVICE) {
        uarths_config(u->baudrate, (uarths_stopbit_t)u->stop);
    } else if (u->uart_num < UART_DEVICE_MAX) {
        uart_config(u->uart_num, (size_t)u->baudrate, (size_t)u->bitwidth, u->stop, u->parity);
    }
}

/* power(sleep=None, clock=None, uarts=()) -> CPU clock in Hz.
 * clock: the CPU clock wanted (Hz); PLL0 is set again (the CPU runs from the
 * 26 MHz crystal for the moment it takes), then every UART given, and the
 * REPL's, gets its divisor for its baud rate at the new clock. */
STATIC mp_obj_t catalyst_power(size_t n_args, const mp_obj_t *pos_args, mp_map_t *kw_args)
{
    enum { ARG_sleep, ARG_clock, ARG_uarts };
    static const mp_arg_t allowed[] = {
        { MP_QSTR_sleep, MP_ARG_KW_ONLY | MP_ARG_OBJ, {.u_obj = mp_const_none} },
        { MP_QSTR_clock, MP_ARG_KW_ONLY | MP_ARG_OBJ, {.u_obj = mp_const_none} },
        { MP_QSTR_uarts, MP_ARG_KW_ONLY | MP_ARG_OBJ, {.u_obj = mp_const_none} },
    };
    mp_arg_val_t args[MP_ARRAY_SIZE(allowed)];
    mp_arg_parse_all(n_args, pos_args, kw_args, MP_ARRAY_SIZE(allowed), allowed, args);

    if (args[ARG_sleep].u_obj != mp_const_none) {
        catalyst_sleeping = mp_obj_is_true(args[ARG_sleep].u_obj) ? 1 : 0;
    }
    if (args[ARG_clock].u_obj != mp_const_none) {
        uint32_t want = (uint32_t)mp_obj_get_int(args[ARG_clock].u_obj);
        if (want < 26000000U) {
            want = 26000000U;
        }
        if (want > 600000000U) {
            want = 600000000U;
        }
        uint32_t was = sysctl_clock_get_freq(SYSCTL_CLOCK_CPU);
        if (want != was) {
            uint64_t now = catalyst_time_us();
            sysctl_cpu_set_freq(want);
            uint32_t hz = sysctl_clock_get_freq(SYSCTL_CLOCK_CPU);
            tb_us = now;
            tb_mtime = clint->mtime;
            tb_hz = hz;
            if (args[ARG_uarts].u_obj != mp_const_none) {
                size_t n;
                mp_obj_t *items;
                mp_obj_get_array(args[ARG_uarts].u_obj, &n, &items);
                for (size_t i = 0; i < n; i++) {
                    uart_again(items[i]);
                }
            }
            if (MP_STATE_PORT(Maix_stdio_uart) != NULL) {
                uart_again(MP_OBJ_FROM_PTR(MP_STATE_PORT(Maix_stdio_uart)));
            }
        }
    }
    return mp_obj_new_int_from_uint(sysctl_clock_get_freq(SYSCTL_CLOCK_CPU));
}
STATIC MP_DEFINE_CONST_FUN_OBJ_KW(catalyst_power_obj, 0, catalyst_power);

/* kpu(on) -> whether its clock runs. Off: the KPU's clock gated and PLL1
 * (which only the KPU uses) stopped; on: PLL1 locked again at the rate it
 * had, then the clock. A loaded model stays loaded (it is in main memory). */
static uint32_t kpu_pll_hz;
static bool kpu_on = true;

STATIC mp_obj_t catalyst_kpu(size_t n_args, const mp_obj_t *args)
{
    if (n_args > 0) {
        bool on = mp_obj_is_true(args[0]);
        if (on && !kpu_on) {
            sysctl_pll_enable(SYSCTL_PLL1);
            sysctl_pll_set_freq(SYSCTL_PLL1, kpu_pll_hz ? kpu_pll_hz : 400000000U);
            sysctl_clock_enable(SYSCTL_CLOCK_AI);
            kpu_on = true;
        } else if (!on && kpu_on) {
            kpu_pll_hz = sysctl_clock_get_freq(SYSCTL_CLOCK_PLL1);
            sysctl_clock_disable(SYSCTL_CLOCK_AI);
            sysctl_pll_disable(SYSCTL_PLL1);
            kpu_on = false;
        }
    }
    return mp_obj_new_bool(kpu_on);
}
STATIC MP_DEFINE_CONST_FUN_OBJ_VAR_BETWEEN(catalyst_kpu_obj, 0, 1, catalyst_kpu);

/* sensor(on): off, the sensor's PWDN line high (the OV7740 stands by, its
 * registers kept) and the DVP clock stopped, so its XCLK stops too; on, the
 * clock back and PWDN low. sensor.reset() after `on` sets it up afresh
 * either way (what main.py does). */
STATIC mp_obj_t catalyst_sensor(mp_obj_t on_in)
{
    if (mp_obj_is_true(on_in)) {
        sysctl_clock_enable(SYSCTL_CLOCK_DVP);
        DCMI_PWDN_LOW();
    } else {
        DCMI_PWDN_HIGH();
        mp_hal_delay_ms(2);
        sysctl_clock_disable(SYSCTL_CLOCK_DVP);
    }
    return mp_const_none;
}
STATIC MP_DEFINE_CONST_FUN_OBJ_1(catalyst_sensor_obj, catalyst_sensor);

/* --------------------------------------------------------------- health --- */

STATIC mp_obj_t catalyst_time_us_py(void)
{
    return mp_obj_new_int_from_ull(catalyst_time_us());
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_time_us_obj, catalyst_time_us_py);

/* Milliseconds since start, as a small int (wraps after 12 days). */
STATIC mp_obj_t catalyst_time_ms(void)
{
    return MP_OBJ_NEW_SMALL_INT((mp_int_t)((catalyst_time_us() / 1000ULL) & 0x3FFFFFFFULL));
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_time_ms_obj, catalyst_time_ms);

STATIC mp_obj_t catalyst_heap(void)
{
    return mp_obj_new_int_from_uint((mp_uint_t)get_free_heap_size2());
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_heap_obj, catalyst_heap);

/* Why it last started: "power", "reset" (the RST line), "soft", "watchdog". */
STATIC mp_obj_t catalyst_reset_cause(void)
{
    const char *why;
    switch (sysctl_get_reset_status()) {
    case SYSCTL_RESET_STATUS_HARD:
        why = "reset";
        break;
    case SYSCTL_RESET_STATUS_SOFT:
        why = "soft";
        break;
    case SYSCTL_RESET_STATUS_WDT0:
    case SYSCTL_RESET_STATUS_WDT1:
        why = "watchdog";
        break;
    default:
        why = "power";
        break;
    }
    return mp_obj_new_str(why, strlen(why));
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_reset_cause_obj, catalyst_reset_cause);

STATIC mp_obj_t catalyst_version(void)
{
    static const char id[] = CATALYST_FIRMWARE_ID;
    return mp_obj_new_str(id, sizeof(id) - 1);
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_version_obj, catalyst_version);

/* -------------------------------------------------------------- speech --- */

static fe_t fe;
static bool fe_ready;
static float fe_mean[FE_BANDS];
static float fe_inv_std[FE_BANDS];
static bool fe_normed;

static void fe_ensure(void)
{
    if (!fe_ready) {
        fe_init(&fe);
        fe_ready = true;
    }
}

STATIC mp_obj_t catalyst_fe_reset(void)
{
    fe_init(&fe);
    fe_ready = true;
    return mp_const_none;
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_fe_reset_obj, catalyst_fe_reset);

/* fe_push(pcm) -> frames added; pcm: int16 LE samples (bytes-like). */
STATIC mp_obj_t catalyst_fe_push(mp_obj_t pcm_in)
{
    mp_buffer_info_t b;
    mp_get_buffer_raise(pcm_in, &b, MP_BUFFER_READ);
    fe_ensure();
    /* Whole samples only; the buffer may not be 2-aligned. */
    const uint8_t *p = (const uint8_t *)b.buf;
    size_t n = b.len / 2;
    int16_t tmp[256];
    int added = 0;
    while (n > 0) {
        size_t m = n < 256 ? n : 256;
        for (size_t i = 0; i < m; i++) {
            tmp[i] = (int16_t)(p[2 * i] | (p[2 * i + 1] << 8));
        }
        added += fe_push(&fe, tmp, (int)m);
        p += 2 * m;
        n -= m;
    }
    return MP_OBJ_NEW_SMALL_INT(added);
}
STATIC MP_DEFINE_CONST_FUN_OBJ_1(catalyst_fe_push_obj, catalyst_fe_push);

STATIC mp_obj_t catalyst_fe_count(void)
{
    fe_ensure();
    return MP_OBJ_NEW_SMALL_INT(fe_count(&fe));
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_fe_count_obj, catalyst_fe_count);

/* fe_vad() -> (state, start, last, speech): 0 none, 1 speech, 2 ended. */
STATIC mp_obj_t catalyst_fe_vad(void)
{
    int state, start, last, speech;
    fe_ensure();
    fe_vad(&fe, &state, &start, &last, &speech);
    mp_obj_t t[4] = { MP_OBJ_NEW_SMALL_INT(state), MP_OBJ_NEW_SMALL_INT(start), MP_OBJ_NEW_SMALL_INT(last), MP_OBJ_NEW_SMALL_INT(speech) };
    return mp_obj_new_tuple(4, t);
}
STATIC MP_DEFINE_CONST_FUN_OBJ_0(catalyst_fe_vad_obj, catalyst_fe_vad);

STATIC mp_obj_t catalyst_fe_best(mp_obj_t from_in, mp_obj_t to_in)
{
    fe_ensure();
    return MP_OBJ_NEW_SMALL_INT(fe_best_window(&fe, mp_obj_get_int(from_in), mp_obj_get_int(to_in)));
}
STATIC MP_DEFINE_CONST_FUN_OBJ_2(catalyst_fe_best_obj, catalyst_fe_best);

STATIC mp_obj_t catalyst_fe_energy(mp_obj_t k_in)
{
    fe_ensure();
    return mp_obj_new_float(fe_energy(&fe, mp_obj_get_int(k_in)));
}
STATIC MP_DEFINE_CONST_FUN_OBJ_1(catalyst_fe_energy_obj, catalyst_fe_energy);

/* fe_norm(mean, inv_std): 40 float32 each (bytes-like), the model's input
 * normalisation (tools/unitv/speech/kws.json). */
STATIC mp_obj_t catalyst_fe_norm(mp_obj_t mean_in, mp_obj_t inv_in)
{
    mp_buffer_info_t m, s;
    mp_get_buffer_raise(mean_in, &m, MP_BUFFER_READ);
    mp_get_buffer_raise(inv_in, &s, MP_BUFFER_READ);
    if (m.len != sizeof(fe_mean) || s.len != sizeof(fe_inv_std)) {
        mp_raise_ValueError("fe_norm: 40 float32 each");
    }
    memcpy(fe_mean, m.buf, sizeof(fe_mean));
    memcpy(fe_inv_std, s.buf, sizeof(fe_inv_std));
    fe_normed = true;
    return mp_const_none;
}
STATIC MP_DEFINE_CONST_FUN_OBJ_2(catalyst_fe_norm_obj, catalyst_fe_norm);

/* fe_window(start, out): the 48 x 40 float32 model input into `out` (a
 * writable buffer of 7680 bytes, 8-aligned: an array('f') of 1920). */
STATIC mp_obj_t catalyst_fe_window(mp_obj_t start_in, mp_obj_t out_in)
{
    mp_buffer_info_t o;
    mp_get_buffer_raise(out_in, &o, MP_BUFFER_WRITE);
    if (o.len < FE_WINDOW * FE_BANDS * sizeof(float) || ((uintptr_t)o.buf & 3) != 0) {
        mp_raise_ValueError("fe_window: 1920 aligned float32");
    }
    fe_ensure();
    fe_window(&fe, mp_obj_get_int(start_in), fe_normed ? fe_mean : NULL, fe_normed ? fe_inv_std : NULL, (float *)o.buf);
    return mp_const_none;
}
STATIC MP_DEFINE_CONST_FUN_OBJ_2(catalyst_fe_window_obj, catalyst_fe_window);

/* -------------------------------------------------------------- module --- */

STATIC const mp_rom_map_elem_t catalyst_globals_table[] = {
    { MP_ROM_QSTR(MP_QSTR___name__), MP_ROM_QSTR(MP_QSTR_catalyst) },
    { MP_ROM_QSTR(MP_QSTR_FIRMWARE), MP_ROM_INT(CATALYST_FIRMWARE) },
    { MP_ROM_QSTR(MP_QSTR_version), MP_ROM_PTR(&catalyst_version_obj) },
    { MP_ROM_QSTR(MP_QSTR_power), MP_ROM_PTR(&catalyst_power_obj) },
    { MP_ROM_QSTR(MP_QSTR_kpu), MP_ROM_PTR(&catalyst_kpu_obj) },
    { MP_ROM_QSTR(MP_QSTR_sensor), MP_ROM_PTR(&catalyst_sensor_obj) },
    { MP_ROM_QSTR(MP_QSTR_time_us), MP_ROM_PTR(&catalyst_time_us_obj) },
    { MP_ROM_QSTR(MP_QSTR_time_ms), MP_ROM_PTR(&catalyst_time_ms_obj) },
    { MP_ROM_QSTR(MP_QSTR_heap), MP_ROM_PTR(&catalyst_heap_obj) },
    { MP_ROM_QSTR(MP_QSTR_reset_cause), MP_ROM_PTR(&catalyst_reset_cause_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_reset), MP_ROM_PTR(&catalyst_fe_reset_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_push), MP_ROM_PTR(&catalyst_fe_push_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_count), MP_ROM_PTR(&catalyst_fe_count_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_vad), MP_ROM_PTR(&catalyst_fe_vad_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_best), MP_ROM_PTR(&catalyst_fe_best_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_energy), MP_ROM_PTR(&catalyst_fe_energy_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_norm), MP_ROM_PTR(&catalyst_fe_norm_obj) },
    { MP_ROM_QSTR(MP_QSTR_fe_window), MP_ROM_PTR(&catalyst_fe_window_obj) },
};
STATIC MP_DEFINE_CONST_DICT(catalyst_globals, catalyst_globals_table);

const mp_obj_module_t mp_module_catalyst = {
    .base = { &mp_type_module },
    .globals = (mp_obj_dict_t *)&catalyst_globals,
};
