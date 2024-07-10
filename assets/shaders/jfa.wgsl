#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var jfa_texture: texture_2d<u32>;
@group(0) @binding(1) var texture_sampler: sampler;

const F32_MAX = 3.40282e+38;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec2<i32> {
    var uv_offsets = array<vec2<f32>, 9>(
        vec2<f32>(-1, 1),
        vec2<f32>(0, 1),
        vec2<f32>(1, 1),
        vec2<f32>(-1, 0),
        vec2<f32>(0, 0),
        vec2<f32>(1, 0),
        vec2<f32>(-1, -1),
        vec2<f32>(0, -1),
        vec2<f32>(1, -1),
    );

    var best_distance = F32_MAX;

    for (var i = 0; i < 9; i++) {
        let offset = uv_offsets[i];
        let jfa_tex = textureSample(screen_texture, texture_sampler, in.uv + offset);

        if jfa_tex.x > 0.0 {
            // Valid
        } else {
            // Invalid
        }
    }
}
