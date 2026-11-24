//! The browser's own media pipeline: a detached `<video>` element plays the
//! file, a detached canvas draws each picture, and the pixels read back as
//! RGBA.
//!
//! `wasm32-unknown-unknown` has no threads and nothing to block on, so the
//! browser keeps the clock: the element plays at the playback rate and the
//! player reads whichever picture the element shows when the screen draws.
//! `read` therefore reports the position the browser is at, and `has_frame`
//! states whether the browser has moved past the picture read last.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    Blob, BlobPropertyBag, CanvasRenderingContext2d, Document, HtmlCanvasElement,
    HtmlScriptElement, HtmlVideoElement, ImageData, Url,
};

use crate::{DecodedFrame, VideoDecoder, VideoError, VideoInfo, VideoSource};

pub(crate) const NAME: &str = "HTML video";
pub(crate) const SUPPORTED: bool = true;

/// The smallest change of position that counts as a new picture. The browser
/// states time in seconds as a double, so an unchanged picture repeats the
/// same number exactly.
const STEP: f64 = 1e-4;

/// `HTMLMediaElement.HAVE_CURRENT_DATA`, the first state in which a picture can
/// be drawn.
const HAVE_CURRENT_DATA: u16 = 2;

/// The MIME type of an HLS playlist, which Safari plays on its own.
const HLS_TYPE: &str = "application/vnd.apple.mpegurl";

/// Where a browser without native HLS fetches `hls.js` from.
const HLS_JS: &str = "https://cdn.jsdelivr.net/npm/hls.js@1/dist/hls.min.js";

pub(crate) fn open(source: &VideoSource) -> Result<Box<dyn VideoDecoder>, VideoError> {
    let document = document()?;
    let element: HtmlVideoElement = document
        .create_element("video")
        .ok()
        .and_then(|element| element.dyn_into().ok())
        .ok_or_else(|| VideoError::Source("the browser created no video element".to_owned()))?;
    // A muted element may play without a person's consent, which is what
    // decoding ahead needs. Sound is opt-in: the app unmutes with
    // `set_muted(false)` after a person interacts with the page, when the
    // browser allows it.
    element.set_muted(true);
    element.set_preload("auto");
    let _ = element.set_attribute("playsinline", "");
    let _ = element.set_attribute("webkit-playsinline", "");

    let hls = source.is_hls();
    if hls {
        // The playlist names fragments on its own host, which the browser
        // fetches itself. The server has to allow this page to read them.
        let _ = element.set_attribute("crossorigin", "anonymous");
    }

    let object_url = match source {
        VideoSource::Bytes(bytes) => Some(object_url(bytes)?),
        VideoSource::Path(path) => Some(path.to_string_lossy().into_owned()),
        VideoSource::Url(url) => Some(url.to_string()),
    };
    let Some(url) = object_url else {
        return Err(VideoError::Source("the clip has no address".to_owned()));
    };
    element.set_src(&url);
    // Opening starts the media pipeline. A browser refusing to play leaves the
    // element paused, and `read` still draws from the position a seek sets.
    let _ = element.play();

    // A browser that plays HLS itself, and every source that is not HLS, needs
    // no script. Elsewhere `hls.js` reads the playlist and feeds the element.
    let attach = if hls && element.can_play_type(HLS_TYPE).is_empty() {
        Some(attach_hls(&element, &url)?)
    } else {
        None
    };

    Ok(Box::new(Reader {
        element,
        attach,
        object_url: matches!(source, VideoSource::Bytes(_)).then_some(url),
        surface: None,
        last_time: -1.0,
        finished: false,
    }))
}

fn document() -> Result<Document, VideoError> {
    web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| VideoError::Source("the page has no document".to_owned()))
}

/// A `Blob` URL for bytes the caller holds, such as a clip inside the page's
/// own data.
fn object_url(bytes: &[u8]) -> Result<String, VideoError> {
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let bag = BlobPropertyBag::new();
    bag.set_type("video/mp4");
    Blob::new_with_u8_array_sequence_and_options(parts.as_ref(), &bag)
        .and_then(|blob| Url::create_object_url_with_blob(&blob))
        .map_err(|_| VideoError::Source("the browser created no blob for the clip".to_owned()))
}

/// `hls.js` reading a stream the browser cannot read itself.
///
/// The script is fetched once per page and left on the window, so a second
/// player finds it loaded. The player object destroys its own media source,
/// which releases the element and its network buffers.
struct Attach {
    /// The script this reader added, when it added one.
    script: Option<HtmlScriptElement>,
    /// The player `hls.js` created, once the script has run.
    player: Arc<Mutex<Option<JsValue>>>,
    /// Why the stream never started, when it did not.
    problem: Arc<Mutex<Option<String>>>,
}

impl Attach {
    /// Whether the stream is ready to be drawn, or failed to start.
    fn ready(&self) -> bool {
        self.player.lock().is_some() || self.problem.lock().is_some()
    }

