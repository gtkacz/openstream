// A picture stays pixel exact until `ascii` rises. Then it reduces to characters cell by cell, which
// is the one time video looks analog, because that is the one time something has gone wrong.
struct Fit {
    scale: vec2<f32>,
    shown: f32,
    flash: f32,
    origin: vec2<f32>,
    size: vec2<f32>,
    ink: vec4<f32>,
    hot: vec4<f32>,
    ascii: f32,
    time: f32,
    cell_w: f32,
    cell_h: f32,
    lod: f32,
    count: f32,
    invert: f32,
    pad1: f32,
};
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> fit: Fit;
@group(0) @binding(4) var atlas: texture_2d<f32>;
@group(0) @binding(5) var atlas_samp: sampler;

@vertex fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var c = array<vec2<f32>, 6>(vec2(0., 0.), vec2(1., 0.), vec2(0., 1.), vec2(0., 1.), vec2(1., 0.), vec2(1., 1.));
    let p = c[vi];
    var o: VsOut;
    o.pos = vec4((p * 2. - 1.) * fit.scale, 0., 1.);
    o.uv = vec2(p.x, 1. - p.y);
    return o;
}

fn luma_at(at: vec2<f32>) -> f32 {
    return (textureSampleLevel(y_tex, samp, at, 0.).r - 16. / 255.) * (255. / 219.);
}

fn rgb_at(at: vec2<f32>) -> vec3<f32> {
    let y = luma_at(at);
    let uv = (textureSampleLevel(uv_tex, samp, at, 0.).rg - 128. / 255.) * (255. / 224.);
    let r = y + 1.5748 * uv.y;
    let g = y - .1873 * uv.x - .4681 * uv.y;
    let b = y + 1.8556 * uv.x;
    return clamp(vec3(r, g, b), vec3(0.), vec3(1.));
}

fn hash(q: vec2<f32>) -> f32 {
    var v = fract(vec3<f32>(q.x, q.y, q.x) * 0.1031);
    v += dot(v, v.yzx + 33.33);
    return fract((v.x + v.y) * v.z);
}

@fragment fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    let pixel_uv = vec2(dpdx(i.uv.x), dpdy(i.uv.y));
    let picture = rgb_at(i.uv);
    if (fit.ascii <= 0.0) {
        return vec4(clamp(picture * fit.flash, vec3(0.), vec3(1.)) * fit.shown, 1.);
    }
    let cell_size = vec2(fit.cell_w, fit.cell_h);
    let local = i.pos.xy - fit.origin;
    let coord = local / cell_size;
    let cell = floor(coord);
    let inner = coord - cell;
    let rows = max(ceil(fit.size.y / fit.cell_h), 1.);
    let order = (hash(cell) * .74 + (cell.y / rows) * .26) * 1.35 + .05;
    let age = fit.ascii * 1.5 - order;
    // Everything not yet reduced has already lost its colour: the signal is gone either way.
    let scan = .78 + .22 * step(1.5, (i.pos.y % 3.));
    let tinted = fit.ink.rgb * dot(picture, vec3(.2126, .7152, .0722)) * .45 * scan;
    let base = mix(picture, tinted, smoothstep(0., .06, fit.ascii));
    if (age < 0.) {
        return vec4(base * fit.shown, 1.);
    }
    let centre = i.uv + (cell + .5 - coord) * cell_size * pixel_uv;
    var raw = clamp(luma_at(centre), 0., 1.);
    if (fit.invert > .5) {
        raw = 1. - raw;
    }
    let l = pow(raw, 1.35);
    let tick = floor(fit.time * 12.);
    var index = min(floor(l * fit.count), fit.count - 1.);
    var ink = fit.ink.rgb * (.24 + .76 * l);
    if (age < .22) {
        index = 1. + floor(hash(cell + tick * 1.37) * (fit.count - 1.));
        ink = fit.hot.rgb;
    } else if (hash(cell * 3.1 + tick) < .007) {
        index = 1. + floor(hash(cell + tick * 1.37) * (fit.count - 1.));
    }
    let glyph_uv = vec2((index + inner.x) / fit.count, inner.y);
    let coverage = textureSampleLevel(atlas, atlas_samp, glyph_uv, fit.lod).r;
    return vec4(ink * coverage * fit.shown, 1.);
}
