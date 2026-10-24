// SPDX-License-Identifier: GPL-3.0-or-later
//! Qt calls that cxx-qt-lib doesn't wrap (see `cpp/app_helpers.cpp`).

#[cxx::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("app_helpers.h");
        include!("video_view.h");
        include!("cxx-qt-lib/qqmlapplicationengine.h");
        type QQmlApplicationEngine = cxx_qt_lib::QQmlApplicationEngine;

        /// The latest picture of a video stream (32-bit BGRX rows), for
        /// `VideoView`s showing it. Any thread.
        fn video_frame(stream: &str, width: u32, height: u32, bgrx: &[u8]);
        /// The stream ended: its views go blank. Any thread.
        fn video_clear(stream: &str);

        /// Actual native queue/lifecycle checks; synthetic pixels on the window-gate GUI thread.
        #[cfg(target_os = "macos")]
        fn video_wakeup_burst_check() -> String;
        #[cfg(target_os = "macos")]
        fn video_wakeup_lifecycle_check() -> String;

        /// Must run before `QGuiApplication` is created.
        fn prepare_qt();
        /// Like `prepare_qt`, without the QML log file: for the installer
        /// and uninstaller, which mustn't create the app's data folder.
        fn prepare_qt_without_log();
        /// Like `prepare_qt`, for `--ui-check`: Qt's messages go to stderr
        /// instead of the log file, and QML warnings and errors are counted.
        fn prepare_qt_for_ui_check();
        /// How many QML warnings and errors there were since
        /// `prepare_qt_for_ui_check`.
        fn qml_problem_count() -> i32;
        fn add_app_icon_image(size: i32, rgba: &[u8]);
        fn apply_app_icon();
        fn keep_running_without_windows();
        /// Registers the fonts compiled in under `:/fonts/`; returns how
        /// many loaded.
        fn load_bundled_fonts() -> i32;
        /// Clears Qt pixmap caches and trims the process working set.
        fn trim_memory_caches();
        /// Records the active QML engine so `trim_memory_caches` can run GC and
        /// clear component caches.
        fn register_qml_app_engine(engine: Pin<&mut QQmlApplicationEngine>);
        /// Starts a native OS drag-and-drop operation for a local file or text.
        fn start_external_drag(file_path: &str, text_fallback: &str) -> bool;

        /// Installs the NectarLink and Qt translations for `language` (a
        /// locale name, or "" for the system's), looking in `dir` for the
        /// .qm files and replacing those installed before. Run after the
        /// `QGuiApplication` exists; before any QML engine loads, or later to
        /// retranslate the open windows. Returns true when NectarLink's own
        /// translation was installed.
        fn install_translator(dir: &str, language: &str) -> bool;

        /// `text` in the installed language (`QCoreApplication::translate`).
        fn translate(context: &str, text: &str) -> String;

        /// Decodes image bytes into a byte vector: 4-byte width (LE), 4-byte height (LE), followed by ARGB32 bytes.
        #[allow(dead_code)]
        fn decode_image(bytes: &[u8]) -> Vec<u8>;

        #[allow(dead_code)]
        fn mac_clipboard_change_count() -> i64;

        #[allow(dead_code)]
        fn mac_clipboard_read_text(is_private: &mut bool) -> String;

        #[allow(dead_code)]
        fn mac_clipboard_write_text(text: &str, is_private: bool) -> bool;

        #[allow(dead_code)]
        fn mac_local_offset_ms(at_ms: i64) -> i64;

        #[allow(dead_code)]
        fn mac_free_space(path: &str, ok: &mut bool) -> u64;
    }

    #[cfg(target_os = "macos")]
    unsafe extern "C++" {
        include!("mac_hfp.h");

        /// Main thread. Starts one owned attempt for an exact binding; both
        /// callbacks run synchronously on the main thread with `session`,
        /// and must not call back into this API. 0: nothing was started.
        fn mac_hfp_start(
            binding_json: &str,
            session: u64,
            lease: fn(u64, &[u8]) -> bool,
            event: fn(u64, &[u8]),
        ) -> u64;
        /// Main thread, never from a callback. Irreversible handback request.
        fn mac_hfp_handback(handle: u64) -> bool;
        /// Main thread, never from a callback. True once the handle is freed.
        fn mac_hfp_release(handle: u64) -> bool;

        /// Usage descriptions and authorization status; requests nothing.
        fn mac_hfp_prerequisites() -> String;
        /// Main thread, explicit user action only.
        fn mac_hfp_paired_gateways(ok: &mut bool) -> String;
        /// Already-paired phones with cached SDP/AG facts; side-effect free,
        /// ok is false when they can't be listed.
        fn mac_hfp_paired_phone_candidates(ok: &mut bool) -> String;
        /// Main thread, explicit user action only. Starts one SDP query for
        /// the exact already-paired phone: 1 started, 2 busy (an earlier
        /// query is still outstanding; nothing started), 0 refused. `event`
        /// runs once on Main with the request and result JSON when that
        /// query's OS completion actually arrives, wanted or not.
        fn mac_hfp_refresh_phone_sdp(address: &str, request: u64, event: fn(u64, &[u8])) -> i32;
        fn mac_hfp_default_endpoints(ok: &mut bool) -> String;
        /// Main thread, explicit user action only; macOS asks while undecided.
        /// Both requests retain the same process-lifetime fn item. It runs on
        /// Main after native completion/state updates, and only refreshes status.
        fn mac_hfp_request_bluetooth(changed: fn()) -> bool;
        /// Same callback lifetime/thread contract as the Bluetooth request.
        fn mac_hfp_request_microphone(changed: fn()) -> bool;
        /// Main thread.
        fn mac_hfp_pump_main_run_loop(seconds: f64);
    }
}
