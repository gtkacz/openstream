use crate::{
    cli::WatchArgs,
    error::AppError,
    identity,
    window::{App, AppEvent},
};
use brp_codec::open_decoder;
use brp_net::{MediaClient, RelaySetting, bind_endpoint};
use brp_pipeline::Viewer;
use brp_proto::{PublisherMessage, RoomTicket};
use std::str::FromStr;
use std::sync::Arc;
use tokio::runtime::Runtime;
use winit::event_loop::EventLoop;

const LIVE_ID: u32 = 1;
const PRESET_ID: u32 = 1;

pub fn run(runtime: &Runtime, args: WatchArgs) -> Result<(), AppError> {
    let ticket = RoomTicket::from_str(&args.ticket)?;
    let bootstrap = ticket
        .bootstrap
        .first()
        .cloned()
        .ok_or(AppError::EmptyTicket)?;
    let relay = if args.no_relay {
        RelaySetting::Disabled
    } else {
        RelaySetting::Default
    };
    let (endpoint, client, subscription) = runtime.block_on(async {
        let endpoint = bind_endpoint(identity::load_or_create()?, relay).await?;
        let client = MediaClient::connect(&endpoint, bootstrap).await?;
        let subscription = client.subscribe(LIVE_ID, PRESET_ID).await?;
        Ok::<_, AppError>((endpoint, client, subscription))
    })?;
    let params = subscription.params.clone();
    let decoder = open_decoder(&params)?;
    let publisher = client.remote_id().fmt_short();
    println!(
        "Subscribed to {publisher}: {:?} {}x{} @ {} fps",
        params.codec, params.width, params.height, params.fps
    );
    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .map_err(|e| AppError::Window(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    let frame_proxy = proxy.clone();
    let viewer = Viewer::start(
        runtime.handle().clone(),
        subscription.frames,
        subscription.control,
        decoder,
        Arc::new(move || {
            let _ = frame_proxy.send_event(AppEvent::NewFrame);
        }),
    );
    let mut events = subscription.events;
    runtime.spawn(async move {
        while let Some(msg) = events.recv().await {
            if matches!(msg, PublisherMessage::LiveEnded) {
                let _ = proxy.send_event(AppEvent::Status("live ended by the publisher".into()));
            }
        }
    });
    let description = format!(
        "{:?} {}x{} @ {} fps from {publisher}",
        params.codec, params.width, params.height, params.fps
    );
    let mut app = App::new(
        format!("brp: {publisher}"),
        description,
        viewer.slot(),
        viewer.stats(),
    );
    let outcome = event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::Window(e.to_string()));
    viewer.stop();
    client.close();
    runtime.block_on(endpoint.close());
    outcome
}
