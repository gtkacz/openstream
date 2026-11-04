//! `brp://` links on macOS. LaunchServices never passes the URL on the command line: it starts the
//! bundle (or finds it running) and sends a `GURL` Apple Event, which arrives here and becomes an
//! [`AppEvent::OpenLink`] for the window to route.

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSObject, NSObjectProtocol};
use winit::event_loop::EventLoopProxy;

use crate::window::AppEvent;

/// `kInternetEventClass` and `kAEGetURL`: both the four characters `GURL`.
const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
/// `keyDirectObject`: the event parameter holding the URL.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

struct Ivars {
    proxy: Mutex<EventLoopProxy<AppEvent>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "BrpLinkHandler"]
    #[ivars = Ivars]
    struct LinkHandler;

    unsafe impl NSObjectProtocol for LinkHandler {}

    impl LinkHandler {
        #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
        fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let Some(link) = event
                .paramDescriptorForKeyword(DIRECT_OBJECT)
                .and_then(|descriptor| descriptor.stringValue())
            else {
                tracing::warn!("a link event arrived without a URL");
                return;
            };
            let proxy = self
                .ivars()
                .proxy
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if proxy.send_event(AppEvent::OpenLink(link.to_string())).is_err() {
                tracing::warn!("a link arrived after the window closed");
            }
        }
    }
);

impl LinkHandler {
    fn new(proxy: EventLoopProxy<AppEvent>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            proxy: Mutex::new(proxy),
        });
        // SAFETY: NSObject's designated initialiser.
        unsafe { msg_send![super(this), init] }
    }
}

/// Routes `GURL` events to the window through `proxy`. Must run before the event loop starts:
/// the event that launched the app is delivered while it finishes launching.
pub fn install(proxy: EventLoopProxy<AppEvent>) {
    let handler = LinkHandler::new(proxy);
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // SAFETY: the selector names a method `LinkHandler` defines with the signature the manager
    // calls, and the handler is kept alive for the rest of the process below.
    unsafe {
        manager.setEventHandler_andSelector_forEventClass_andEventID(
            &handler,
            sel!(handleGetURLEvent:withReplyEvent:),
            GET_URL,
            GET_URL,
        );
    }
    // The manager does not retain its handler.
    std::mem::forget(handler);
}
