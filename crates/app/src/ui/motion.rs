//! Interruptible motion. Every animated value is a critically damped spring: new input only moves
//! its target, so motion continues from the value on screen at the velocity it already has, and
//! anything can be reversed mid-flight without a jump. It settles without overshoot, because
//! nothing in this interface is thrown; bounce would be motion with no gesture to explain it.

use egui::{Context, Id, Rect, Ui, UiBuilder, Vec2, vec2};

/// Roughly how long each kind of motion takes to settle, in seconds.
pub const HOVER: f32 = 0.12;
pub const PRESS_DOWN: f32 = 0.05;
pub const PRESS_UP: f32 = 0.22;
pub const TRAVEL: f32 = 0.2;
pub const SURFACE: f32 = 0.28;
pub const REFLOW: f32 = 0.36;

/// A dropped frame shortens a step rather than teleporting the value.
pub const LONGEST_STEP: f32 = 0.05;
/// The step taken by a spring leaving rest. The time since its last frame says nothing, since an
/// idle window may not have drawn for minutes.
const FIRST_STEP: f32 = 1.0 / 60.0;
/// Close enough to stop asking for frames: a twentieth of a point across a 1000-point canvas.
const REST_DISTANCE: f32 = 0.00005;
const REST_SPEED: f32 = 0.0005;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Spring {
    value: f32,
    velocity: f32,
    time: f64,
    frame: u64,
}

impl Spring {
    fn at_rest(&self, target: f32) -> bool {
        (self.value - target).abs() < REST_DISTANCE && self.velocity.abs() < REST_SPEED
    }

    /// The closed form of a critically damped spring, which stays stable at any step size.
    fn advance(&mut self, target: f32, response: f32, dt: f32) {
        let stiffness = std::f32::consts::TAU / response;
        let offset = self.value - target;
        let drift = self.velocity + stiffness * offset;
        let decay = (-stiffness * dt).exp();
        let remaining = offset + drift * dt;
        self.value = target + remaining * decay;
        self.velocity = (drift - stiffness * remaining) * decay;
    }
}

fn drive(
    ctx: &Context,
    id: Id,
    target: f32,
    initial: f32,
    response: f32,
    restart_after_gap: bool,
) -> f32 {
    let (now, frame) = (ctx.input(|i| i.time), ctx.cumulative_frame_nr());
    let fresh = Spring {
        value: initial,
        velocity: 0.0,
        time: now,
        frame,
    };
    let stored = ctx.data(|data| data.get_temp::<Spring>(id));
    let mut spring = match stored {
        // Frames count only while the window draws, so a gap means this value was not on screen.
        Some(stored) if restart_after_gap && frame > stored.frame + 1 => fresh,
        Some(stored) => stored,
        None => fresh,
    };
    let dt = if spring.velocity == 0.0 {
        FIRST_STEP
    } else {
        ((now - spring.time) as f32).clamp(0.0, LONGEST_STEP)
    };
    // A second pass of the same frame has no time to account for.
    if now > spring.time || spring.velocity == 0.0 {
        spring.advance(target, response, dt);
    }
    spring.time = now;
    spring.frame = frame;
    if spring.at_rest(target) {
        spring.value = target;
        spring.velocity = 0.0;
    } else {
        ctx.request_repaint();
    }
    ctx.data_mut(|data| data.insert_temp(id, spring));
    spring.value
}

/// A value that starts where it is told to and then follows `target`.
pub fn follow(ctx: &Context, id: Id, on: bool, response: f32) -> f32 {
    let target = f32::from(u8::from(on));
    drive(ctx, id, target, target, response, false)
}

/// Like [`follow`], but starts from nothing, for something that arrives.
pub fn appear(ctx: &Context, id: Id, on: bool, response: f32) -> f32 {
    drive(ctx, id, f32::from(u8::from(on)), 0.0, response, false)
}

/// Comes up from nothing each time something starts being drawn again.
pub fn arrive(ctx: &Context, id: Id, response: f32) -> f32 {
    drive(ctx, id, 1.0, 0.0, response, true)
}

/// Goes down to nothing from fully shown, each time something starts leaving.
pub fn leave(ctx: &Context, id: Id, response: f32) -> f32 {
    drive(ctx, id, 0.0, 1.0, response, true)
}

/// Travels to `target` from wherever it has got to, for a position rather than an on/off state.
pub fn glide(ctx: &Context, id: Id, target: f32, response: f32) -> f32 {
    drive(ctx, id, target, target, response, false)
}

