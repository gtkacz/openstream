// Light drawn in characters. `fs_cells` draws one texel per character cell and decides everything a
// cell shows. `fs_halo` spreads that light into its neighbours. `fs_show` runs across the window and
// only looks those answers up, so the costly part scales with the few thousand cells a window has
// rather than with its pixels.

struct Scene {
    window: vec2<f32>,
    cell: vec2<f32>,
    centre: vec2<f32>,
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
    ground: vec4<f32>,
    dim: vec4<f32>,
    phosphor: vec4<f32>,
    hot: vec4<f32>,
    spark: vec4<f32>,
    stirs: array<vec4<f32>, 16>,
    rings: array<vec4<f32>, 4>,
    quiet: array<vec4<f32>, 3>,
    push: array<vec4<f32>, 24>,
    push_life: array<vec4<f32>, 6>,
};

struct Vertex {
    @builtin(position) position: vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var cells: texture_2d<f32>;
@group(0) @binding(2) var halo: texture_2d<f32>;
@group(0) @binding(3) var glyph_atlas: texture_2d<f32>;
@group(0) @binding(4) var glyph_sampler: sampler;
@group(0) @binding(5) var soft: sampler;

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> Vertex {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Vertex(vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0));
}

// PCG's output function: integer noise, so a cell keeps its numbers at any window size.
fn scramble(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn chance(place: vec2<u32>, salt: u32) -> f32 {
    return f32(scramble(place.x ^ scramble(place.y ^ scramble(salt)))) / 4294967295.0;
}

// Three sines, each bending the next, which wander without ever looking like a pattern.
fn drift(at: vec2<f32>) -> f32 {
    let t = scene.drift_clock;
    let a = sin(at.x * 3.1 + t * 0.21);
    let b = sin(at.y * 4.3 - t * 0.17 + a * 0.8);
    let c = sin((at.x + at.y) * 2.3 + t * 0.13 + b * 0.6);
    return (a + b + c) / 3.0;
}

struct Light {
    amount: f32,
    // How late in the opening this place resolves, from 0 to 1.
    order: f32,
};

// Offset so that negative coordinates hash like positive ones.
fn lattice(at: vec2<f32>) -> f32 {
    return chance(vec2<u32>(vec2<i32>(floor(at)) + vec2<i32>(4096)), 7u);
}

fn noise(at: vec2<f32>) -> f32 {
    let i = floor(at);
    let f = fract(at);
    let u = f * f * (3.0 - 2.0 * f);
    let low = mix(lattice(i), lattice(i + vec2<f32>(1.0, 0.0)), u.x);
    let high = mix(lattice(i + vec2<f32>(0.0, 1.0)), lattice(i + vec2<f32>(1.0, 1.0)), u.x);
    return mix(low, high, u.y);
}

fn fbm(start: vec2<f32>) -> f32 {
    var at = start;
    var sum = 0.0;
    var amplitude = 0.5;
    for (var n = 0; n < 4; n++) {
        sum += amplitude * noise(at);
        at = at * 2.03 + vec2<f32>(1.7, 9.2);
        amplitude *= 0.5;
    }
    return sum / 0.9375;
}

// One noise field warped by two others, so the light gathers in folds that drift and change shape
// rather than sliding past as a fixed pattern.
fn folds(at: vec2<f32>) -> f32 {
    let t = scene.drift_clock;
    let p = at * 1.5;
    let q = vec2<f32>(
        fbm(p + vec2<f32>(0.0, t * 0.07)),
        fbm(p + vec2<f32>(5.2, 1.3) + vec2<f32>(t * 0.06, 0.0)),
    );
    let r = vec2<f32>(
        fbm(p + 2.0 * q + vec2<f32>(1.7, 9.2) - vec2<f32>(t * 0.04)),
        fbm(p + 2.0 * q + vec2<f32>(8.3, 2.8) + vec2<f32>(t * 0.035)),
    );
    return fbm(p + 2.2 * r);
}

// 1 inside a region the interface sits on, easing to 0 over the reach, so the field thins out
// towards text instead of stopping at its edge.
fn quiet(at: vec2<f32>) -> f32 {
    var most = 0.0;
    for (var n = 0; n < 3; n++) {
        let region = scene.quiet[n];
        let outside = max(max(region.xy - at, at - region.zw), vec2<f32>(0.0));
        most = max(most, 1.0 - smoothstep(0.0, scene.quiet_reach, length(outside)));
    }
    return most * scene.quiet_strength;
}

// How far, in window heights squared, a push sample's reach spreads.
const PUSH_NEAR: f32 = 0.0045;
const PUSH_SCALE: f32 = 0.012;
// The longest a push can shift the field, so a fast flick bends it rather than tearing it.
const PUSH_SOFTEN: f32 = 0.06;
const PUSH_LIGHT: f32 = 0.22;
const PUSH_STIR: f32 = 0.35;
// The speed, in window heights a second, at which a push counts as fully fast.
const PUSH_FAST: f32 = 1.5;

struct Wake {
    shift: vec2<f32>,
    strength: f32,
};

// The recent pointer samples near this place, as how far they push the field and how strongly.
fn push_at(at: vec2<f32>) -> Wake {
    var sum = vec2<f32>(0.0);
    var strongest = 0.0;
    for (var n = 0; n < 24; n++) {
        let life = scene.push_life[n / 4][n % 4];
        if (life <= 0.0) {
            continue;
        }
        let recent = scene.push[n];
        let off = at - recent.xy;
        let near = exp(-dot(off, off) / PUSH_NEAR);
        sum += recent.zw * life * near;
        strongest = max(strongest, life * near * min(length(recent.zw), PUSH_FAST) / PUSH_FAST);
    }
    let shift = sum * PUSH_SCALE;
    return Wake(shift / (1.0 + length(shift) / PUSH_SOFTEN), strongest);
}

// The floor leaves sparse faint dots in the dark between folds. The push's light is added before
// the quiet, so the pointer cannot light the ground under text. Bright places resolve first.
fn field(at: vec2<f32>, wake: Wake) -> Light {
    let lit = 0.06 + 0.94 * smoothstep(0.44, 0.8, folds(at - wake.shift))
        + PUSH_LIGHT * wake.strength;
    let amount = lit * scene.gain * (1.0 - quiet(at));
    return Light(amount, 1.0 - 0.8 * clamp(amount, 0.0, 1.0));
}

// Falls away from a point, the way the delay it sits behind does.
fn scope(at: vec2<f32>) -> Light {
    let away = at - scene.centre;
    let distance = length(away) / scene.reach;
    let wander = 1.0 + drift(away / scene.reach * 1.6) * 0.22;
    let fall = 1.0 - smoothstep(0.0, 1.25, distance * wander);
    var amount = fall * fall * scene.gain;
    if (scene.sweep >= 0.0) {
        let behind_hand = fract((scene.sweep - atan2(away.y, away.x)) / 6.2831853);
        amount += exp(-behind_hand * 22.0) * 0.28 * (1.0 - smoothstep(0.95, 1.12, distance));
    }
    return Light(amount, clamp(distance * 0.7, 0.0, 1.0));
}

// Red is how much light the cell has, green how much ink its character gets, blue which character it
// is, and alpha whether it sits out of focus.
@fragment
fn fs_cells(vertex: Vertex) -> @location(0) vec4<f32> {
    let cell = floor(vertex.position.xy);
    let place = vec2<u32>(cell);
    let at = (cell + 0.5) * scene.cell / scene.window.y;
    var light: Light;
    var wake = Wake(vec2<f32>(0.0), 0.0);
    if (scene.kind < 0.5) {
        wake = push_at(at);
        light = field(at, wake);
    } else {
        light = scope(at);
    }

    let own = chance(place, 1u);
    let beat = u32(floor(scene.churn_clock * 9.0 + own * 9.0));
    let wobble = (chance(place, 16u + beat) - 0.5) * scene.churn;
    let shade = clamp(pow(light.amount, 0.75) + wobble, 0.0, 1.0);
    var glyph = round(shade * (scene.glyphs - 1.0));

    // A cell resolves through noise rather than fading in, and whatever stirs it goes back to noise.
    let due = light.order * 0.7 + own * 0.3;
    let resolved = clamp((scene.opening * 1.4 - due) / 0.4, 0.0, 1.0);
    var stirred = max(1.0 - resolved, PUSH_STIR * wake.strength);
    for (var n = 0; n < 16; n++) {
        let stir = scene.stirs[n];
        if (stir.z > 0.0) {
            let off = at - stir.xy;
            let spread = 1.0 + dot(off, off) / 0.012;
            stirred = max(stirred, stir.z / (spread * spread));
        }
    }
    for (var n = 0; n < 4; n++) {
        let ring = scene.rings[n];
        if (ring.w > 0.0) {
            let off = abs(length(at - ring.xy) - ring.z);
            stirred = max(stirred, ring.w * (1.0 - smoothstep(0.0, 0.07, off)));
        }
    }
    let flip = u32(floor(scene.churn_clock * 20.0));
    if (chance(place, 64u + flip) < stirred * 1.25) {
        glyph = 1.0 + floor(chance(place, 4096u + flip) * (scene.glyphs - 1.0));
    }

    let kept = step(chance(place, 2u), 0.86 * mix(1.0, 0.3 + shade * 1.1, 0.55));
    let bright_enough = smoothstep(0.03, 0.1, light.amount);
    let level = mix(1.0, 0.35 + 0.65 * own, 0.55);
    let out_of_focus = step(chance(place, 3u), 0.15);
    let arrived = smoothstep(0.0, 0.25, resolved);
    let ink = kept * bright_enough * level * arrived * (1.0 - out_of_focus * 0.5);
    return vec4<f32>(clamp(light.amount, 0.0, 1.0), ink, glyph / 255.0, out_of_focus);
}

// What a character gives off, spread over the cells around it, nearer ones most.
@fragment
fn fs_halo(vertex: Vertex) -> @location(0) vec4<f32> {
    let cell = vec2<i32>(floor(vertex.position.xy));
    let last = vec2<i32>(textureDimensions(cells)) - vec2<i32>(1);
    var sum = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let weight = f32((3 - abs(x)) * (3 - abs(y))) / 81.0;
            let there = textureLoad(cells, clamp(cell + vec2<i32>(x, y), vec2<i32>(0), last), 0);
            sum += there.r * there.g * weight;
        }
    }
    return vec4<f32>(sum, 0.0, 0.0, 1.0);
}

