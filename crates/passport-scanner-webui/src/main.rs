//! The browser client: the shared interface, talking to the local service over HTTP.
//!
//! Events arrive on the service's server-sent-event stream, commands go back as
//! posts - the same wire contract the native window speaks in-process. `?demo`
//! in the address runs against the simulated scanner instead.

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "passport-scanner-webui runs in a browser. Build it with \
         `cargo build --target wasm32-unknown-unknown -p passport-scanner-webui` (see package.ps1)."
    );
    std::process::exit(2);
}

#[cfg(target_arch = "wasm32")]
fn main() -> iced::Result {
    web::run()
}

#[cfg(target_arch = "wasm32")]
mod web {
    use iced::Task;
    use passport_scanner_protocol::{Command, Event};
    use passport_scanner_ui::{
        App, Appearance, Backend, ExportOutcome, ExportRequest, Inbound, Locale, Message,
        Preferences, Store,
        mock::Mock,
        pdf_name::{self, Stamp},
    };
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};
    use wasm_bindgen_futures::{JsFuture, spawn_local};

    /// Replaced by the canvas; see `index.html`.
    const MOUNT: &str = "passport-scanner";

    pub(crate) fn run() -> iced::Result {
        console_error_panic_hook::set_once();
        let demo = window().location().search().is_ok_and(|query| query.contains("demo"));
        iced::application(
            move || {
                let backend: Box<dyn Backend> =
                    if demo { Box::new(Mock::new()) } else { Box::new(Http::default()) };
                App::new(backend, Box::new(LocalStore))
            },
            App::update,
            App::view,
        )
        .title(passport_scanner_ui::TITLE)
        .theme(App::theme)
        .settings(passport_scanner_ui::settings())
        .subscription(App::subscription)
        .window(iced::window::Settings {
            platform_specific: iced::window::settings::PlatformSpecific {
                target: Some(MOUNT.into()),
            },
            ..Default::default()
        })
        .run()
    }

    fn window() -> web_sys::Window {
        web_sys::window().expect("running in a browser window")
    }

    /// What the stream callbacks share with the interface's polling.
    #[derive(Debug, Default)]
    struct Link {
        inbound: Vec<Inbound>,
        /// `None` until the first attempt settles, so a service that was never
        /// reachable is reported as well as one that went away.
        open: Option<bool>,
    }

    impl Link {
        fn settle(&mut self, open: bool) {
            if self.open != Some(open) {
                self.open = Some(open);
                self.inbound.push(Inbound::Connection(open));
            }
        }
    }

    #[derive(Default)]
    struct Http {
        link: Rc<RefCell<Link>>,
        source: Option<web_sys::EventSource>,
        /// When the current stream was opened, in milliseconds since the epoch.
        opened_at: f64,
        // The stream holds only references; these keep its callbacks alive.
        callbacks: Vec<Closure<dyn FnMut(web_sys::MessageEvent)>>,
    }

    /// Between attempts to reach a service that refused or was not running.
    const RETRY_MS: f64 = 3000.0;

    impl std::fmt::Debug for Http {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Http").field("link", &self.link).finish_non_exhaustive()
        }
    }

    impl Backend for Http {
        fn connect(&mut self) -> Result<Vec<Event>, String> {
            // The browser reconnects an open stream on its own; the service then
            // replays the full state, settings and calibration.
            if self.source.is_some() {
                return Ok(Vec::new());
            }
            let source = web_sys::EventSource::new("/events").map_err(describe)?;
            let link = Rc::clone(&self.link);
            let message = Closure::<dyn FnMut(_)>::new(move |event: web_sys::MessageEvent| {
                let item = event
                    .data()
                    .as_string()
                    .and_then(|data| serde_json::from_str::<Event>(&data).ok())
                    .map_or(Inbound::Unreadable, Inbound::Event);
                link.borrow_mut().inbound.push(item);
            });
            let link = Rc::clone(&self.link);
            let open = Closure::<dyn FnMut(_)>::new(move |_: web_sys::MessageEvent| {
                link.borrow_mut().settle(true);
            });
            let link = Rc::clone(&self.link);
            let error = Closure::<dyn FnMut(_)>::new(move |_: web_sys::MessageEvent| {
                link.borrow_mut().settle(false);
            });
            source.set_onmessage(Some(message.as_ref().unchecked_ref()));
            source.set_onopen(Some(open.as_ref().unchecked_ref()));
            source.set_onerror(Some(error.as_ref().unchecked_ref()));
            self.callbacks = vec![message, open, error];
            self.source = Some(source);
            self.opened_at = js_sys::Date::now();
            Ok(Vec::new())
        }

        fn connected(&self) -> bool {
            self.link.borrow().open == Some(true)
        }

        fn send(&mut self, command: Command) -> Result<(), String> {
            let body = serde_json::to_string(&command).map_err(|e| e.to_string())?;
            let link = Rc::clone(&self.link);
            spawn_local(async move {
                if let Err(message) = post_command(body).await {
                    link.borrow_mut()
                        .inbound
                        .push(Inbound::Event(Event::Failed { code: "transport".into(), message }));
                }
            });
            Ok(())
        }

        fn poll(&mut self) -> Vec<Inbound> {
            // A browser retries a dropped stream itself, but gives up for good
            // on an error response or a refused connection; retry those here.
            let closed = self
                .source
                .as_ref()
                .is_some_and(|source| source.ready_state() == web_sys::EventSource::CLOSED);
            if closed && js_sys::Date::now() - self.opened_at >= RETRY_MS {
                if let Some(source) = self.source.take() {
                    source.close();
                }
                if let Err(message) = self.connect() {
                    self.link
                        .borrow_mut()
                        .inbound
                        .push(Inbound::Event(Event::Failed { code: "transport".into(), message }));
                }
            }
            std::mem::take(&mut self.link.borrow_mut().inbound)
        }

        fn open_link(&self, url: &'static str) -> Task<Message> {
            let _ = window().open_with_url_and_target(url, "_blank");
            Task::none()
        }

        fn export_pdf(&self, document_id: String, _: ExportRequest) -> Task<Message> {
            Task::perform(download_pdf(document_id), Message::Exported)
        }

        fn copy_text(&self, text: String) -> Task<Message> {
            let clipboard = window().navigator().clipboard();
            spawn_local(async move {
                let _ = JsFuture::from(clipboard.write_text(&text)).await;
            });
            Task::none()
        }
    }

    fn describe(error: JsValue) -> String {
        error.as_string().unwrap_or_else(|| format!("{error:?}"))
    }

    async fn fetch(url: &str, init: &web_sys::RequestInit) -> Result<web_sys::Response, String> {
        let request = web_sys::Request::new_with_str_and_init(url, init).map_err(describe)?;
        let response =
            JsFuture::from(window().fetch_with_request(&request)).await.map_err(describe)?;
        let response: web_sys::Response = response.dyn_into().map_err(describe)?;
        if response.ok() {
            return Ok(response);
        }
        let detail = match response.text() {
            Ok(text) => JsFuture::from(text).await.ok().and_then(|t| t.as_string()),
            Err(_) => None,
        };
        Err(detail
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| format!("HTTP {}", response.status())))
    }

    async fn post_command(body: String) -> Result<(), String> {
        let init = web_sys::RequestInit::new();
        init.set_method("POST");
        init.set_body(&JsValue::from_str(&body));
        fetch("/command", &init).await.map(drop)
    }

    /// Fetch first, so a refused export shows its reason instead of saving an error page.
    async fn download_pdf(document_id: String) -> Result<ExportOutcome, String> {
        let file_name = next_file_name();
        let id = String::from(js_sys::encode_uri_component(&document_id));
        let response = fetch(&format!("/document/{id}/pdf"), &web_sys::RequestInit::new()).await?;
        let blob: web_sys::Blob = JsFuture::from(response.blob().map_err(describe)?)
            .await
            .map_err(describe)?
            .dyn_into()
            .map_err(describe)?;
        let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(describe)?;
        let document = window().document().ok_or("no document")?;
        let anchor: web_sys::HtmlAnchorElement =
            document.create_element("a").map_err(describe)?.unchecked_into();
        anchor.set_href(&url);
        anchor.set_download(&file_name);
        anchor.click();
        let _ = web_sys::Url::revoke_object_url(&url);
        if let Some(storage) = LocalStore::storage() {
            let _ = storage.set_item(LAST_PDF, &file_name);
        }
        Ok(ExportOutcome::Downloaded)
    }

    /// A browser cannot look into its download folder, so the day's count
    /// continues from the last name this browser handed out.
    fn next_file_name() -> String {
        let now = js_sys::Date::new_0();
        let stamp = Stamp {
            year: now.get_full_year() as i32,
            month: now.get_month() as u8 + 1,
            day: now.get_date() as u8,
            hour: now.get_hours() as u8,
            minute: now.get_minutes() as u8,
            second: now.get_seconds() as u8,
        };
        let last = LocalStore::storage().and_then(|s| s.get_item(LAST_PDF).ok().flatten());
        stamp.file_name(pdf_name::next_number(&stamp.date(), last.as_deref()))
    }

    /// Per-browser preferences. Storage may be blocked; then nothing is remembered.
    #[derive(Debug)]
    struct LocalStore;

    const APPEARANCE: &str = "passport-scanner.appearance";
    const LOCALE: &str = "passport-scanner.locale";
    const LAST_PDF: &str = "passport-scanner.last-pdf";

    impl LocalStore {
        fn storage() -> Option<web_sys::Storage> {
            window().local_storage().ok().flatten()
        }
    }

    impl Store for LocalStore {
        fn load(&self) -> Result<Preferences, String> {
            let Some(storage) = Self::storage() else { return Ok(Preferences::default()) };
            let read = |key| storage.get_item(key).ok().flatten();
            Ok(Preferences {
                appearance: read(APPEARANCE)
                    .and_then(|v| Appearance::from_key(&v))
                    .unwrap_or_default(),
                locale: read(LOCALE).and_then(|v| Locale::from_key(&v)),
                ..Preferences::default()
            })
        }

        fn save(&self, preferences: Preferences) -> Result<(), String> {
            let storage = Self::storage().ok_or("browser storage is unavailable")?;
            storage.set_item(APPEARANCE, preferences.appearance.key()).map_err(describe)?;
            if let Some(locale) = preferences.locale {
                storage.set_item(LOCALE, locale.key()).map_err(describe)?;
            }
            Ok(())
        }

        fn system_locale(&self) -> Option<String> {
            window().navigator().language()
        }
    }
}
