struct Fit { scale: vec2<f32>, _pad: vec2<f32> };
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
@group(0) @binding(3) var<uniform> fit: Fit;
@vertex fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut { var c = array<vec2<f32>, 6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.)); let p=c[vi]; var o:VsOut; o.pos=vec4((p*2.-1.)*fit.scale,0.,1.); o.uv=vec2(p.x,1.-p.y); return o; }
@fragment fn fs_main(i:VsOut) -> @location(0) vec4<f32> { let y=(textureSample(y_tex,samp,i.uv).r-16./255.)*(255./219.); let uv=(textureSample(uv_tex,samp,i.uv).rg-128./255.)*(255./224.); let r=y+1.5748*uv.y; let g=y-.1873*uv.x-.4681*uv.y; let b=y+1.8556*uv.x; return vec4(clamp(vec3(r,g,b),vec3(0.),vec3(1.)),1.); }