// Dim where there is little light, the phosphor's own colour in the middle, hot at the top.
fn tone(amount: f32) -> vec3<f32> {
    let stepped = smoothstep(0.08, 0.95, amount) * 2.0;
    if (stepped < 1.0) {
        return mix(scene.dim.rgb, scene.phosphor.rgb, stepped);
    }
    return mix(scene.phosphor.rgb, scene.hot.rgb, stepped - 1.0);
}

// Where the signal's colour gathers: soft patches about half a window apart, wandering on the same
// clock as everything else in the light, so they hold still when motion is reduced. Driven mostly by
// the width, because a field free to wander in both directions can leave the bright part empty.
fn signal_patch(across: vec2<f32>) -> f32 {
    let t = scene.drift_clock;
    let x = across.x * scene.window.x / scene.window.y;
    let bend = sin(x * 1.9 - t * 0.09) * 1.1 + sin(across.y * 3.0 + t * 0.11) * 0.5;
    return smoothstep(0.35, 0.85, sin(x * 7.0 + t * 0.13 + bend));
}

// How much of the signal's colour this much light carries. Faint light stays blue, whatever patch it
// is in, so the signal falls through the blues to the floor like the rest of the light.
fn carried(amount: f32, gathered: f32) -> f32 {
    return gathered * scene.spark.a * smoothstep(0.2, 0.55, amount);
}

