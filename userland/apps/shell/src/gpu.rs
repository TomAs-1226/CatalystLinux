//! The VideoCore's share of drawing: the glass pass, and an app's window
//! growing out of its tile, in OpenGL ES 2 through Mesa's EGL.
//!
//! The panel hangs off SPI, which no GPU can scan out to, so whatever the GPU
//! draws is read back and sent to the panel like any other frame. What it
//! takes on is the per-pixel work that costs the processor most: refracting
//! and frosting everything under the glass, and scaling a whole screen into
//! a window with its shadow and rim, every frame they move.
//!
//! Mesa is loaded when the shell starts (`dlopen`), not linked: without it,
//! or without a GPU, or if the GPU's glass does not come out as the
//! processor's does, the shell carries on drawing everything on the processor.
//! A software rasteriser is never used in place of the processor.

use bezel::{Canvas, GlassShape, Rect, Tokens};

/// Why the GPU is or is not drawing, for Settings › About.
static NOTE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn note() -> String {
    NOTE.lock().map(|n| n.clone()).unwrap_or_default()
}

pub fn set_note(text: impl Into<String>) {
    if let Ok(mut n) = NOTE.lock() {
        *n = text.into();
    }
}

#[cfg(not(target_os = "linux"))]
pub use stub::{device_present, Gpu};
#[cfg(target_os = "linux")]
pub use vc::{device_present, Gpu};

#[cfg(not(target_os = "linux"))]
mod stub {
    use super::*;

    /// No GPU off Linux: the processor draws everything.
    pub struct Gpu;

    pub fn device_present() -> bool {
        false
    }

    impl Gpu {
        pub fn open(_allow_software: bool) -> Result<Gpu, String> {
            Err("no gpu on this system".into())
        }
        pub fn renderer(&self) -> &str {
            ""
        }
        pub fn worth_it(&self) -> bool {
            false
        }
        pub fn use_everything(&mut self) {}
        pub fn release(&self) {}
        pub fn adopt(&self) -> Result<(), String> {
            Ok(())
        }
        pub fn jobs(&self) -> String {
            String::new()
        }
        pub fn glass(&mut self, _: &mut Canvas, _: &[GlassShape], _: &Tokens, _: (f32, f32), _: bool) -> Result<bool, String> {
            Ok(false)
        }
        #[allow(clippy::too_many_arguments)]
        pub fn grow(&mut self, _: &mut Canvas, _: &[u32], _: &[u32], _: u64, _: Rect, _: f32, _: bezel::Color, _: Option<&crate::shell::Morph>) -> Result<bool, String> {
            Ok(false)
        }
    }
}

#[cfg(target_os = "linux")]
mod vc {
    use super::*;
    use bezel::glass::{BEZEL_MAX, BEZEL_MIN, CLEAR_DIMMING, DISPERSION, IOR, REACH, SATURATION, SHADOW_OFFSET, THICKNESS};
    use bezel::Tone;
    use std::ffi::{c_char, c_void, CStr, CString};

    type Ptr = *mut c_void;

    // ---- loading Mesa ----

    struct Lib(Ptr);

    impl Lib {
        fn open(name: &str) -> Result<Lib, String> {
            let c = CString::new(name).expect("a library name has no NUL");
            // Kept for the life of the process: Mesa is never unloaded.
            let handle = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if handle.is_null() {
                Err(format!("{name}: {}", dl_error()))
            } else {
                Ok(Lib(handle))
            }
        }

        fn sym(&self, name: &str) -> Ptr {
            let c = CString::new(name).expect("a symbol has no NUL");
            unsafe { libc::dlsym(self.0, c.as_ptr()) }
        }
    }

    /// Where the kernel's GPU driver puts its render node once it has loaded.
    pub const RENDER_NODE: &str = "/dev/dri/renderD128";

    pub fn device_present() -> bool {
        std::path::Path::new(RENDER_NODE).exists()
    }

    fn dl_error() -> String {
        let e = unsafe { libc::dlerror() };
        if e.is_null() {
            "not found".into()
        } else {
            unsafe { CStr::from_ptr(e) }.to_string_lossy().into_owned()
        }
    }

    /// A table of entry points, each looked up by its own name.
    macro_rules! api {
        ($api:ident { $($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)* }) => {
            #[allow(non_snake_case)]
            struct $api {
                $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
            }
            impl $api {
                fn load(find: &dyn Fn(&str) -> Ptr) -> Result<Self, String> {
                    Ok($api {
                        $($name: {
                            let p = find(stringify!($name));
                            if p.is_null() {
                                return Err(format!("{} is missing", stringify!($name)));
                            }
                            unsafe { std::mem::transmute::<Ptr, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(p) }
                        },)*
                    })
                }
            }
        };
    }

    api!(Egl {
        eglGetProcAddress: fn(*const c_char) -> Ptr;
        eglGetPlatformDisplay: fn(u32, Ptr, *const isize) -> Ptr;
        eglInitialize: fn(Ptr, *mut i32, *mut i32) -> u32;
        eglBindAPI: fn(u32) -> u32;
        eglChooseConfig: fn(Ptr, *const i32, *mut Ptr, i32, *mut i32) -> u32;
        eglCreateContext: fn(Ptr, Ptr, Ptr, *const i32) -> Ptr;
        eglMakeCurrent: fn(Ptr, Ptr, Ptr, Ptr) -> u32;
        eglTerminate: fn(Ptr) -> u32;
        eglGetError: fn() -> i32;
    });

    api!(Gl {
        glGetString: fn(u32) -> *const u8;
        glGetError: fn() -> u32;
        glViewport: fn(i32, i32, i32, i32);
        glDisable: fn(u32);
        glGenTextures: fn(i32, *mut u32);
        glBindTexture: fn(u32, u32);
        glActiveTexture: fn(u32);
        glTexParameteri: fn(u32, u32, i32);
        glTexImage2D: fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
        glTexSubImage2D: fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
        glPixelStorei: fn(u32, i32);
        glGenFramebuffers: fn(i32, *mut u32);
        glBindFramebuffer: fn(u32, u32);
        glFramebufferTexture2D: fn(u32, u32, u32, u32, i32);
        glCheckFramebufferStatus: fn(u32) -> u32;
        glCreateShader: fn(u32) -> u32;
        glShaderSource: fn(u32, i32, *const *const c_char, *const i32);
        glCompileShader: fn(u32);
        glGetShaderiv: fn(u32, u32, *mut i32);
        glGetShaderInfoLog: fn(u32, i32, *mut i32, *mut c_char);
        glCreateProgram: fn() -> u32;
        glAttachShader: fn(u32, u32);
        glBindAttribLocation: fn(u32, u32, *const c_char);
        glLinkProgram: fn(u32);
        glGetProgramiv: fn(u32, u32, *mut i32);
        glGetProgramInfoLog: fn(u32, i32, *mut i32, *mut c_char);
        glUseProgram: fn(u32);
        glGetUniformLocation: fn(u32, *const c_char) -> i32;
        glUniform1i: fn(i32, i32);
        glUniform1f: fn(i32, f32);
        glUniform2f: fn(i32, f32, f32);
        glUniform3f: fn(i32, f32, f32, f32);
        glUniform4f: fn(i32, f32, f32, f32, f32);
        glVertexAttribPointer: fn(u32, i32, u32, u8, i32, *const c_void);
        glEnableVertexAttribArray: fn(u32);
        glDrawArrays: fn(u32, i32, i32);
        glReadPixels: fn(i32, i32, i32, i32, u32, u32, *mut c_void);
    });

