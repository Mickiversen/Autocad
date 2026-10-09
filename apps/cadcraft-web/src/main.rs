//! CADCraft in the browser.
//!
//! Runs the same [`cadcraft_ui_egui::CadApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`). URL flags: `?webgl` forces WebGL2, `?sample` opens the sample
//! drawing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
mod web {
    use cadcraft_engine::Session;
    use cadcraft_ui_egui::{CadApp, Services};
    use wasm_bindgen::JsCast as _;

    const CANVAS_ID: &str = "cadcraft_canvas";
    const LOADING_ID: &str = "cadcraft_loading";

    /// Largest file the open dialog will read (hostile or accidental huge files).
    const MAX_OPEN_BYTES: f64 = 256.0 * 1024.0 * 1024.0;

    thread_local! {
        /// Files chosen in the browser's file dialog, waiting for the app to open them.
        static INCOMING: std::cell::RefCell<Vec<(String, Vec<u8>)>> = const { std::cell::RefCell::new(Vec::new()) };
        static CTX: std::cell::RefCell<Option<egui::Context>> = const { std::cell::RefCell::new(None) };
    }

    struct Shell(CadApp);

    impl eframe::App for Shell {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            let files = INCOMING.with(|q| std::mem::take(&mut *q.borrow_mut()));
            for (name, bytes) in files {
                let data = cadcraft_engine::cmd::file::base64_encode(&bytes);
                match self.0.run("open", serde_json::json!({ "data": data, "name": name })) {
                    Ok(_) => {
                        self.0.ui.start_tab = false;
                        self.0.canvas.zoom_pending = true;
                    }
                    Err(e) => self.0.set_status(e),
                }
            }
            self.0.logic(ctx);
        }
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            self.0.ui(ui);
        }
    }

    fn query() -> String {
        web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
    }

    /// Offer `bytes` to the user as a file download named `name`.
    fn download(name: &str, bytes: &[u8]) {
        let result = (|| -> Result<(), wasm_bindgen::JsValue> {
            let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
            let parts = js_sys::Array::new();
            parts.push(&js_sys::Uint8Array::from(bytes));
            let opts = web_sys::BlobPropertyBag::new();
            opts.set_type("application/octet-stream");
            let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts)?;
            let url = web_sys::Url::create_object_url_with_blob(&blob)?;
            let a = document.create_element("a")?.dyn_into::<web_sys::HtmlAnchorElement>()?;
            a.set_href(&url);
            a.set_download(name);
            a.click();
            web_sys::Url::revoke_object_url(&url)?;
            Ok(())
        })();
        if let Err(e) = result {
            log::error!("download failed: {e:?}");
        }
    }

    /// Show the browser's file dialog; chosen files are queued in `INCOMING`.
    fn pick_file() {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
        let Some(input) = document.create_element("input").ok().and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok()) else {
            return;
        };
        input.set_type("file");
        input.set_accept(".dxf,.dwg");
        let handle = input.clone();
        let on_change = wasm_bindgen::closure::Closure::once_into_js(move || {
            let Some(file) = handle.files().and_then(|l| l.get(0)) else { return };
            if file.size() > MAX_OPEN_BYTES {
                log::error!("{} is too large to open", file.name());
                return;
            }
            wasm_bindgen_futures::spawn_local(async move {
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(buf) => {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        INCOMING.with(|q| q.borrow_mut().push((file.name(), bytes)));
                        CTX.with(|c| {
                            if let Some(ctx) = c.borrow().as_ref() {
                                ctx.request_repaint();
                            }
                        });
                    }
                    Err(e) => log::error!("reading {} failed: {e:?}", file.name()),
                }
            });
        });
        input.set_onchange(Some(on_change.unchecked_ref()));
        input.click();
    }

    pub fn start() {
        eframe::WebLogger::init(log::LevelFilter::Info).ok();
        cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
            read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
            write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
            plot: Some(|d, space, opts| cadcraft_io::plot(d, space, opts).map_err(|e| e.to_string())),
        });
        cadcraft_engine::cmd::file::set_save_sink(download);
        wasm_bindgen_futures::spawn_local(async {
            let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
            let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
                log::error!("missing <canvas id=\"{CANVAS_ID}\">");
                return;
            };
            let mut options = eframe::WebOptions::default();
            if query().contains("webgl")
                && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
            {
                create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
            }
            let result = eframe::WebRunner::new()
                .start(
                    canvas,
                    options,
                    Box::new(move |cc| {
                        CTX.with(|c| *c.borrow_mut() = Some(cc.egui_ctx.clone()));
                        let services = Services {
                            pick_open: Some(Box::new(|| {
                                pick_file();
                                None
                            })),
                            // The browser has no save dialog: the file name is kept and the drawing downloads.
                            pick_save: Some(Box::new(|name: &str| Some(name.to_string()))),
                        };
                        let mut app = CadApp::new(Session::new(), services);
                        if let Some(rs) = &cc.wgpu_render_state {
                            app.set_wgpu(rs);
                        }
                        if query().contains("sample") {
                            let _ = app.run("ui.sample", serde_json::json!({}));
                        }
                        Ok(Box::new(Shell(app)))
                    }),
                )
                .await;
            if let Some(el) = document.get_element_by_id(LOADING_ID) {
                match result {
                    Ok(()) => el.remove(),
                    Err(e) => {
                        el.set_inner_html(&format!("<p>CADCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>"))
                    }
                }
            }
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("cadcraft-web only runs in the browser: build it with `trunk build --release` in apps/cadcraft-web");
}
