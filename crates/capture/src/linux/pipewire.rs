//! Consumes the portal's PipeWire node on a dedicated thread and pushes frames into the sink.

use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::sync::mpsc;

use brp_proto::{PixelFormat, monotonic_us};
use pipewire as pw;
use pw::spa::buffer::DataType;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::param::{ParamType, format_utils};
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, Value};
use pw::spa::utils::{
    Choice, ChoiceEnum, ChoiceFlags, Direction, Fraction, Id, Rectangle, SpaTypes,
};

use crate::error::CaptureError;
use crate::frame::{CaptureFrame, FrameSink, SourceInfo};

pub(crate) enum PwEvent {
    Format(SourceInfo),
    Error(CaptureError),
}

struct UserData {
    info: VideoInfoRaw,
    format: Option<PixelFormat>,
    size: (u32, u32),
    events: mpsc::Sender<PwEvent>,
    sink: FrameSink,
    target_fps: u32,
}

pub(crate) fn run_stream(
    fd: OwnedFd,
    node_id: u32,
    target_fps: u32,
    events: mpsc::Sender<PwEvent>,
    sink: FrameSink,
    quit: pw::channel::Receiver<()>,
) -> Result<(), CaptureError> {
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(pw_error)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(pw_error)?;
    let core = context.connect_fd_rc(fd, None).map_err(pw_error)?;
    let stream = pw::stream::StreamBox::new(
        &core,
        "brp-screen-capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(pw_error)?;
    let user_data = UserData {
        info: VideoInfoRaw::default(),
        format: None,
        size: (0, 0),
        events,
        sink,
        target_fps,
    };
    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .param_changed(|_, ud, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                return;
            };
            if media_type != MediaType::Video
                || media_subtype != MediaSubtype::Raw
                || ud.info.parse(param).is_err()
            {
                return;
            }
            let size = ud.info.size();
            ud.size = (size.width, size.height);
            let Some(format) = pixel_format(ud.info.format()) else {
                let _ = ud
                    .events
                    .send(PwEvent::Error(CaptureError::UnsupportedFormat(format!(
                        "{:?}",
                        ud.info.format()
                    ))));
                return;
            };
            ud.format = Some(format);
            let framerate = ud.info.framerate();
            let max_framerate = ud.info.max_framerate();
            let fps = negotiated_fps(
                (framerate.num, framerate.denom),
                (max_framerate.num, max_framerate.denom),
                ud.target_fps,
            );
            let _ = ud.events.send(PwEvent::Format(SourceInfo {
                width: size.width,
                height: size.height,
                fps,
            }));
        })
        .process(|stream, ud| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(format) = ud.format else { return };
            let (width, height) = ud.size;
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            if !matches!(data.type_(), DataType::MemPtr | DataType::MemFd) {
                return;
            }
            let chunk = data.chunk();
            let offset = chunk.offset() as usize;
            let size = chunk.size() as usize;
            let stride = if chunk.stride() > 0 {
                chunk.stride() as usize
            } else {
                width as usize * format.bytes_per_pixel()
            };
            let Some(bytes) = data.data() else { return };
            let Some(pixels) = bytes.get(offset..offset.saturating_add(size)) else {
                return;
            };
            (ud.sink)(CaptureFrame {
                width,
                height,
                stride,
                format,
                data: pixels.to_vec(),
                capture_ts_us: monotonic_us(),
            });
        })
        .register()
        .map_err(pw_error)?;

    let format_pod = enum_format_pod(target_fps);
    let Some(pod) = Pod::from_bytes(&format_pod) else {
        return Err(CaptureError::PipeWire(
            "format pod did not serialize".into(),
        ));
    };
    let mut params = [pod];
    stream
        .connect(
            Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(pw_error)?;
    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });
    mainloop.run();
    Ok(())
}

fn pw_error(error: pw::Error) -> CaptureError {
    CaptureError::PipeWire(error.to_string())
}