    const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
    const EGL_OPENGL_ES_API: u32 = 0x30A0;
    const EGL_SURFACE_TYPE: i32 = 0x3033;
    const EGL_PBUFFER_BIT: i32 = 0x0001;
    const EGL_RENDERABLE_TYPE: i32 = 0x3040;
    const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
    const EGL_NONE: i32 = 0x3038;
    const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;

    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_TEXTURE0: u32 = 0x84C0;
    const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
    const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
    const GL_TEXTURE_WRAP_S: u32 = 0x2802;
    const GL_TEXTURE_WRAP_T: u32 = 0x2803;
    const GL_LINEAR: i32 = 0x2601;
    const GL_NEAREST: i32 = 0x2600;
    const GL_CLAMP_TO_EDGE: i32 = 0x812F;
    const GL_RGBA: u32 = 0x1908;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_FRAMEBUFFER: u32 = 0x8D40;
    const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
    const GL_VERTEX_SHADER: u32 = 0x8B31;
    const GL_FRAGMENT_SHADER: u32 = 0x8B30;
    const GL_COMPILE_STATUS: u32 = 0x8B81;
    const GL_LINK_STATUS: u32 = 0x8B82;
    const GL_FLOAT: u32 = 0x1406;
    const GL_TRIANGLE_STRIP: u32 = 0x0005;
    const GL_RENDERER: u32 = 0x1F01;
    const GL_PACK_ALIGNMENT: u32 = 0x0D05;
    const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
    const GL_BLEND: u32 = 0x0BE2;
    const GL_DEPTH_TEST: u32 = 0x0B71;
    const GL_SCISSOR_TEST: u32 = 0x0C11;
    const GL_DITHER: u32 = 0x0BD0;

    // ---- the shaders ----
    //
    // Pixels go up and come back in the frame's own byte order (blue, green,
    // red, unused), so nothing is shuffled on the way: shaders read `.bgr`
    // to get red, green, blue, and write `.bgr` back. Framebuffer row y is
    // the panel's row y, so a pixel's centre is `gl_FragCoord.xy` in panel
    // pixels and nothing is flipped.

    const VERTEX: &str = "
attribute vec2 a_pos;
uniform vec2 u_target;
void main() { gl_Position = vec4(a_pos / u_target * 2.0 - 1.0, 0.0, 1.0); }
";

    const PRECISION: &str = "
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
";

    /// The scene at half size: each 2 × 2 block averaged (one bilinear read
    /// at its corner), a block cut off at the patch's edge taking its own column or row.
    const DOWN: &str = "
uniform sampler2D u_scene;
uniform vec2 u_size;
uniform vec2 u_origin;
uniform vec2 u_limit;
void main() {
    vec2 p = min(u_origin + 2.0 * floor(gl_FragCoord.xy) + 1.0, u_limit);
    gl_FragColor = vec4(texture2D(u_scene, p / u_size).rgb, 0.0);
}
";

    /// One box blur of radius 2 along `u_dir`, clamped to the patch.
    const BOX: &str = "
uniform sampler2D u_src;
uniform vec2 u_tex;
uniform vec2 u_dir;
uniform vec2 u_max;
void main() {
    vec2 t = floor(gl_FragCoord.xy);
    vec3 s = vec3(0.0);
    for (int i = -2; i <= 2; i++) {
        vec2 q = clamp(t + u_dir * float(i), vec2(0.0), u_max) + 0.5;
        s += texture2D(u_src, q / u_tex).rgb;
    }
    gl_FragColor = vec4(s / 5.0, 0.0);
}
";

