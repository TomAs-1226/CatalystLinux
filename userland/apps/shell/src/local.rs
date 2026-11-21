//! The on-device model (llama.cpp's server on this handheld), as the
//! assistant sees it. A stand-in with the agreed interface: the on-device
//! helper's real module takes its place, which downloads the model, starts
//! `llama-server` on 127.0.0.1 when asked and stops it when idle.

/// Whether the model is on this handheld, ready to start.
pub fn ready() -> bool {
    false
}

/// The model's name as people know it.
pub fn model_name() -> &'static str {
    "on-device model"
}

/// Start the model's server if it is not running, wait until it answers,
/// and give its base URL (OpenAI-compatible, e.g. `http://127.0.0.1:8088/v1`).
pub fn start() -> Result<String, String> {
    Err("the on-device model is not on this build".into())
}

/// Stop the model's server, giving its memory back.
pub fn stop() {}

/// The model was just used: it stays up a while longer before stopping.
pub fn touch() {}