pub(crate) fn pixel_format(format: VideoFormat) -> Option<PixelFormat> {
    match format {
        VideoFormat::BGRx => Some(PixelFormat::Bgrx),
        VideoFormat::BGRA => Some(PixelFormat::Bgra),
        VideoFormat::RGBx => Some(PixelFormat::Rgbx),
        VideoFormat::RGBA => Some(PixelFormat::Rgba),
        _ => None,
    }
}

pub(crate) fn negotiated_fps(framerate: (u32, u32), max_framerate: (u32, u32), target: u32) -> u32 {
    let ratio = |(num, denom): (u32, u32)| {
        if num == 0 || denom == 0 {
            0
        } else {
            (f64::from(num) / f64::from(denom)).round() as u32
        }
    };
    match (ratio(framerate), ratio(max_framerate)) {
        (fixed, _) if fixed > 0 => fixed,
        (_, max) if max > 0 => max,
        _ => target.max(1),
    }
}

fn enum_format_pod(target_fps: u32) -> Vec<u8> {
    let id = |value: u32| Value::Id(Id(value));
    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: vec![
            Property::new(
                FormatProperties::MediaType.as_raw(),
                id(MediaType::Video.as_raw()),
            ),
            Property::new(
                FormatProperties::MediaSubtype.as_raw(),
                id(MediaSubtype::Raw.as_raw()),
            ),
            Property::new(
                FormatProperties::VideoFormat.as_raw(),
                Value::Choice(ChoiceValue::Id(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum {
                        default: Id(VideoFormat::BGRx.as_raw()),
                        alternatives: vec![
                            Id(VideoFormat::BGRx.as_raw()),
                            Id(VideoFormat::BGRA.as_raw()),
                            Id(VideoFormat::RGBx.as_raw()),
                            Id(VideoFormat::RGBA.as_raw()),
                        ],
                    },
                ))),
            ),
            Property::new(
                FormatProperties::VideoSize.as_raw(),
                Value::Choice(ChoiceValue::Rectangle(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Rectangle {
                            width: 1920,
                            height: 1080,
                        },
                        min: Rectangle {
                            width: 1,
                            height: 1,
                        },
                        max: Rectangle {
                            width: 8192,
                            height: 8192,
                        },
                    },
                ))),
            ),
            Property::new(
                FormatProperties::VideoFramerate.as_raw(),
                Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Fraction {
                            num: target_fps,
                            denom: 1,
                        },
                        min: Fraction { num: 0, denom: 1 },
                        max: Fraction {
                            num: 1000,
                            denom: 1,
                        },
                    },
                ))),
            ),
            Property::new(
                FormatProperties::VideoMaxFramerate.as_raw(),
                Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Fraction {
                            num: target_fps,
                            denom: 1,
                        },
                        min: Fraction { num: 1, denom: 1 },
                        max: Fraction {
                            num: 1000,
                            denom: 1,
                        },
                    },
                ))),
            ),
        ],
    };
    PodSerializer::serialize(Cursor::new(Vec::new()), &Value::Object(obj))
        .expect("the capture format is a valid SPA object")
        .0
        .into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_prefers_a_fixed_rate_then_max_rate_then_the_request() {
        assert_eq!(negotiated_fps((60, 1), (0, 1), 30), 60);
        assert_eq!(negotiated_fps((0, 1), (144, 1), 30), 144);
        assert_eq!(negotiated_fps((0, 1), (0, 1), 30), 30);
        assert_eq!(negotiated_fps((30000, 1001), (0, 1), 60), 30);
    }

    #[test]
    fn only_32_bit_packed_formats_are_accepted() {
        assert_eq!(pixel_format(VideoFormat::BGRx), Some(PixelFormat::Bgrx));
        assert_eq!(pixel_format(VideoFormat::BGRA), Some(PixelFormat::Bgra));
        assert_eq!(pixel_format(VideoFormat::RGBx), Some(PixelFormat::Rgbx));
        assert_eq!(pixel_format(VideoFormat::RGBA), Some(PixelFormat::Rgba));
        assert_eq!(pixel_format(VideoFormat::NV12), None);
    }
}