    /// Bezel's glass over one shape: the processor's `Glass::render`, line
    /// for line, with its bend worked out exactly rather than from a table.
    fn glass_source() -> String {
        format!(
            "
uniform sampler2D u_scene;
uniform sampler2D u_frost;
uniform vec2 u_size;
uniform vec2 u_frost_tex;
uniform vec2 u_frost_origin;
uniform vec2 u_frost_max;
uniform vec4 u_rect;
uniform float u_radius;
uniform float u_bezel;
uniform float u_strength;
uniform float u_size_scale;
uniform vec2 u_light;
uniform float u_dark;
uniform float u_calm;
uniform float u_clear;
uniform vec3 u_surface2;
uniform vec4 u_tint;

const float THICKNESS = {THICKNESS:.4};
const float REACH = {REACH:.4};
const float SHADOW_OFFSET = {SHADOW_OFFSET:.4};
const float IOR = {IOR:.4};
const float DISPERSION = {DISPERSION:.4};
const float SATURATION = {SATURATION:.4};
const float CLEAR_DIMMING = {CLEAR_DIMMING:.4};

float sd(vec2 p) {{
    float r = min(u_radius, min(u_rect.z, u_rect.w) * 0.5);
    vec2 q = abs(p - (u_rect.xy + u_rect.zw * 0.5)) - (u_rect.zw * 0.5 - r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}}

vec2 outward(vec2 p) {{
    float r = min(u_radius, min(u_rect.z, u_rect.w) * 0.5);
    vec2 d = p - (u_rect.xy + u_rect.zw * 0.5);
    float qx = abs(d.x) - (u_rect.z * 0.5 - r);
    float qy = abs(d.y) - (u_rect.w * 0.5 - r);
    float sx = d.x < 0.0 ? -1.0 : 1.0;
    float sy = d.y < 0.0 ? -1.0 : 1.0;
    float cx = max(qx, 0.0);
    float cy = max(qy, 0.0);
    if (cx > 0.0 && cy > 0.0) {{
        return vec2(sx * cx, sy * cy) / length(vec2(cx, cy));
    }}
    float lean = clamp((qx - qy) * 0.5, -1.0, 1.0) * 0.5 + 0.5;
    vec2 n = vec2(sx * lean, sy * (1.0 - lean));
    return n / max(length(n), 1e-5);
}}

float bend(float t) {{
    float m = 1.0 - t;
    float q = max(1.0 - m * m * m * m, 1e-4);
    float height = sqrt(sqrt(q));
    float slope = (m * m * m / (sqrt(q) * sqrt(sqrt(q)))) * (THICKNESS / u_bezel);
    float incident = atan(min(slope, 12.0));
    float refracted = asin(sin(incident) / IOR);
    return THICKNESS * (0.3 + 0.7 * height) * tan(incident - refracted);
}}

float luma(vec3 c) {{ return dot(c, vec3(0.2126, 0.7152, 0.0722)); }}

vec3 scene(vec2 p) {{ return texture2D(u_scene, p / u_size).bgr * 255.0; }}

vec3 frost(vec2 p) {{
    vec2 t = clamp((p - u_frost_origin) * 0.5, vec2(0.5), u_frost_max + 0.5);
    return texture2D(u_frost, t / u_frost_tex).bgr * 255.0;
}}

vec3 read(vec2 p, float frost_mix) {{
    vec3 f = frost(p);
    if (frost_mix >= 0.999) return f;
    vec3 s = scene(p);
    return s + (f - s) * frost_mix;
}}

void main() {{
    vec2 p = gl_FragCoord.xy;
    vec3 under = scene(p);
    float strength = clamp(u_strength, 0.0, 1.0);
    float d = sd(p);
    if (d > REACH) {{
        gl_FragColor = vec4(under.bgr / 255.0, 0.0);
        return;
    }}
    // Outside: the adaptive shadow, darker over busy content.
    float cover = clamp(0.5 - d / 1.5, 0.0, 1.0);
    vec3 outside = under;
    if (cover < 1.0) {{
        float ds = sd(p - vec2(0.0, SHADOW_OFFSET));
        float fall = clamp(1.0 - (ds + 2.0) / (REACH - 1.0), 0.0, 1.0);
        if (fall > 0.0) {{
            float busy = min(abs(luma(under) - luma(frost(p))) / 48.0, 1.0);
            outside = under * (1.0 - (0.08 + 0.12 * busy) * u_size_scale * fall * fall * strength);
        }}
    }}
    vec3 o = outside;
    if (cover > 0.0) {{
        // Inside: where on the bezel, which way it faces, how far it bends light.
        float depth = max(-d, 0.0);
        float t = min(depth / u_bezel, 1.0);
        vec2 n = vec2(0.0);
        float offset = 0.0;
        if (t < 1.0) {{
            n = outward(p);
            offset = bend(t) * strength;
        }}
        vec3 c;
        if (u_calm > 0.5) {{
            c = u_surface2 * 255.0;
        }} else {{
            float frost_mix = strength * (u_clear > 0.5 ? 0.12 : 1.0);
            c = read(p - n * offset, frost_mix);
            if (offset > 1.5) {{
                c.r = read(p - n * offset * (1.0 + DISPERSION), frost_mix).r;
                c.b = read(p - n * offset * (1.0 - DISPERSION), frost_mix).b;
            }}
            float grey = u_dark > 0.5 ? 0.17 * 255.0 : 0.93 * 255.0;
            float amount = (u_dark > 0.5 ? 0.22 : 0.38) * strength * (u_clear > 0.5 ? 0.0 : 1.0);
            float l = luma(c);
            c = c + (grey - c) * amount;
            c = l + (c - l) * (1.0 + (SATURATION - 1.0) * strength);
            if (u_clear > 0.5) c *= 1.0 - CLEAR_DIMMING * strength;
        }}
        if (u_tint.a > 0.5) {{
            float k = 0.82 + 0.35 * (luma(c) / 255.0);
            c += (u_tint.rgb * 255.0 * k - c) * (0.82 * strength);
        }}
        // One light: the edge line, its fainter return, a soft band inside.
        if (t < 1.0) {{
            float facing = dot(n, u_light);
            float line = clamp(1.0 - depth / 1.3, 0.0, 1.0);
            float band = (1.0 - t) * (1.0 - t) * 0.05;
            float lit = ((0.16 + 0.62 * max(facing, 0.0) + 0.26 * max(-facing, 0.0)) * line + band) * strength;
            c += (255.0 - c) * lit;
            if (u_dark < 0.5) c *= 1.0 - 0.10 * max(-facing, 0.0) * line * strength;
        }}
        o = outside + (c - outside) * cover;
    }}
    // Whole levels, cut down as the processor's are.
    gl_FragColor = vec4(floor(clamp(o, 0.0, 255.0)).bgr / 255.0, 0.0);
}}
"
        )
    }

    /// A screen scaled into a window over the one beneath: its soft shadow,
    /// bilinear sampling, a one-pixel rim, and smooth rounded corners.
    const GROW: &str = "
uniform sampler2D u_under;
uniform sampler2D u_over;
uniform vec2 u_size;
uniform vec4 u_window;
uniform float u_radius;
uniform vec3 u_edge;
// The tile the window grows from, in the picture beneath, and how much of
// the window the screen has taken over (1: all screen).
uniform vec4 u_tile;
uniform float u_screen;

float sd(vec2 p, vec4 r, float radius) {
    float k = min(radius, min(r.z, r.w) * 0.5);
    vec2 q = abs(p - (r.xy + r.zw * 0.5)) - (r.zw * 0.5 - k);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - k;
}

void main() {
    vec2 p = gl_FragCoord.xy;
    vec3 base = texture2D(u_under, p / u_size).bgr;
    // The shadow, a little lower than the window, as light from above casts it.
    float ds = sd(p, u_window + vec4(0.0, 2.0, 0.0, 0.0), u_radius);
    if (ds > 0.0 && ds < 8.0) {
        float fall = 1.0 - ds / 8.0;
        base *= 1.0 - 0.22 * fall * fall;
    }
    float d = sd(p, u_window, u_radius);
    vec3 o = base;
    if (d <= 0.5) {
        vec2 uv = (p - u_window.xy) / u_window.zw;
        vec3 c = texture2D(u_over, uv).bgr;
        if (u_screen < 1.0) {
            vec3 tile = texture2D(u_under, (u_tile.xy + uv * u_tile.zw) / u_size).bgr;
            c = mix(tile, c, u_screen);
        }
        if (d > -1.0 && u_radius > 0.5) c = mix(c, u_edge, 90.0 / 256.0 * clamp(d + 1.0, 0.0, 1.0));
        o = mix(base, c, clamp(0.5 - d, 0.0, 1.0));
    }
    gl_FragColor = vec4(o.bgr, 0.0);
}
";

    // ---- the renderer ----

    const W: i32 = crate::gfx::W as i32;
    const H: i32 = crate::gfx::H as i32;
    /// The frost is kept at half size, as the processor keeps it.
    const FW: i32 = (W + 1) / 2;
    const FH: i32 = (H + 1) / 2;

    /// A texture and the framebuffer that draws into it.
    #[derive(Clone, Copy)]
    struct Target {
        texture: u32,
        fbo: u32,
    }

    /// One group of glass as last drawn, as the processor's pass keeps it:
    /// unchanged glass over an unchanged patch is copied back, not redrawn.
    struct Kept {
        group: u8,
        key: Vec<u32>,
        region: (i32, i32, i32, i32),
        before: Vec<u32>,
        after: Vec<u32>,
    }

    pub struct Gpu {
        egl: Egl,
        gl: Gl,
        display: Ptr,
        context: Ptr,
        renderer: String,
        down: u32,
        boxblur: u32,
        glass: u32,
        grow: u32,
        /// The scene, the frost (two, to blur back and forth), and what is drawn.
        scene: u32,
        frost: [Target; 2],
        out: Target,
        /// A window growing: the screen beneath and the one arriving, and
        /// which pair of pictures they were taken from.
        under: u32,
        over: u32,
        pictures: Option<u64>,
        kept: Vec<Kept>,
        readback: Vec<u32>,
        /// Which jobs the GPU does faster than the processor here, as timed
        /// on the test frame when it opened (milliseconds: GPU, processor).
        use_glass: bool,
        use_grow: bool,
        glass_ms: (f32, f32),
        grow_ms: (f32, f32),
    }

    impl Gpu {
        /// Load Mesa, make a GLES 2 context on the GPU (no window: the panel
        /// is not the GPU's), and check that its glass is the processor's.
        /// A software renderer is refused unless `allow_software` (for tests
        /// off the device).
        pub fn open(allow_software: bool) -> Result<Gpu, String> {
            // No GPU device, no GPU: Mesa is not even loaded.
            if !allow_software && !device_present() {
                return Err("no gpu device".into());
            }
            let egl_lib = Lib::open("libEGL.so.1")?;
            let gles_lib = Lib::open("libGLESv2.so.2")?;
            let egl = Egl::load(&|name| egl_lib.sym(name))?;
            let gl = Gl::load(&|name| {
                let p = gles_lib.sym(name);
                if p.is_null() {
                    let c = CString::new(name).expect("no NUL");
                    unsafe { (egl.eglGetProcAddress)(c.as_ptr()) }
                } else {
                    p
                }
            })?;
            let egl_err = |what: &str| format!("{what} failed (egl {:#x})", unsafe { (egl.eglGetError)() });
            let (display, context) = unsafe {
                // Surfaceless: Mesa finds the GPU's render node by itself.
                let display = (egl.eglGetPlatformDisplay)(EGL_PLATFORM_SURFACELESS_MESA, std::ptr::null_mut(), std::ptr::null());
                if display.is_null() {
                    return Err(egl_err("eglGetPlatformDisplay"));
                }
                let (mut major, mut minor) = (0, 0);
                if (egl.eglInitialize)(display, &mut major, &mut minor) == 0 {
                    return Err(egl_err("eglInitialize"));
                }
                let fail = |what: &str| {
                    let why = egl_err(what);
                    (egl.eglTerminate)(display);
                    why
                };
                if (egl.eglBindAPI)(EGL_OPENGL_ES_API) == 0 {
                    return Err(fail("eglBindAPI"));
                }
                let attribs = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE];
                let mut config: Ptr = std::ptr::null_mut();
                let mut count = 0;
                if (egl.eglChooseConfig)(display, attribs.as_ptr(), &mut config, 1, &mut count) == 0 || count < 1 {
                    // Mesa's surfaceless platform also takes a context with no config.
                    config = std::ptr::null_mut();
                }
                let context_attribs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
                let context = (egl.eglCreateContext)(display, config, std::ptr::null_mut(), context_attribs.as_ptr());
                if context.is_null() {
                    return Err(fail("eglCreateContext"));
                }
                if (egl.eglMakeCurrent)(display, std::ptr::null_mut(), std::ptr::null_mut(), context) == 0 {
                    return Err(fail("eglMakeCurrent"));
                }
                (display, context)
            };
            let renderer = unsafe {
                let s = (gl.glGetString)(GL_RENDERER);
                if s.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(s as *const c_char).to_string_lossy().into_owned()
                }
            };
            // From here, whatever goes wrong, dropping the GPU lets Mesa go of what it made.
            let mut gpu = Gpu {
                egl,
                gl,
                display,
                context,
                renderer,
                down: 0,
                boxblur: 0,
                glass: 0,
                grow: 0,
                scene: 0,
                frost: [Target { texture: 0, fbo: 0 }; 2],
                out: Target { texture: 0, fbo: 0 },
                under: 0,
                over: 0,
                pictures: None,
                kept: Vec::new(),
                readback: Vec::new(),
                use_glass: false,
                use_grow: false,
                glass_ms: (0.0, 0.0),
                grow_ms: (0.0, 0.0),
            };
            let software = ["softpipe", "llvmpipe", "swrast", "software"].iter().any(|s| gpu.renderer.to_lowercase().contains(s));
            if software && !allow_software {
                return Err(format!("only a software renderer ({})", gpu.renderer));
            }
            gpu.build()?;
            gpu.self_test()?;
            Ok(gpu)
        }