/// Goes down fast enough to land on the frame of the press, and comes back slowly enough to be seen.
pub fn press(ctx: &Context, id: Id, down: bool) -> f32 {
    follow(ctx, id, down, if down { PRESS_DOWN } else { PRESS_UP })
}

/// How much smaller than its place a tile is on the first frame of arriving and the last of leaving.
pub const GROW_FROM: f32 = 0.03;

#[derive(Clone, Copy)]
struct Flow {
    place: Id,
    target: Rect,
    frame: u64,
}

/// A rect at `target` plus an offset that decays to nothing, on four independent edges. Only a change
/// of `place` adds to the offset, which keeps the rect where it is on screen and lets it travel from
/// there, at the velocity it already has. Any other change of `target`, such as a resized window or a
/// sliding sheet, moves it 1:1 with no lag behind the pointer. A new rect starts at `origin`, or
/// grows out of its own place. Offsets are held in fractions of `area`, the unit rest is judged in.
pub fn flowing_rect(
    ctx: &Context,
    id: Id,
    area: Rect,
    target: Rect,
    place: Id,
    origin: Option<Rect>,
) -> Rect {
    if area.width() <= 0.0 || area.height() <= 0.0 {
        return target;
    }
    let flow_id = id.with("flow");
    let frame = ctx.cumulative_frame_nr();
    let stored = ctx.data(|data| data.get_temp::<Flow>(flow_id));
    ctx.data_mut(|data| {
        data.insert_temp(
            flow_id,
            Flow {
                place,
                target,
                frame,
            },
        );
    });
    // Frames count only while the window draws, so a gap means this rect was not on screen.
    let moved_from = stored
        .filter(|flow| frame <= flow.frame + 1 && flow.place != place)
        .map(|flow| flow.target);
    let start = origin.unwrap_or_else(|| target.shrink2(target.size() * GROW_FROM));
    let edge = |salt: &str, to: f32, from: f32, was: Option<f32>, span: f32| {
        let spring = id.with(salt);
        if let Some(was) = was {
            shift(ctx, spring, (was - to) / span);
        }
        to + drive(ctx, spring, 0.0, (from - to) / span, REFLOW, true) * span
    };
    let (w, h) = (area.width(), area.height());
    Rect::from_min_max(
        egui::pos2(
            edge(
                "left",
                target.min.x,
                start.min.x,
                moved_from.map(|r| r.min.x),
                w,
            ),
            edge(
                "top",
                target.min.y,
                start.min.y,
                moved_from.map(|r| r.min.y),
                h,
            ),
        ),
        egui::pos2(
            edge(
                "right",
                target.max.x,
                start.max.x,
                moved_from.map(|r| r.max.x),
                w,
            ),
            edge(
                "bottom",
                target.max.y,
                start.max.y,
                moved_from.map(|r| r.max.y),
                h,
            ),
        ),
    )
}

fn shift(ctx: &Context, id: Id, by: f32) {
    ctx.data_mut(|data| {
        if let Some(spring) = data.get_temp::<Spring>(id) {
            data.insert_temp(
                id,
                Spring {
                    value: spring.value + by,
                    ..spring
                },
            );
        }
    });
}

/// Draws `add`, growing its height and fading it in the first time it appears, so what is below
/// makes room instead of jumping. Once arrived it costs nothing and adds no frame of lag.
pub fn reveal<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let shown = arrive(ui.ctx(), ui.id().with(("reveal", salt)), SURFACE);
    grow(ui, salt, shown, true, add)
}

/// Like [`reveal`], for something that opens and closes: its height and opacity follow `open` from
/// wherever they have got to. Draws nothing once fully shut.
pub fn unfold<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    open: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let shown = follow(ui.ctx(), ui.id().with(("reveal", salt)), open, SURFACE);
    (shown > 0.0).then(|| grow(ui, salt, shown, open, add))
}

fn grow<R>(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    shown: f32,
    remeasure: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let measured_id = ui.id().with(("reveal", salt)).with("height");
    let measured = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(measured_id))
        .unwrap_or(0.0);
    let width = ui.available_width();
    let origin = ui.cursor().min;
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("reveal", salt))
            .max_rect(Rect::from_min_size(origin, vec2(width, f32::INFINITY)))
            .layout(*ui.layout()),
    );
    if shown < 1.0 {
        let window = Rect::from_min_size(origin, vec2(width, measured * shown));
        child.set_clip_rect(child.clip_rect().intersect(window));
        child.set_opacity(shown);
    }
    let output = add(&mut child);
    let content = child.min_rect().height();
    // While shutting, what it holds may already be gone; the height it had is what has to ease away.
    let height = if remeasure { content } else { measured };
    ui.ctx()
        .data_mut(|data| data.insert_temp(measured_id, height));
    ui.advance_cursor_after_rect(Rect::from_min_size(origin, vec2(width, height * shown)));
    output
}

