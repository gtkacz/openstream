//! The light behind the start screen and the room map, drawn in characters: it resolves out of noise
//! when it opens, churns while it runs, and goes back to noise wherever the pointer stirs it. It runs
//! only while one of those screens is on show, and holds still for an unfocused window or for someone
//! who asked their system for less motion.

use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use egui::{Color32, Id, Pos2, Vec2};

use super::glyphs;
use super::passes::{self, Target, rgba};
use crate::ui::theme;

const SHADER: &str = include_str!("backdrop.wgsl");
const CELLS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const HALO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// The height of a character cell. Three times the interface's grid, so it reads as a texture.
const CELL_POINTS: f32 = 18.0;
const OPENING_SECONDS: f32 = 2.4;
const SWEEP_SECONDS: f32 = 1.7;
const STIR_SECONDS: f32 = 0.7;
const RING_SECONDS: f32 = 1.2;
const STIR_SPACING: f32 = 0.02;
const RING_SPEED: f32 = 0.75;
const GRAIN_PER_SECOND: f32 = 24.0;
const MAX_STIRS: usize = 16;
const MAX_RINGS: usize = 4;
/// How far past a quiet region the field takes to come back to full strength, so it thins out
/// towards text instead of stopping at a hard edge.
const QUIET_REACH_POINTS: f32 = 80.0;
/// Full, so text only ever sits on the ground and keeps its 4.5:1 whatever fold passes beneath.
const QUIET_STRENGTH: f32 = 1.0;
/// In window heights: where a region that should quiet nothing is put, too far to reach anything.
const FAR_AWAY: f32 = 1.0e4;
/// In window heights of pointer travel between push samples: close enough that a quick sweep
/// leaves a continuous wake.
const PUSH_SPACING: f32 = 0.008;
/// Enough samples to cover about a second of sweeping at the spacing above.
const MAX_PUSH: usize = 24;
/// How long a sample keeps pushing, so the field settles soon after the pointer stops.
const PUSH_SECONDS: f32 = 1.2;
/// How much of the previous velocity survives each frame, smoothing out jittery pointer events.
const PUSH_SMOOTHING: f32 = 0.6;
/// Below a 240 Hz frame, a measured velocity is noise rather than motion.
const SHORTEST_PUSH_STEP: f32 = 1.0 / 240.0;