    /// Why the script did not load, when it did not.
    fn problem(&self) -> Option<String> {
        self.problem.lock().clone()
    }

    /// Stops `hls.js` and takes the script out of the page.
    fn destroy(&self) {
        if let Some(player) = self.player.lock().take()
            && let Ok(destroy) = js_sys::Reflect::get(&player, &JsValue::from_str("destroy"))
                .and_then(|value| value.dyn_into::<js_sys::Function>().map_err(Into::into))
        {
            let _ = destroy.call0(&player);
        }
        if let Some(script) = &self.script {
            script.remove();
        }
    }
}

/// Loads `hls.js` when the page does not already carry it, and points it at
/// `url`.
fn attach_hls(element: &HtmlVideoElement, url: &str) -> Result<Attach, VideoError> {
    let player = Arc::new(Mutex::new(None));
    let problem = Arc::new(Mutex::new(None));

    // A page that has already run the script, or a player after the first,
    // starts at once. `Reflect` keeps the check from needing the script's
    // types, which this crate does not carry.
    if hls_constructor().is_some() {
        let attach = Attach {
            script: None,
            player: Arc::clone(&player),
            problem: Arc::clone(&problem),
        };
        match start_hls(element, url) {
            Ok(started) => *attach.player.lock() = Some(started),
            Err(message) => *attach.problem.lock() = Some(message),
        }
        return Ok(attach);
    }

    let document = document()?;
    let script: HtmlScriptElement = document
        .create_element("script")
        .ok()
        .and_then(|element| element.dyn_into().ok())
        .ok_or_else(|| VideoError::Source("the browser created no script".to_owned()))?;

    // The element keeps the callbacks, and `once_into_js` releases them after
    // the browser calls one, so neither is a closure this struct owns.
    let address = url.to_owned();
    let when_loaded = Arc::clone(&player);
    let missing = Arc::clone(&problem);
    let element_for_load = element.clone();
    let loaded = Closure::once_into_js(move || match start_hls(&element_for_load, &address) {
        Ok(started) => *when_loaded.lock() = Some(started),
        Err(message) => *missing.lock() = Some(message),
    });
    let failed = Closure::once_into_js({
        let problem = Arc::clone(&problem);
        move || *problem.lock() = Some(format!("the browser did not load {HLS_JS}"))
    });

    script.set_src(HLS_JS);
    script.set_onload(Some(loaded.unchecked_ref()));
    script.set_onerror(Some(failed.unchecked_ref()));
    let head = document
        .head()
        .ok_or_else(|| VideoError::Source("the page has no head".to_owned()))?;
    head.append_child(&script)
        .map_err(|_| VideoError::Source("the page refused the hls.js script".to_owned()))?;

    Ok(Attach {
        script: Some(script),
        player,
        problem,
    })
}

/// The `Hls` constructor the page carries, once the script has run.
fn hls_constructor() -> Option<js_sys::Function> {
    let window = web_sys::window()?;
    js_sys::Reflect::get(&window, &JsValue::from_str("Hls"))
        .ok()
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
}

/// Creates an `hls.js` player on `element`, pointed at `url`.
fn start_hls(element: &HtmlVideoElement, url: &str) -> Result<JsValue, String> {
    let constructor = hls_constructor().ok_or_else(|| "the page carries no hls.js".to_owned())?;
    let player = js_sys::Reflect::construct(&constructor, &js_sys::Array::new())
        .map_err(|_| "hls.js did not start".to_owned())?;
    method(&player, "attachMedia", element.as_ref())
        .map_err(|_| "hls.js did not take the video element".to_owned())?;
    method(&player, "loadSource", &JsValue::from_str(url))
        .map_err(|_| "hls.js did not take the stream address".to_owned())?;
    Ok(player)
}

/// Calls `name` on `target` with one argument.
fn method(target: &JsValue, name: &str, argument: &JsValue) -> Result<JsValue, JsValue> {
    let function = js_sys::Reflect::get(target, &JsValue::from_str(name))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| JsValue::from_str(name))?;
    function.call1(target, argument)
}

/// The canvas a picture is drawn into and read back from. The context holds
/// the canvas alive, so nothing here needs to.
struct Surface {
    context: CanvasRenderingContext2d,
    width: u32,
    height: u32,
}

impl Surface {
    fn new(document: &Document, width: u32, height: u32) -> Result<Self, VideoError> {
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .ok()
            .and_then(|element| element.dyn_into().ok())
            .ok_or_else(|| VideoError::Source("the browser created no canvas".to_owned()))?;
        canvas.set_width(width);
        canvas.set_height(height);
        let context = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|context| context.dyn_into::<CanvasRenderingContext2d>().ok())
            .ok_or_else(|| VideoError::Source("the browser has no 2d canvas".to_owned()))?;
        Ok(Self {
            context,
            width,
            height,
        })
    }
}

