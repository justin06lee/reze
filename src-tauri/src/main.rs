// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // WebKitGTK's DMA-BUF renderer draws a blank window on NVIDIA's drivers
    // (Tegra included) and under Xvfb. The fallback costs nothing a palette
    // would notice. Set here rather than in a launcher so every way of starting
    // the app gets it; an explicit value from the environment still wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // On an NVIDIA Jetson the hardware H.264 decoder outranks every other, and
    // hands its frames back in GPU memory WebKitGTK's video sink cannot take:
    // every video preview fails with "internal data stream error". Ranking it
    // out for this process leaves the software decoder, which is plenty for a
    // thumbnail. A no-op anywhere the decoder does not exist.
    #[cfg(target_os = "linux")]
    {
        let demote = "nvv4l2decoder:NONE";
        let ranks = match std::env::var("GST_PLUGIN_FEATURE_RANK") {
            Ok(set) if set.contains("nvv4l2decoder") => set,
            Ok(set) if !set.is_empty() => format!("{set},{demote}"),
            _ => demote.to_string(),
        };
        std::env::set_var("GST_PLUGIN_FEATURE_RANK", ranks);
    }

    reze_lib::run()
}
