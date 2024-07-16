#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

const SIZE: u32 = 2;

@group(0) @binding(0) var screen_texture: texture_2d<f32>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dimension = textureDimensions(screen_texture, 0);
    // Calculate texture coordinates
    let tex_coordsf = in.uv * vec2<f32>(dimension);
    let tex_coords = vec2<u32>(tex_coordsf);

    let grid_cell = vec2<f32>(tex_coords % SIZE) / f32(SIZE);

    let main_tex = vec4<f32>(textureLoad(screen_texture, tex_coords, 0));

    let dimensionf = vec2<f32>(dimension);

    return vec4<f32>(main_tex);// + vec4<f32>(grid_cell * 0.1, 0, 0);
    // return vec4<f32>(length(main_tex.rg - tex_coordsf)/vec2<f32>(dimension).x);
}