        /// Let go of the GPU on this thread, so another can take it up.
        pub fn release(&self) {
            unsafe { (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        }

        /// Take the GPU up on this thread: the shell opens it on its own
        /// thread, so the start animation never waits for Mesa, and draws
        /// with it on the main one.
        pub fn adopt(&self) -> Result<(), String> {
            match unsafe { (self.egl.eglMakeCurrent)(self.display, std::ptr::null_mut(), std::ptr::null_mut(), self.context) } {
                0 => Err(format!("eglMakeCurrent failed (egl {:#x})", unsafe { (self.egl.eglGetError)() })),
                _ => Ok(()),
            }
        }

        /// What the GPU calls itself ("VC4 V3D 2.1").
        pub fn renderer(&self) -> &str {
            &self.renderer
        }

        /// Whether it does any job faster than the processor.
        pub fn worth_it(&self) -> bool {
            self.use_glass || self.use_grow
        }

        /// Every job, however long it takes (for testing the path).
        pub fn use_everything(&mut self) {
            (self.use_glass, self.use_grow) = (true, true);
        }

        /// Which jobs it does, and the timings that decided it.
        pub fn jobs(&self) -> String {
            let job = |name: &str, used: bool, (gpu, cpu): (f32, f32)| format!("{name} {} ({gpu:.2} ms gpu, {cpu:.2} ms processor)", if used { "on the gpu" } else { "on the processor" });
            format!("{} · {}", job("glass", self.use_glass, self.glass_ms), job("windows", self.use_grow, self.grow_ms))
        }

        fn build(&mut self) -> Result<(), String> {
            self.down = self.program(DOWN)?;
            self.boxblur = self.program(BOX)?;
            self.glass = self.program(&glass_source())?;
            self.grow = self.program(GROW)?;
            self.scene = self.texture(W, H, true);
            self.frost = [self.target(FW, FH, true)?, self.target(FW, FH, true)?];
            self.out = self.target(W, H, false)?;
            self.under = self.texture(W, H, true);
            self.over = self.texture(W, H, true);
            unsafe {
                let gl = &self.gl;
                for cap in [GL_BLEND, GL_DEPTH_TEST, GL_SCISSOR_TEST, GL_DITHER] {
                    (gl.glDisable)(cap);
                }
                (gl.glPixelStorei)(GL_UNPACK_ALIGNMENT, 4);
                (gl.glPixelStorei)(GL_PACK_ALIGNMENT, 4);
            }
            self.check("setting up")
        }

        fn check(&self, what: &str) -> Result<(), String> {
            match unsafe { (self.gl.glGetError)() } {
                0 => Ok(()),
                e => Err(format!("gl error {e:#x} {what}")),
            }
        }

        fn shader(&self, kind: u32, source: &str) -> Result<u32, String> {
            let gl = &self.gl;
            let source = CString::new(source).expect("shader source has no NUL");
            unsafe {
                let s = (gl.glCreateShader)(kind);
                let ptr = source.as_ptr();
                (gl.glShaderSource)(s, 1, &ptr, std::ptr::null());
                (gl.glCompileShader)(s);
                let mut ok = 0;
                (gl.glGetShaderiv)(s, GL_COMPILE_STATUS, &mut ok);
                if ok == 0 {
                    let mut log = vec![0u8; 2048];
                    let mut len = 0;
                    (gl.glGetShaderInfoLog)(s, log.len() as i32, &mut len, log.as_mut_ptr() as *mut c_char);
                    return Err(format!("shader: {}", String::from_utf8_lossy(&log[..len.max(0) as usize])));
                }
                Ok(s)
            }
        }

        fn program(&self, fragment: &str) -> Result<u32, String> {
            let gl = &self.gl;
            let vs = self.shader(GL_VERTEX_SHADER, VERTEX)?;
            let fs = self.shader(GL_FRAGMENT_SHADER, &format!("{PRECISION}{fragment}"))?;
            unsafe {
                let p = (gl.glCreateProgram)();
                (gl.glAttachShader)(p, vs);
                (gl.glAttachShader)(p, fs);
                (gl.glBindAttribLocation)(p, 0, c"a_pos".as_ptr());
                (gl.glLinkProgram)(p);
                let mut ok = 0;
                (gl.glGetProgramiv)(p, GL_LINK_STATUS, &mut ok);
                if ok == 0 {
                    let mut log = vec![0u8; 2048];
                    let mut len = 0;
                    (gl.glGetProgramInfoLog)(p, log.len() as i32, &mut len, log.as_mut_ptr() as *mut c_char);
                    return Err(format!("program: {}", String::from_utf8_lossy(&log[..len.max(0) as usize])));
                }
                Ok(p)
            }
        }

        fn texture(&self, w: i32, h: i32, linear: bool) -> u32 {
            let gl = &self.gl;
            let filter = if linear { GL_LINEAR } else { GL_NEAREST };
            let mut t = 0;
            unsafe {
                (gl.glGenTextures)(1, &mut t);
                (gl.glBindTexture)(GL_TEXTURE_2D, t);
                (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filter);
                (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filter);
                (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
                (gl.glTexParameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
                (gl.glTexImage2D)(GL_TEXTURE_2D, 0, GL_RGBA as i32, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, std::ptr::null());
            }
            t
        }

        fn target(&self, w: i32, h: i32, linear: bool) -> Result<Target, String> {
            let gl = &self.gl;
            let texture = self.texture(w, h, linear);
            let mut fbo = 0;
            unsafe {
                (gl.glGenFramebuffers)(1, &mut fbo);
                (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
                (gl.glFramebufferTexture2D)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, texture, 0);
                let status = (gl.glCheckFramebufferStatus)(GL_FRAMEBUFFER);
                if status != GL_FRAMEBUFFER_COMPLETE {
                    return Err(format!("framebuffer incomplete ({status:#x})"));
                }
            }
            Ok(Target { texture, fbo })
        }

        /// Draw into `fbo` (`w` × `h`) with `program`.
        fn begin(&self, program: u32, fbo: u32, w: i32, h: i32) {
            let gl = &self.gl;
            unsafe {
                (gl.glBindFramebuffer)(GL_FRAMEBUFFER, fbo);
                (gl.glViewport)(0, 0, w, h);
                (gl.glUseProgram)(program);
            }
            self.uniform2(program, "u_target", w as f32, h as f32);
        }

        fn location(&self, program: u32, name: &str) -> i32 {
            let c = CString::new(name).expect("no NUL");
            unsafe { (self.gl.glGetUniformLocation)(program, c.as_ptr()) }
        }
        fn uniform1(&self, program: u32, name: &str, v: f32) {
            unsafe { (self.gl.glUniform1f)(self.location(program, name), v) }
        }
        fn uniform2(&self, program: u32, name: &str, a: f32, b: f32) {
            unsafe { (self.gl.glUniform2f)(self.location(program, name), a, b) }
        }
        fn uniform3(&self, program: u32, name: &str, c: bezel::Color) {
            unsafe { (self.gl.glUniform3f)(self.location(program, name), c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0) }
        }
        fn uniform4(&self, program: u32, name: &str, v: [f32; 4]) {
            unsafe { (self.gl.glUniform4f)(self.location(program, name), v[0], v[1], v[2], v[3]) }
        }
        /// Texture `texture` on unit `unit`, as sampler `name`.
        fn sampler(&self, program: u32, name: &str, unit: u32, texture: u32) {
            unsafe {
                (self.gl.glActiveTexture)(GL_TEXTURE0 + unit);
                (self.gl.glBindTexture)(GL_TEXTURE_2D, texture);
                (self.gl.glUniform1i)(self.location(program, name), unit as i32);
            }
        }

        /// Fill a rectangle of the target, in its pixels.
        fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
            let v: [f32; 8] = [x0, y0, x1, y0, x0, y1, x1, y1];
            unsafe {
                (self.gl.glVertexAttribPointer)(0, 2, GL_FLOAT, 0, 0, v.as_ptr() as *const c_void);
                (self.gl.glEnableVertexAttribArray)(0);
                (self.gl.glDrawArrays)(GL_TRIANGLE_STRIP, 0, 4);
            }
        }

        /// Rows `y0..y1` of a whole-panel picture up into `texture`.
        fn upload(&self, texture: u32, px: &[u32], y0: i32, y1: i32) {
            if y1 <= y0 {
                return;
            }
            unsafe {
                (self.gl.glBindTexture)(GL_TEXTURE_2D, texture);
                let rows = &px[(y0 * W) as usize..(y1 * W) as usize];
                (self.gl.glTexSubImage2D)(GL_TEXTURE_2D, 0, 0, y0, W, y1 - y0, GL_RGBA, GL_UNSIGNED_BYTE, rows.as_ptr() as *const c_void);
            }
        }

        /// A rectangle of what was drawn into `out`, back into the frame.
        fn read_back(&mut self, canvas: &mut Canvas, (x0, y0, x1, y1): (i32, i32, i32, i32)) {
            let (w, h) = (x1 - x0, y1 - y0);
            if w <= 0 || h <= 0 {
                return;
            }
            unsafe { (self.gl.glBindFramebuffer)(GL_FRAMEBUFFER, self.out.fbo) };
            if (x0, y0, x1, y1) == (0, 0, W, H) {
                unsafe { (self.gl.glReadPixels)(0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE, canvas.px.as_mut_ptr() as *mut c_void) };
                return;
            }
            self.readback.resize((w * h) as usize, 0);
            unsafe { (self.gl.glReadPixels)(x0, y0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, self.readback.as_mut_ptr() as *mut c_void) };
            for row in 0..h {
                let at = ((y0 + row) * W + x0) as usize;
                canvas.px[at..at + w as usize].copy_from_slice(&self.readback[(row * w) as usize..((row + 1) * w) as usize]);
            }
        }

        /// The glass pass on the GPU. `Ok(false)`: these shapes are the
        /// processor's to draw (glass melting together within a group).
        pub fn glass(&mut self, canvas: &mut Canvas, shapes: &[GlassShape], tokens: &Tokens, light: (f32, f32), calm: bool) -> Result<bool, String> {
            if !self.use_glass {
                return Ok(false);
            }
            let live: Vec<&GlassShape> = shapes.iter().filter(|s| s.strength > 0.004 && s.rect.w > 0.5 && s.rect.h > 0.5).collect();
            if live.is_empty() {
                return Ok(true);
            }
            let mut groups: Vec<u8> = live.iter().map(|s| s.group).collect();
            groups.sort_unstable();
            if groups.windows(2).any(|g| g[0] == g[1]) {
                return Ok(false);
            }
            let (w, h) = (canvas.w, canvas.h);
            let dark = tokens.tone == Tone::Dark;
            let len = (light.0 * light.0 + light.1 * light.1).sqrt().max(1e-6);
            let light = (light.0 / len, light.1 / len);
            let pad = REACH + THICKNESS;
            let region_of = |s: &GlassShape| {
                let (x0, y0) = (((s.rect.x - pad).floor() as i32).clamp(0, w), ((s.rect.y - pad).floor() as i32).clamp(0, h));
                (x0, y0, ((s.rect.right() + pad).ceil() as i32).clamp(x0, w), ((s.rect.bottom() + pad).ceil() as i32).clamp(y0, h))
            };
            let gather = |px: &[u32], (x0, y0, x1, y1): (i32, i32, i32, i32)| -> Vec<u32> {
                let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0)).max(0) as usize);
                for y in y0..y1 {
                    out.extend_from_slice(&px[(y * w + x0) as usize..(y * w + x1) as usize]);
                }
                out
            };
            // What each shape's patch looks like before any glass is drawn:
            // glass refracts the scene, never other glass.
            let mut jobs = Vec::new();
            let mut kept_before = std::mem::take(&mut self.kept);
            for s in &live {
                let region = region_of(s);
                let mut key: Vec<u32> = vec![dark as u32, calm as u32, light.0.to_bits(), light.1.to_bits(), tokens.surface2.to_u32()];
                key.extend([s.rect.x, s.rect.y, s.rect.w, s.rect.h, s.radius, s.strength].map(f32::to_bits));
                key.extend([s.tint.map_or(u32::MAX, |c| c.to_u32()), s.clear as u32]);
                let before = gather(&canvas.px, region);
                let last = kept_before.iter().position(|k| k.group == s.group).map(|at| kept_before.swap_remove(at));
                jobs.push((*s, region, key, before, last));
            }
            // The scene goes up once, the rows any changed patch covers.
            let changed: Vec<(i32, i32)> = jobs
                .iter()
                .filter(|(_, region, key, before, last)| !last.as_ref().is_some_and(|k| k.key == *key && k.region == *region && k.before == *before))
                .map(|(_, r, ..)| (r.1, r.3))
                .collect();
            if let (Some(y0), Some(y1)) = (changed.iter().map(|r| r.0).min(), changed.iter().map(|r| r.1).max()) {
                self.upload(self.scene, &canvas.px, y0, y1);
            }
            for (s, region, key, before, last) in jobs {
                if let Some(last) = last.filter(|k| k.key == key && k.region == region && k.before == before) {
                    // Nothing under this glass has changed: neither has the glass.
                    let (x0, y0, x1, y1) = region;
                    let row_w = (x1 - x0) as usize;
                    for (row, y) in (y0..y1).enumerate() {
                        canvas.px[(y * w + x0) as usize..(y * w + x1) as usize].copy_from_slice(&last.after[row * row_w..(row + 1) * row_w]);
                    }
                    self.kept.push(last);
                    continue;
                }
                if region.0 >= region.2 || region.1 >= region.3 {
                    continue;
                }
                self.frost_patch(region);
                self.glass_shape(s, region, tokens, light, dark, calm);
                let (gx0, gy0) = (((s.rect.x - REACH).floor() as i32).max(0), ((s.rect.y - REACH).floor() as i32).max(0));
                let (gx1, gy1) = (((s.rect.right() + REACH).ceil() as i32).min(w), ((s.rect.bottom() + REACH + SHADOW_OFFSET).ceil() as i32).min(h));
                self.read_back(canvas, (gx0, gy0, gx1, gy1));
                self.check("drawing glass")?;
                let after = gather(&canvas.px, region);
                self.kept.push(Kept { group: s.group, key, region, before, after });
            }
            Ok(true)
        }

