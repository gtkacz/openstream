use std::os::fd::OwnedFd;

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream,
};
use ashpd::enumflags2::BitFlags;
use brp_proto::SourceKind;

use crate::error::CaptureError;

/// Keeps the portal session alive until PipeWire delivery has stopped.
pub(crate) struct PortalHandle {
    _keep_alive: tokio::sync::oneshot::Sender<()>,
}

pub(crate) struct PortalStream {
    pub node_id: u32,
    pub fd: OwnedFd,
    pub handle: PortalHandle,
}

pub(crate) async fn open_screencast(kind: SourceKind) -> Result<PortalStream, CaptureError> {
    let proxy = Screencast::new().await.map_err(portal_error)?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(portal_error)?;
    let sources = BitFlags::from(match kind {
        SourceKind::Monitor => SourceType::Monitor,
        SourceKind::Window => SourceType::Window,
    });
    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(CursorMode::Embedded)
                .set_sources(sources)
                .set_multiple(false)
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(portal_error)?;
    let streams = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(portal_error)?
        .response()
        .map_err(portal_error)?;
    let stream: Stream = streams
        .streams()
        .first()
        .cloned()
        .ok_or(CaptureError::PortalDenied)?;
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(portal_error)?;
    let (keep_alive, released) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _session = session;
        let _ = released.await;
    });
    Ok(PortalStream {
        node_id: stream.pipe_wire_node_id(),
        fd,
        handle: PortalHandle {
            _keep_alive: keep_alive,
        },
    })
}

fn portal_error(error: ashpd::Error) -> CaptureError {
    match error {
        ashpd::Error::Response(_) | ashpd::Error::NoResponse => CaptureError::PortalDenied,
        other => CaptureError::Portal(other.to_string()),
    }
}