/// How often a running backdrop asks for a frame. It steps rather than glides, so this is plenty.
pub const FRAME: Duration = Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackdropKind {
    /// Folds of light across the whole window, going quiet around `quiet`, in points: the brand,
    /// the form and the window's buttons. A region that is not positive quiets nothing.
    Field { quiet: [egui::Rect; 3] },
    /// A light around a point, in points, reaching `radius` points.
    Scope { center: Pos2, radius: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropRequest {
    pub kind: BackdropKind,
    /// For fading with the screen it belongs to.
    pub shown: f32,
    /// How much of the signal's colour the light carries, from 0 to 1.
    pub signal: f32,
}

const REQUEST: &str = "backdrop-request";

/// Called during the egui pass by whichever screen wants the light behind it.
pub fn request(ctx: &egui::Context, request: BackdropRequest) {
    ctx.data_mut(|data| data.insert_temp(Id::new(REQUEST), request));
}

/// Called after the egui pass. Taking it means a screen that stops asking turns the light off.
pub fn take_request(ctx: &egui::Context) -> Option<BackdropRequest> {
    ctx.data_mut(|data| {
        let found = data.get_temp::<BackdropRequest>(Id::new(REQUEST));
        data.remove::<BackdropRequest>(Id::new(REQUEST));
        found
    })
}

/// What the pointer did this frame, in window pixels.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pointer {
    pub at: Option<Pos2>,
    pub down: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Scene {
    window: [f32; 2],
    cell: [f32; 2],
    centre: [f32; 2],
    reach: f32,
    kind: f32,
    drift_clock: f32,
    churn_clock: f32,
    opening: f32,
    sweep: f32,
    gain: f32,
    lod: f32,
    grain: f32,
    grain_seed: f32,
    ink: f32,
    halo: f32,
    wash: f32,
    shown: f32,
    glyphs: f32,
    churn: f32,
    quiet_reach: f32,
    quiet_strength: f32,
    ground: [f32; 4],
    dim: [f32; 4],
    phosphor: [f32; 4],
    hot: [f32; 4],
    spark: [f32; 4],
    stirs: [[f32; 4]; MAX_STIRS],
    rings: [[f32; 4]; MAX_RINGS],
    quiet: [[f32; 4]; 3],
    push: [[f32; 4]; MAX_PUSH],
    push_life: [[f32; 4]; MAX_PUSH / 4],
}

struct PushSample {
    at: Pos2,
    velocity: Vec2,
    age: f32,
}

/// Where the pointer has been and how fast it was going, in window heights, newest first. The
/// field is sampled behind each recent sample, so the pointer pushes it along its path.
#[derive(Default)]
struct Push {
    samples: Vec<PushSample>,
    /// The last sample's position and the seconds since, which the velocity is measured from.
    anchor: Option<(Pos2, f32)>,
    velocity: Vec2,
}

impl Push {
    fn step(&mut self, dt: f32, here: Option<Pos2>) {
        for sample in &mut self.samples {
            sample.age += dt;
        }
        self.samples.retain(|sample| sample.age < PUSH_SECONDS);
        let Some(here) = here else {
            // Coming back in elsewhere is not a flick across the window.
            self.anchor = None;
            return;
        };
        let Some((from, since)) = self.anchor else {
            self.anchor = Some((here, 0.0));
            self.velocity = Vec2::ZERO;
            return;
        };
        let since = since + dt;
        let measured = (here - from) / since.max(SHORTEST_PUSH_STEP);
        self.velocity = self.velocity * PUSH_SMOOTHING + measured * (1.0 - PUSH_SMOOTHING);
        if from.distance(here) >= PUSH_SPACING {
            self.samples.insert(
                0,
                PushSample {
                    at: here,
                    velocity: self.velocity,
                    age: 0.0,
                },
            );
            self.samples.truncate(MAX_PUSH);
            self.anchor = Some((here, 0.0));
        } else {
            self.anchor = Some((from, since));
        }
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    /// Each sample's place and velocity, and their lives four to a `vec4`, because a uniform
    /// array's elements are 16 bytes apart.
    fn uniforms(&self) -> ([[f32; 4]; MAX_PUSH], [[f32; 4]; MAX_PUSH / 4]) {
        let mut samples = [[0.0; 4]; MAX_PUSH];
        let mut lives = [[0.0; 4]; MAX_PUSH / 4];
        for (index, sample) in self.samples.iter().enumerate() {
            samples[index] = [
                sample.at.x,
                sample.at.y,
                sample.velocity.x,
                sample.velocity.y,
            ];
            lives[index / 4][index % 4] = 1.0 - sample.age / PUSH_SECONDS;
        }
        (samples, lives)
    }
}

/// Each region as its edges in window heights, the unit the shader measures in.
fn quiet_in_heights(
    quiet: [egui::Rect; 3],
    pixels_per_point: f32,
    window_height: f32,
) -> [[f32; 4]; 3] {
    quiet.map(|region| {
        if region.is_positive() {
            [region.min.x, region.min.y, region.max.x, region.max.y]
                .map(|edge| edge * pixels_per_point / window_height)
        } else {
            [FAR_AWAY; 4]
        }
    })
}

/// How each kind of light is mixed. The horizon sits behind a form, so it stays low and warm; the
/// scope is the picture on the room map, so it may run hot.
struct Look {
    gain: f32,
    ink: f32,
    halo: f32,
    wash: f32,
    ground: Color32,
    dim: Color32,
    hot: Color32,
}

fn look(kind: BackdropKind) -> Look {
    match kind {
        BackdropKind::Field { .. } => Look {
            gain: 0.74,
            ink: 0.85,
            halo: 0.22,
            wash: 0.24,
            ground: theme::ground(),
            dim: theme::wire(),
            hot: theme::text(),
        },
        BackdropKind::Scope { .. } => Look {
            gain: 0.62,
            ink: 1.0,
            halo: 0.34,
            wash: 0.16,
            ground: Color32::BLACK,
            dim: theme::wire(),
            hot: theme::text(),
        },
    }
}

/// One texel per character cell, and the view of it `fs_show` reads.
struct Sized {
    grid: (u32, u32),
    cells: Target,
    halo: Target,
    cells_group: wgpu::BindGroup,
    halo_group: wgpu::BindGroup,
    show_group: wgpu::BindGroup,
}

pub struct Backdrop {
    uniforms: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    atlas: wgpu::TextureView,
    atlas_sampler: wgpu::Sampler,
    soft: wgpu::Sampler,
    blank: Target,
    cells_pipeline: wgpu::RenderPipeline,
    halo_pipeline: wgpu::RenderPipeline,
    show_pipeline: wgpu::RenderPipeline,
    sized: Option<Sized>,
    last_frame: Instant,
    opened_kind: Option<std::mem::Discriminant<BackdropKind>>,
    opened_at: Instant,
    drift_clock: f32,
    churn_clock: f32,
    opening: f32,
    /// Where the pointer has been, newest first, in units of the window's height, with their ages.
    stirs: Vec<(Pos2, f32)>,
    rings: Vec<(Pos2, f32)>,
    was_down: bool,
    push: Push,
}

impl Backdrop {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop-layout"),
            entries: &[
                passes::uniform_entry(0),
                passes::texture_entry(1),
                passes::texture_entry(2),
                passes::texture_entry(3),
                passes::sampler_entry(4),
                passes::sampler_entry(5),
            ],
        });
        let now = Instant::now();
        Self {
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("backdrop-scene"),
                size: size_of::<Scene>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            atlas: glyphs::atlas(device, queue),
            atlas_sampler: glyphs::sampler(device),
            soft: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("backdrop-soft"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            blank: passes::target(device, HALO_FORMAT, 1, 1, "backdrop-blank"),
            cells_pipeline: passes::pipeline(
                device,
                &shader,
                &layout,
                "fs_cells",
                CELLS_FORMAT,
                None,
            ),
            halo_pipeline: passes::pipeline(device, &shader, &layout, "fs_halo", HALO_FORMAT, None),
            show_pipeline: passes::pipeline(device, &shader, &layout, "fs_show", format, None),
            layout,
            sized: None,
            last_frame: now,
            opened_kind: None,
            opened_at: now,
            drift_clock: 0.0,
            churn_clock: 0.0,
            opening: 0.0,
            stirs: Vec::new(),
            rings: Vec::new(),
            was_down: false,
            push: Push::default(),
        }
    }

    fn group(
        &self,
        device: &wgpu::Device,
        cells: &wgpu::TextureView,
        halo: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("backdrop-group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(cells),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(halo),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.soft),
                },
            ],
        })
    }

    fn fit(&mut self, device: &wgpu::Device, grid: (u32, u32)) {
        if self.sized.as_ref().is_some_and(|sized| sized.grid == grid) {
            return;
        }
        let cells = passes::target(device, CELLS_FORMAT, grid.0, grid.1, "backdrop-cells");
        let halo = passes::target(device, HALO_FORMAT, grid.0, grid.1, "backdrop-halo");
        // A pass cannot read what it draws to, so each gets a stand-in for its own target.
        let sized = Sized {
            grid,
            cells_group: self.group(device, &self.blank.view, &self.blank.view),
            halo_group: self.group(device, &cells.view, &self.blank.view),
            show_group: self.group(device, &cells.view, &halo.view),
            cells,
            halo,
        };
        self.sized = Some(sized);
    }

    fn step_pointer(&mut self, dt: f32, window: (f32, f32), pointer: Pointer) {
        for stir in &mut self.stirs {
            stir.1 += dt;
        }
        self.stirs.retain(|stir| stir.1 < STIR_SECONDS);
        for ring in &mut self.rings {
            ring.1 += dt;
        }
        self.rings.retain(|ring| ring.1 < RING_SECONDS);
        self.push.step(
            dt,
            pointer
                .at
                .map(|at| Pos2::new(at.x / window.1, at.y / window.1)),
        );
        if let Some(at) = pointer.at {
            let here = Pos2::new(at.x / window.1, at.y / window.1);
            let moved_far_enough = self
                .stirs
                .first()
                .is_none_or(|last| last.0.distance(here) >= STIR_SPACING);
            if moved_far_enough {
                self.stirs.insert(0, (here, 0.0));
                self.stirs.truncate(MAX_STIRS);
            }
            if pointer.down && !self.was_down {
                self.rings.insert(0, (here, 0.0));
                self.rings.truncate(MAX_RINGS);
            }
        }
        self.was_down = pointer.down;
    }

    /// Moves the light on and draws its small passes. `moving` false holds it where it is.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
        pixels_per_point: f32,
        request: BackdropRequest,
        pointer: Pointer,
        moving: bool,
    ) {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_frame)
            .as_secs_f32()
            .clamp(0.0, 0.1);
        self.last_frame = now;
        let window = (size.0.max(1) as f32, size.1.max(1) as f32);

        let kind = std::mem::discriminant(&request.kind);
        if self.opened_kind != Some(kind) {
            self.opened_kind = Some(kind);
            self.opened_at = now;
            self.opening = 0.0;
        }
        if moving {
            self.drift_clock += dt * 0.5;
            self.churn_clock += dt;
            self.opening = (self.opening + dt / OPENING_SECONDS).min(1.0);
            self.step_pointer(dt, window, pointer);
        } else {
            self.opening = 1.0;
            self.stirs.clear();
            self.rings.clear();
            self.push.clear();
            self.was_down = pointer.down;
        }

        let cell_h = (CELL_POINTS * pixels_per_point).round().max(1.0);
        let cell_w = (cell_h * glyphs::CELL_WIDTH as f32 / glyphs::CELL_HEIGHT as f32)
            .round()
            .max(1.0);
        let grid = (
            (window.0 / cell_w).ceil() as u32,
            (window.1 / cell_h).ceil() as u32,
        );
        self.fit(device, grid);

        let mut stirs = [[0.0; 4]; MAX_STIRS];
        for (slot, (at, age)) in stirs.iter_mut().zip(&self.stirs) {
            let left = 1.0 - age / STIR_SECONDS;
            *slot = [at.x, at.y, left * left, 0.0];
        }
        let mut rings = [[0.0; 4]; MAX_RINGS];
        for (slot, (at, age)) in rings.iter_mut().zip(&self.rings) {
            let left = 1.0 - age / RING_SECONDS;
            *slot = [at.x, at.y, age * RING_SPEED, left * left];
        }

        let (centre, reach, kind_value, quiet, quiet_strength) = match request.kind {
            BackdropKind::Field { quiet } => (
                [0.0, 0.0],
                1.0,
                0.0,
                quiet_in_heights(quiet, pixels_per_point, window.1),
                QUIET_STRENGTH,
            ),
            BackdropKind::Scope { center, radius } => (
                [
                    center.x * pixels_per_point / window.1,
                    center.y * pixels_per_point / window.1,
                ],
                radius * pixels_per_point / window.1,
                1.0,
                [[FAR_AWAY; 4]; 3],
                0.0,
            ),
        };
        // One turn when the map opens, starting from straight up.
        let swept = now.duration_since(self.opened_at).as_secs_f32() / SWEEP_SECONDS;
        let sweep = if moving && kind_value > 0.5 && swept < 1.0 {
            (std::f32::consts::TAU * (0.75 + swept)) % std::f32::consts::TAU
        } else {
            -1.0
        };
        let look = look(request.kind);
        let (push, push_life) = self.push.uniforms();
        let scene = Scene {
            window: [window.0, window.1],
            cell: [cell_w, cell_h],
            centre,
            reach,
            kind: kind_value,
            drift_clock: self.drift_clock,
            churn_clock: self.churn_clock,
            opening: self.opening,
            sweep,
            gain: look.gain,
            lod: (glyphs::CELL_HEIGHT as f32 / cell_h).log2().max(0.0),
            grain: 0.03,
            grain_seed: (self.churn_clock * GRAIN_PER_SECOND).floor(),
            ink: look.ink,
            halo: look.halo,
            wash: look.wash,
            shown: request.shown,
            glyphs: glyphs::RAMP.chars().count() as f32,
            churn: if moving { 0.1 } else { 0.0 },
            quiet_reach: QUIET_REACH_POINTS * pixels_per_point / window.1,
            quiet_strength,
            ground: rgba(look.ground),
            dim: rgba(look.dim),
            phosphor: rgba(theme::accent()),
            hot: rgba(look.hot),
            spark: {
                let [red, green, blue, _] = rgba(theme::signal());
                [red, green, blue, request.signal]
            },
            stirs,
            rings,
            quiet,
            push,
            push_life,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&scene));

        let Some(sized) = &self.sized else {
            return;
        };
        {
            let mut pass = passes::begin(encoder, &sized.cells.view, "backdrop-cells");
            pass.set_pipeline(&self.cells_pipeline);
            pass.set_bind_group(0, &sized.cells_group, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = passes::begin(encoder, &sized.halo.view, "backdrop-halo");
            pass.set_pipeline(&self.halo_pipeline);
            pass.set_bind_group(0, &sized.halo_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// For stills taken far faster than real time, where the opening would never finish.
    pub fn skip_opening(&mut self) {
        self.opening = 1.0;
    }

    /// Lays the light down first thing in the window's pass, under the video and the interface.
    pub fn show(&self, pass: &mut wgpu::RenderPass<'static>) {
        if let Some(sized) = &self.sized {
            pass.set_pipeline(&self.show_pipeline);
            pass.set_bind_group(0, &sized.show_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2, pos2};

    use super::*;

    fn walk(push: &mut Push, steps: usize, dt: f32, stride: f32) {
        for n in 0..=steps {
            push.step(dt, Some(pos2(n as f32 * stride, 0.5)));
        }
    }

    #[test]
    fn push_records_a_sample_every_0_008_window_heights_of_travel() {
        let mut push = Push::default();
        // Steps of 0.005 record on every second step, well clear of 0.008's float edge.
        walk(&mut push, 10, 0.01, 0.005);
        assert_eq!(push.samples.len(), 5);
    }

    #[test]
    fn push_keeps_the_newest_24_samples_and_lets_them_age_out() {
        let mut push = Push::default();
        walk(&mut push, 40, 0.01, 0.01);
        assert_eq!(push.samples.len(), 24);
        assert!((push.samples[0].at.x - 0.40).abs() < 1e-5);
        for _ in 0..13 {
            push.step(0.1, None);
        }
        assert!(push.samples.is_empty());
    }

    #[test]
    fn push_velocity_is_window_heights_per_second_from_the_last_sample() {
        let mut push = Push::default();
        walk(&mut push, 50, 0.01, 0.01);
        assert!(
            (push.velocity.x - 1.0).abs() < 1e-3 && push.velocity.y.abs() < 1e-6,
            "{:?}",
            push.velocity
        );
    }

    #[test]
    fn a_pointer_that_leaves_and_comes_back_far_away_does_not_throw_the_field() {
        let mut push = Push::default();
        push.step(0.01, Some(pos2(0.1, 0.1)));
        push.step(0.01, None);
        push.step(0.01, Some(pos2(0.9, 0.9)));
        assert!(push.samples.is_empty() && push.velocity == Vec2::ZERO);
    }

    #[test]
    fn push_lives_pack_four_to_a_vec4_in_sample_order() {
        let mut push = Push::default();
        walk(&mut push, 2, 0.01, 0.01);
        push.step(0.6, None);
        let (samples, lives) = push.uniforms();
        assert!(
            (lives[0][0] - 0.5).abs() < 1e-5 && (lives[0][1] - (1.0 - 0.61 / 1.2)).abs() < 1e-5,
            "{lives:?}"
        );
        assert_eq!(lives[1], [0.0; 4]);
        assert!((samples[0][0] - 0.02).abs() < 1e-6);
    }

    #[test]
    fn quiet_regions_turn_points_into_window_heights_and_ignore_empty_ones() {
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
        let regions = [
            Rect::from_min_max(pos2(24.0, 36.0), pos2(170.0, 68.0)),
            Rect::NOTHING,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(50.0, 100.0)),
        ];
        let heights = quiet_in_heights(regions, 2.0, 1000.0);
        assert!(
            close(heights[0], [0.048, 0.072, 0.34, 0.136]),
            "{:?}",
            heights[0]
        );
        assert!(
            heights[1].iter().all(|edge| *edge >= 1.0e3),
            "{:?}",
            heights[1]
        );
        assert!(close(heights[2], [0.0, 0.0, 0.1, 0.2]), "{:?}", heights[2]);
    }
}