        /// Frost one patch of the scene at half size into `frost[0]`: the 2 × 2
        /// average, then three box blurs of radius 2 each way.
        fn frost_patch(&self, (x0, y0, x1, y1): (i32, i32, i32, i32)) {
            let (fw, fh) = ((x1 - x0 + 1) / 2, (y1 - y0 + 1) / 2);
            self.begin(self.down, self.frost[0].fbo, FW, FH);
            self.sampler(self.down, "u_scene", 0, self.scene);
            self.uniform2(self.down, "u_size", W as f32, H as f32);
            self.uniform2(self.down, "u_origin", x0 as f32, y0 as f32);
            self.uniform2(self.down, "u_limit", x1 as f32 - 0.5, y1 as f32 - 0.5);
            self.rect(0.0, 0.0, fw as f32, fh as f32);
            for _ in 0..3 {
                for (from, to, dir) in [(0, 1, (1.0, 0.0)), (1, 0, (0.0, 1.0))] {
                    self.begin(self.boxblur, self.frost[to].fbo, FW, FH);
                    self.sampler(self.boxblur, "u_src", 0, self.frost[from].texture);
                    self.uniform2(self.boxblur, "u_tex", FW as f32, FH as f32);
                    self.uniform2(self.boxblur, "u_dir", dir.0, dir.1);
                    self.uniform2(self.boxblur, "u_max", (fw - 1) as f32, (fh - 1) as f32);
                    self.rect(0.0, 0.0, fw as f32, fh as f32);
                }
            }
        }

