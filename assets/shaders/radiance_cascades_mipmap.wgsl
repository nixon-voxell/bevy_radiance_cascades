#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import "shaders/radiance_probe.wgsl"::Probe;

@group(0) @binding(0) var<uniform> probe: Probe;
@group(0) @binding(1) var tex_main: texture_2d<f32>;
@group(0) @binding(2) var sampler_main: sampler;
@group(0) @binding(3) var tex_radiance_field: texture_2d<f32>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(tex_radiance_field));
    let probe_cell = vec2<u32>(in.uv * dimensions) / probe.width * probe.width;
    let ray_count = probe.width + probe.width;

    var accumulation = vec4<f32>(0.0);
    for (var y: u32 = 0; y < probe.width; y++) {
        for (var x: u32 = 0; x < probe.width; x++) {
            accumulation += textureLoad(tex_radiance_field, probe_cell + vec2<u32>(x, y), 0);
        }
    }
    accumulation /= f32(ray_count);

    let main = textureSample(tex_main, sampler_main, in.uv);

    // Bilinear filtering should be used..?
    // return main + accumulation;
    return textureLoad(tex_radiance_field, vec2<u32>(in.uv * dimensions), 0);
}
