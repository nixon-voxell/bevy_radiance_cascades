#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen_texture: texture_2d<u32>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dimension = textureDimensions(screen_texture, 0);
    // Calculate texture coordinates
    let tex_coords = vec2<u32>(
        in.uv * vec2<f32>(dimension)
    );

    let main_tex = vec4<f32>(textureLoad(screen_texture, tex_coords, 0));

    let dimensionf = vec2<f32>(dimension);
    if main_tex.r <= dimensionf.r {
        return vec4<f32>(main_tex.rg/vec2<f32>(dimension), 0.0, 0.0);
    } else {
        return vec4<f32>(0.0);
    }
}