struct Reader {
    element: HtmlVideoElement,
    /// The `hls.js` player feeding the element, when the browser could not read
    /// the stream itself.
    attach: Option<Attach>,
    object_url: Option<String>,
    surface: Option<Surface>,
    last_time: f64,
    finished: bool,
}

impl Reader {
    fn size(&self) -> (u32, u32) {
        (self.element.video_width(), self.element.video_height())
    }

    fn failure(&self) -> Option<VideoError> {
        self.element.error().map(|error| {
            VideoError::Decode(format!(
                "the browser's media pipeline stopped: {} ({})",
                error.message(),
                error.code()
            ))
        })
    }

    /// Why `hls.js` never started, when it did not.
    fn stream_failure(&self) -> Option<VideoError> {
        self.attach
            .as_ref()
            .and_then(Attach::problem)
            .map(|why| VideoError::Source(format!("the browser did not play the stream: {why}")))
    }

    /// Draws the picture the element shows and reads it back.
    fn draw(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        let (width, height) = self.size();
        if width == 0 || height == 0 {
            return Ok(None);
        }
        if self
            .surface
            .as_ref()
            .is_none_or(|surface| surface.width != width || surface.height != height)
        {
            let document = document()?;
            self.surface = Some(Surface::new(&document, width, height)?);
        }
        let Some(surface) = self.surface.as_ref() else {
            return Ok(None);
        };
        surface
            .context
            .draw_image_with_html_video_element_and_dw_and_dh(
                &self.element,
                0.0,
                0.0,
                f64::from(width),
                f64::from(height),
            )
            .map_err(|_| {
                VideoError::Decode("the browser drew no picture from the video".to_owned())
            })?;
        let image: ImageData = surface
            .context
            .get_image_data(0.0, 0.0, f64::from(width), f64::from(height))
            .map_err(|_| {
                VideoError::Decode("the browser read no pixels from the canvas".to_owned())
            })?;
        let pixels = image.data();
        rgba.clear();
        rgba.extend_from_slice(pixels.as_slice());
        let seconds = self.element.current_time();
        self.last_time = seconds;
        Ok(Some(DecodedFrame {
            pts: Duration::from_secs_f64(seconds.max(0.0)),
            width,
            height,
        }))
    }
}

impl VideoDecoder for Reader {
    fn info(&self) -> VideoInfo {
        let (width, height) = self.size();
        VideoInfo {
            width,
            height,
            duration: stated_duration(&self.element),
            frame_rate: None,
            rotation: Default::default(),
        }
    }

    fn seek(&mut self, position: Duration) -> Result<(), VideoError> {
        self.element.set_current_time(position.as_secs_f64());
        self.last_time = -1.0;
        self.finished = false;
        Ok(())
    }

    fn read(&mut self, rgba: &mut Vec<u8>) -> Result<Option<DecodedFrame>, VideoError> {
        if let Some(error) = self.stream_failure() {
            return Err(error);
        }
        if let Some(error) = self.failure() {
            return Err(error);
        }
        if self.element.ended() {
            self.finished = true;
            return Ok(None);
        }
        self.draw(rgba)
    }

    fn has_frame(&self) -> bool {
        !self.finished
            && !self.element.seeking()
            && self.element.ready_state() >= HAVE_CURRENT_DATA
            && self.element.current_time() > self.last_time + STEP
    }

    fn ready(&self) -> bool {
        // `hls.js` fills the element only after its script has run, so reading
        // before then draws an empty canvas and claims no picture forever.
        self.attach.as_ref().is_none_or(Attach::ready)
    }

    fn is_live(&self) -> bool {
        // `hls.js` keeps `duration` at `Infinity` for a live stream, and a
        // browser playing HLS itself reports the same.
        self.element.duration().is_infinite()
    }

    fn is_seekable(&self) -> bool {
        !self.is_live()
    }

    fn play(&mut self) {
        let _ = self.element.play();
    }

    fn pause(&mut self) {
        let _ = self.element.pause();
    }

    fn set_speed(&mut self, speed: f32) {
        self.element.set_playback_rate(f64::from(speed));
    }

    fn set_muted(&mut self, muted: bool) {
        self.element.set_muted(muted);
    }

    fn set_volume(&mut self, volume: f32) {
        self.element.set_volume(f64::from(volume));
    }

    fn is_muted(&self) -> bool {
        self.element.muted()
    }

    fn volume(&self) -> f32 {
        self.element.volume() as f32
    }

    fn close(&mut self) {
        let _ = self.element.pause();
        if let Some(attach) = self.attach.take() {
            attach.destroy();
        }
        let _ = self.element.remove_attribute("src");
        if let Some(url) = self.object_url.take() {
            Url::revoke_object_url(&url).ok();
        }
        self.element.load();
        self.surface = None;
    }
}

/// How long the media runs. A live stream states no finite length.
fn stated_duration(element: &HtmlVideoElement) -> Option<Duration> {
    let seconds = element.duration();
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}