        fn glass_shape(&self, s: &GlassShape, (x0, y0, x1, y1): (i32, i32, i32, i32), tokens: &Tokens, light: (f32, f32), dark: bool, calm: bool) {
            let p = self.glass;
            let (fw, fh) = ((x1 - x0 + 1) / 2, (y1 - y0 + 1) / 2);
            let bezel = (0.9 * s.radius).clamp(BEZEL_MIN, BEZEL_MAX).min(s.rect.w.min(s.rect.h) * 0.5);
            self.begin(p, self.out.fbo, W, H);
            self.sampler(p, "u_scene", 0, self.scene);
            self.sampler(p, "u_frost", 1, self.frost[0].texture);
            self.uniform2(p, "u_size", W as f32, H as f32);
            self.uniform2(p, "u_frost_tex", FW as f32, FH as f32);
            self.uniform2(p, "u_frost_origin", x0 as f32, y0 as f32);
            self.uniform2(p, "u_frost_max", (fw - 1) as f32, (fh - 1) as f32);
            self.uniform4(p, "u_rect", [s.rect.x, s.rect.y, s.rect.w, s.rect.h]);
            self.uniform1(p, "u_radius", s.radius);
            self.uniform1(p, "u_bezel", bezel);
            self.uniform1(p, "u_strength", s.strength);
            self.uniform1(p, "u_size_scale", 0.6 + 0.4 * (s.rect.w * s.rect.h / 5000.0).min(1.0));
            self.uniform2(p, "u_light", light.0, light.1);
            self.uniform1(p, "u_dark", if dark { 1.0 } else { 0.0 });
            self.uniform1(p, "u_calm", if calm { 1.0 } else { 0.0 });
            self.uniform1(p, "u_clear", if s.clear { 1.0 } else { 0.0 });
            self.uniform3(p, "u_surface2", tokens.surface2);
            let tint = s.tint.unwrap_or_default();
            self.uniform4(p, "u_tint", [tint.0 as f32 / 255.0, tint.1 as f32 / 255.0, tint.2 as f32 / 255.0, if s.tint.is_some() { 1.0 } else { 0.0 }]);
            self.rect(
                (s.rect.x - REACH).floor().max(0.0),
                (s.rect.y - REACH).floor().max(0.0),
                (s.rect.right() + REACH).ceil().min(W as f32),
                (s.rect.bottom() + REACH + SHADOW_OFFSET).ceil().min(H as f32),
            );
        }