/// Lifts a covering wash off a screen that just replaced another, so the change is a dissolve
/// rather than a cut. Paints over the video too, and takes no input.
pub fn dissolve_in(ctx: &Context, name: &str, ground: egui::Color32) {
    let id = Id::new(("screen-dissolve", name));
    let shown = arrive(ctx, id, SURFACE);
    if shown < 1.0 {
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, id))
            .rect_filled(ctx.content_rect(), 0.0, ground.gamma_multiply(1.0 - shown));
    }
}

/// How far a rect should sit below its resting place while it is `shown` that much.
pub fn rise(shown: f32, distance: f32) -> Vec2 {
    vec2(0.0, (1.0 - shown) * distance)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 60.0;

    fn resting_at(value: f32) -> Spring {
        Spring {
            value,
            velocity: 0.0,
            time: 0.0,
            frame: 0,
        }
    }

    #[test]
    fn it_settles_on_the_target_within_about_its_response_without_overshooting() {
        let mut spring = resting_at(0.0);
        let mut steps = 0;
        while !spring.at_rest(1.0) {
            spring.advance(1.0, SURFACE, FRAME);
            assert!(spring.value <= 1.0, "overshot to {}", spring.value);
            steps += 1;
            assert!(steps < 120, "never settled");
        }
        assert!(steps as f32 * FRAME < SURFACE * 2.5, "took {steps} frames");
    }

    #[test]
    fn reversing_mid_flight_keeps_both_position_and_velocity_continuous() {
        let mut spring = resting_at(0.0);
        for _ in 0..6 {
            spring.advance(1.0, SURFACE, FRAME);
        }
        let (position, velocity) = (spring.value, spring.velocity);
        assert!(position > 0.0 && velocity > 0.0);
        // Now aimed back at the start. A tiny step shows what the reversal does at that instant.
        spring.advance(0.0, SURFACE, 0.0001);
        assert!((spring.value - position).abs() < 0.001, "position jumped");
        assert!(
            (spring.velocity - velocity).abs() < velocity * 0.2,
            "velocity hit a wall: {velocity} became {}",
            spring.velocity
        );
        // It keeps travelling forward for a moment before turning, like anything with momentum.
        spring.advance(0.0, SURFACE, FRAME);
        assert!(spring.value > position);
        for _ in 0..60 {
            spring.advance(0.0, SURFACE, FRAME);
        }
        assert!(spring.value.abs() < 0.001);
    }

    #[test]
    fn a_long_stall_cannot_blow_the_spring_up() {
        let mut spring = resting_at(0.0);
        spring.advance(1.0, PRESS_DOWN, 10.0);
        assert!(spring.value.is_finite() && (spring.value - 1.0).abs() < 0.001);
    }

    fn pass(ctx: &Context, time: f64, mut read: impl FnMut(&Context) -> f32) -> (f32, bool) {
        let mut value = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| value = read(ui.ctx()),
        );
        output.textures_delta.clear();
        let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        (value, delay.is_zero())
    }

    #[test]
    fn a_settled_value_stops_asking_for_frames() {
        let ctx = Context::default();
        let id = Id::new("settling");
        let mut wants_frame = true;
        let mut value = 0.0;
        for frame in 0..120 {
            (value, wants_frame) = pass(&ctx, f64::from(frame) / 60.0, |ctx| {
                appear(ctx, id, true, TRAVEL)
            });
        }
        assert_eq!(value, 1.0);
        assert!(!wants_frame, "an idle window must be allowed to sleep");
    }

    #[test]
    fn leaving_rest_after_a_long_idle_starts_gently_instead_of_jumping() {
        let ctx = Context::default();
        let id = Id::new("idle");
        pass(&ctx, 0.0, |ctx| follow(ctx, id, false, SURFACE));
        let (first, _) = pass(&ctx, 600.0, |ctx| follow(ctx, id, true, SURFACE));
        assert!(first > 0.0 && first < 0.2, "first frame reached {first}");
    }

    fn flowing(ctx: &Context, time: f64, target: Rect, place: &str) -> (Rect, bool) {
        let area = Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1000.0, 800.0));
        let mut rect = target;
        let (_, moving) = pass(ctx, time, |ctx| {
            rect = flowing_rect(ctx, Id::new("tile"), area, target, Id::new(place), None);
            0.0
        });
        (rect, moving)
    }

    fn at(left: f32, width: f32) -> Rect {
        Rect::from_min_size(egui::pos2(left, 0.0), vec2(width, width * 0.5))
    }

    fn settled_at(ctx: &Context, target: Rect, place: &str) -> f64 {
        for frame in 0..120 {
            flowing(ctx, f64::from(frame) / 60.0, target, place);
        }
        2.0
    }

    #[test]
    fn a_flowing_rect_grows_into_a_new_place_and_then_rests_exactly_on_it() {
        let ctx = Context::default();
        let target = at(100.0, 400.0);
        let (first, _) = flowing(&ctx, 0.0, target, "a");
        assert!(first.width() < target.width() && first.width() > target.width() * 0.9);
        let time = settled_at(&ctx, target, "a");
        assert_eq!(flowing(&ctx, time, target, "a"), (target, false));
    }

    #[test]
    fn a_target_that_moves_within_its_place_is_followed_on_the_same_frame() {
        let ctx = Context::default();
        let time = settled_at(&ctx, at(100.0, 400.0), "a");
        // A tile keeping its picture's shape changes fractions of the area as the window resizes,
        // which is why position cannot simply be sprung in fractions.
        let resized = at(180.0, 520.0);
        assert_eq!(flowing(&ctx, time, resized, "a"), (resized, false));
    }

    #[test]
    fn a_new_place_starts_from_where_the_rect_is_on_screen_and_travels_there() {
        let ctx = Context::default();
        let old = at(100.0, 400.0);
        let time = settled_at(&ctx, old, "a");
        let new = at(600.0, 200.0);
        let (first, moving) = flowing(&ctx, time, new, "b");
        assert!(moving);
        assert!(
            (first.left() - old.left()).abs() < 60.0 && (first.width() - old.width()).abs() < 60.0,
            "jumped to {first:?}"
        );
        let mut last = first;
        for frame in 1..120 {
            (last, _) = flowing(&ctx, time + f64::from(frame) / 60.0, new, "b");
        }
        assert_eq!(last, new);
    }

    #[test]
    fn a_second_change_of_place_mid_flight_keeps_position_and_velocity() {
        let ctx = Context::default();
        let time = settled_at(&ctx, at(0.0, 200.0), "a");
        let mut rect = at(0.0, 200.0);
        let mut previous = rect;
        for frame in 0..6 {
            previous = rect;
            (rect, _) = flowing(&ctx, time + f64::from(frame) / 60.0, at(800.0, 200.0), "b");
        }
        let step = rect.left() - previous.left();
        assert!(step > 0.0);
        // Heading back the other way: the very next frame must still be carried forward by the
        // velocity it had, rather than starting back from a standstill.
        let (next, _) = flowing(&ctx, time + 6.0 / 60.0, at(0.0, 200.0), "c");
        assert!(
            next.left() > rect.left() + step * 0.25,
            "moved {} after a step of {step}",
            next.left() - rect.left()
        );
    }

    #[test]
    fn leaving_starts_fully_shown_and_goes_all_the_way_to_nothing() {
        let ctx = Context::default();
        let id = Id::new("exit");
        let (first, wants_frame) = pass(&ctx, 0.0, |ctx| leave(ctx, id, SURFACE));
        assert!(first > 0.9 && wants_frame, "started at {first}");
        let mut value = first;
        for frame in 1..120 {
            (value, _) = pass(&ctx, f64::from(frame) / 60.0, |ctx| leave(ctx, id, SURFACE));
        }
        assert_eq!(value, 0.0);
    }

    #[test]
    fn arriving_again_after_being_off_screen_starts_over() {
        let ctx = Context::default();
        let id = Id::new("screen");
        let mut value = 0.0;
        for frame in 0..60 {
            (value, _) = pass(&ctx, f64::from(frame) / 60.0, |ctx| {
                arrive(ctx, id, SURFACE)
            });
        }
        assert_eq!(value, 1.0);
        pass(&ctx, 1.0, |_| 0.0);
        pass(&ctx, 1.1, |_| 0.0);
        let (again, _) = pass(&ctx, 1.2, |ctx| arrive(ctx, id, SURFACE));
        assert!(again < 0.2, "started at {again}");
    }
}
