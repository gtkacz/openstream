// The chrome's soft glow. A small copy of the interface, blurred, then screened over the window.

struct Blur {
    step: vec2<f32>,
    strength: f32,
    pad: f32,
};

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> blur: Blur;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> Vertex {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return Vertex(
        vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0),
        vec2<f32>(corner.x, 1.0 - corner.y),
    );
}

// Nine binomial taps reduced to five by letting the sampler blend each pair.
@fragment
fn fs_blur(vertex: Vertex) -> @location(0) vec4<f32> {
    var sum = textureSampleLevel(source, linear, vertex.uv, 0.0) * 0.2270270270;
    sum += textureSampleLevel(source, linear, vertex.uv + blur.step * 1.3846153846, 0.0) * 0.3162162162;
    sum += textureSampleLevel(source, linear, vertex.uv - blur.step * 1.3846153846, 0.0) * 0.3162162162;
    sum += textureSampleLevel(source, linear, vertex.uv + blur.step * 3.2307692308, 0.0) * 0.0702702703;
    sum += textureSampleLevel(source, linear, vertex.uv - blur.step * 3.2307692308, 0.0) * 0.0702702703;
    return sum;
}

@fragment
fn fs_screen(vertex: Vertex) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, linear, vertex.uv, 0.0) * blur.strength;
}