        /// A screen growing out of its tile (or going back into it): `over`
        /// drawn into `window` over `under`, the whole frame back into `canvas`.
        /// `pictures` names the pair, so they go up only when they change.
        #[allow(clippy::too_many_arguments)]
        pub fn grow(&mut self, canvas: &mut Canvas, under: &[u32], over: &[u32], pictures: u64, window: Rect, radius: f32, edge: bezel::Color, morph: Option<&crate::shell::Morph>) -> Result<bool, String> {
            if !self.use_grow || under.len() != (W * H) as usize || over.len() != under.len() || canvas.px.len() != under.len() {
                return Ok(false);
            }
            if self.pictures != Some(pictures) {
                self.upload(self.under, under, 0, H);
                self.upload(self.over, over, 0, H);
                self.pictures = Some(pictures);
            }
            let p = self.grow;
            self.begin(p, self.out.fbo, W, H);
            self.sampler(p, "u_under", 0, self.under);
            self.sampler(p, "u_over", 1, self.over);
            self.uniform2(p, "u_size", W as f32, H as f32);
            self.uniform4(p, "u_window", [window.x, window.y, window.w, window.h]);
            self.uniform1(p, "u_radius", radius);
            self.uniform3(p, "u_edge", edge);
            // The tile, as the processor reads it (`blit_scaled`): only one really there.
            let morph = morph.filter(|m| m.screen < 1.0 && m.tile.w >= 1.0 && m.tile.h >= 1.0);
            let tile = morph.map_or([0.0, 0.0, 1.0, 1.0], |m| [m.tile.x, m.tile.y, m.tile.w, m.tile.h]);
            self.uniform4(p, "u_tile", tile);
            self.uniform1(p, "u_screen", morph.map_or(1.0, |m| m.screen.clamp(0.0, 1.0)));
            self.rect(0.0, 0.0, W as f32, H as f32);
            self.read_back(canvas, (0, 0, W, H));
            self.check("growing a window")?;
            Ok(true)
        }