@fragment
fn fs_show(vertex: Vertex) -> @location(0) vec4<f32> {
    let spot = vertex.position.xy / scene.cell;
    let cell = vec2<i32>(floor(spot));
    let inner = fract(spot);
    let last = vec2<i32>(textureDimensions(cells)) - vec2<i32>(1);
    let answer = textureLoad(cells, clamp(cell, vec2<i32>(0), last), 0);
    let shape = vec2<f32>((round(answer.b * 255.0) + inner.x) / scene.glyphs, inner.y);
    let coverage = textureSampleLevel(glyph_atlas, glyph_sampler, shape, scene.lod + answer.a * 1.2).r;
    let drawn = coverage * answer.g;

    let across = vertex.position.xy / scene.window;
    let wash = textureSampleLevel(cells, soft, across, 0.0).r;
    let around = textureSampleLevel(halo, soft, across, 0.0).r;

    var color = scene.ground.rgb;
    let gathered = signal_patch(across);
    // Deep where the light is faint, and the phosphor's own colour only where it is strong.
    let glow = mix(mix(scene.dim.rgb, scene.phosphor.rgb, wash), scene.spark.rgb, carried(wash, gathered));
    color += glow * wash * wash * scene.wash;
    color += mix(scene.dim.rgb, scene.spark.rgb, carried(around, gathered)) * around * scene.halo;
    let ink = mix(tone(answer.r), scene.spark.rgb, carried(answer.r, gathered));
    color = mix(color, ink * scene.ink, drawn);

    // Stronger where there is light, the way a tube's own noise is, and it hides the banding of such
    // a dark gradient.
    let pixel = vec2<u32>(vertex.position.xy);
    let noise = chance(pixel, u32(scene.grain_seed)) + chance(pixel, u32(scene.grain_seed) + 977u) - 1.0;
    color += vec3<f32>(noise * scene.grain * (0.2 + wash + drawn));

    return vec4<f32>(mix(scene.ground.rgb, color, scene.shown), 1.0);
}