        /// Draw the same glass and the same growing window on both, and
        /// compare: the GPU is used only if it draws them as the processor does.
        fn self_test(&mut self) -> Result<(), String> {
            self.use_everything();
            let (scene, shapes) = test_scene();
            let tokens = Tokens::dark();
            let mut cpu = scene.clone();
            bezel::Glass::new().render(&mut cpu, &shapes, &tokens, bezel::LIGHT_REST, false);
            let mut gpu = scene.clone();
            if !self.glass(&mut gpu, &shapes, &tokens, bezel::LIGHT_REST, false)? {
                return Err("the test glass was not drawn".into());
            }
            self.kept.clear();
            let glass = Difference::of(&cpu.px, &gpu.px);
            if !glass.close() {
                return Err(format!("its glass is not the processor's ({glass})"));
            }
            let window = Rect::new(40.5, 30.25, 150.0, 80.0);
            let mut over = scene.clone();
            over.px.iter_mut().for_each(|p| *p ^= 0x00ff_ffff);
            // Part way between the tile it grew from and the screen.
            let morph = crate::shell::Morph { under: &scene.px, tile: Rect::new(20.25, 18.5, 64.0, 44.0), screen: 0.4 };
            let mut cpu = scene.clone();
            crate::shell::blit_scaled(&mut cpu, &over.px, window, 9.0, tokens.ground, Some(&morph));
            let mut gpu = scene.clone();
            self.grow(&mut gpu, &scene.px, &over.px, u64::MAX, window, 9.0, tokens.ground, Some(&morph))?;
            self.pictures = None;
            let grow = Difference::of(&cpu.px, &gpu.px);
            if !grow.close() {
                return Err(format!("its windows are not the processor's ({grow})"));
            }
            // Then which is quicker at each, best of three, the shaders
            // already compiled by the runs above. Reading a frame back from
            // the GPU has a fixed cost, so small glass can be quicker on the
            // processor than on the GPU.
            let best = |f: &mut dyn FnMut()| -> f32 {
                (0..3)
                    .map(|_| {
                        let t = std::time::Instant::now();
                        f();
                        t.elapsed().as_secs_f32() * 1e3
                    })
                    .fold(f32::MAX, f32::min)
            };
            let mut frame = scene.clone();
            let cpu_glass = best(&mut || {
                frame.px.copy_from_slice(&scene.px);
                bezel::Glass::new().render(&mut frame, &shapes, &tokens, bezel::LIGHT_REST, false);
            });
            let mut failed = None;
            let gpu_glass = best(&mut || {
                frame.px.copy_from_slice(&scene.px);
                self.kept.clear();
                if let Err(why) = self.glass(&mut frame, &shapes, &tokens, bezel::LIGHT_REST, false) {
                    failed = Some(why);
                }
            });
            let cpu_grow = best(&mut || {
                frame.px.copy_from_slice(&scene.px);
                crate::shell::blit_scaled(&mut frame, &over.px, window, 9.0, tokens.ground, Some(&morph));
            });
            self.grow(&mut frame, &scene.px, &over.px, u64::MAX, window, 9.0, tokens.ground, Some(&morph))?;
            let gpu_grow = best(&mut || {
                if let Err(why) = self.grow(&mut frame, &scene.px, &over.px, u64::MAX, window, 9.0, tokens.ground, Some(&morph)) {
                    failed = Some(why);
                }
            });
            if let Some(why) = failed {
                return Err(why);
            }
            self.kept.clear();
            self.pictures = None;
            (self.glass_ms, self.grow_ms) = ((gpu_glass, cpu_glass), (gpu_grow, cpu_grow));
            (self.use_glass, self.use_grow) = (gpu_glass < cpu_glass, gpu_grow < cpu_grow);
            set_note(format!("{} · glass {glass} · windows {grow}", self.renderer));
            Ok(())
        }
    }

    impl Drop for Gpu {
        fn drop(&mut self) {
            self.release();
            unsafe { (self.egl.eglTerminate)(self.display) };
        }
    }

    // The context is current on one thread at a time: it is released on the
    // thread that opened it before the GPU is handed to the one that draws.
    unsafe impl Send for Gpu {}

    /// How far apart two frames are, channel by channel.
    pub struct Difference {
        /// The mean difference over every channel of every pixel that differs at all.
        pub mean: f32,
        /// How many channels are off by more than 8 levels, per thousand.
        pub far: f32,
        pub max: u32,
    }

    impl Difference {
        pub fn of(a: &[u32], b: &[u32]) -> Difference {
            let (mut sum, mut n, mut far, mut max) = (0u64, 0u64, 0u64, 0u32);
            for (&x, &y) in a.iter().zip(b) {
                if x == y {
                    continue;
                }
                for shift in [0, 8, 16] {
                    let d = ((x >> shift) & 0xff).abs_diff((y >> shift) & 0xff);
                    sum += d as u64;
                    n += 1;
                    far += (d > 8) as u64;
                    max = max.max(d);
                }
            }
            let channels = (a.len() * 3).max(1) as f32;
            Difference { mean: if n == 0 { 0.0 } else { sum as f32 / n as f32 }, far: far as f32 * 1000.0 / channels, max }
        }

        /// Near enough that nobody could tell on the panel: a level or two of
        /// rounding apart, with a refraction that may land a pixel off right
        /// at an edge of the text beneath, but no more.
        pub fn close(&self) -> bool {
            self.mean < 3.0 && self.far < 2.0
        }
    }

    impl std::fmt::Display for Difference {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "mean {:.2}, far {:.2}‰, max {}", self.mean, self.far, self.max)
        }
    }

    /// A scene with something under the glass worth refracting, and the
    /// shell's own glass over it: the island, and the pager.
    fn test_scene() -> (Canvas, Vec<GlassShape>) {
        let mut canvas = Canvas::new(W, H);
        for y in 0..H {
            for x in 0..W {
                // A ground with stripes and hard-edged blocks, like text.
                let stripe = if (x / 7 + y / 5) % 2 == 0 { 40 } else { 0 };
                let block = if (x % 23 < 9) && (y % 11 < 6) { 150 } else { 0 };
                let r = (20 + x * 60 / W + stripe + block).min(255) as u32;
                let g = (24 + y * 50 / H + block).min(255) as u32;
                let b = (30 + stripe + block / 2).min(255) as u32;
                canvas.px[(y * W + x) as usize] = r << 16 | g << 8 | b;
            }
        }
        let shapes = vec![GlassShape::new(Rect::new(100.0, 4.0, 120.0, 19.0), 9.5, 0), GlassShape::new(Rect::new(134.0, 157.0, 52.0, 10.0), 5.0, 1)];
        (canvas, shapes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// On a system with Mesa (the device image, or its ARM tests under
        /// emulation, where the software renderer stands in for the
        /// VideoCore) the GPU must open and draw as the processor does.
        #[test]
        fn the_gpu_draws_as_the_processor_does() {
            if Lib::open("libEGL.so.1").is_err() {
                eprintln!("no Mesa here: nothing to test");
                return;
            }
            let mut gpu = Gpu::open(true).unwrap_or_else(|why| panic!("the GPU did not open: {why}"));
            eprintln!("{}", note());
            eprintln!("{}", gpu.jobs());
            gpu.use_everything();
            // And again, so the kept glass is used: the same frame back.
            let (scene, shapes) = test_scene();
            let tokens = Tokens::dark();
            let mut first = scene.clone();
            assert!(gpu.glass(&mut first, &shapes, &tokens, bezel::LIGHT_REST, false).unwrap());
            let mut again = scene.clone();
            assert!(gpu.glass(&mut again, &shapes, &tokens, bezel::LIGHT_REST, false).unwrap());
            assert!(first.px == again.px, "kept glass comes back unchanged");
            // Glass that melts within a group is left to the processor.
            let merged = [GlassShape::new(Rect::new(10.0, 10.0, 40.0, 20.0), 10.0, 3), GlassShape::new(Rect::new(55.0, 10.0, 40.0, 20.0), 10.0, 3)];
            assert!(!gpu.glass(&mut first, &merged, &tokens, bezel::LIGHT_REST, false).unwrap());
            // The calm version and light mode, too.
            for (tokens, calm) in [(Tokens::light(), false), (Tokens::dark(), true)] {
                let mut cpu = scene.clone();
                bezel::Glass::new().render(&mut cpu, &shapes, &tokens, bezel::LIGHT_REST, calm);
                let mut on_gpu = scene.clone();
                gpu.kept.clear();
                gpu.glass(&mut on_gpu, &shapes, &tokens, bezel::LIGHT_REST, calm).unwrap();
                let d = Difference::of(&cpu.px, &on_gpu.px);
                assert!(d.close(), "calm {calm}, light {}: {d}", tokens.tone == bezel::Tone::Light);
            }
            // As the shell does it: opened on a thread of its own, drawn with on this one.
            drop(gpu);
            let mut handed = std::thread::spawn(|| Gpu::open(true).inspect(|g| g.release())).join().unwrap().unwrap();
            handed.adopt().unwrap();
            handed.use_everything();
            let mut cpu = scene.clone();
            bezel::Glass::new().render(&mut cpu, &shapes, &tokens, bezel::LIGHT_REST, false);
            let mut on_gpu = scene.clone();
            assert!(handed.glass(&mut on_gpu, &shapes, &tokens, bezel::LIGHT_REST, false).unwrap());
            let d = Difference::of(&cpu.px, &on_gpu.px);
            assert!(d.close(), "handed over: {d}");
        }
    }
}
